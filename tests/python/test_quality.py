"""Exercise the quality gates with real analyzers and disposable Rust fixtures."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
METRICS = ROOT / "scripts/check-metrics.sh"
# Enough tokens to be scanned, but no branches or long repeated token sequences.
CLEAN = "pub fn values() -> [u32; 100] { [\n" + ",\n".join(map(str, range(100))) + "\n] }\n"


class QualityMetricsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "clean.rs").write_text(CLEAN)

    def scan(self, *paths, env=None):
        return subprocess.run(
            ["bash", str(METRICS), *map(str, paths or [self.root])],
            cwd=self.root,
            env=env,
            text=True,
            capture_output=True,
            timeout=30,
        )

    def test_clean_code_passes(self):
        result = self.scan()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_limits_are_per_function_not_file_totals(self):
        # Five nested ifs score 15 (1+2+3+4+5); nine sibling ifs score 10 cyclomatic.
        functions = "\n".join(
            f"fn nested_{n}(b: bool) {{ " + "if b { " * 5 + "}" * 6
            for n in range(2)
        )
        functions += "\nfn branches(b: bool) { " + "if b {} " * 9 + "}\n"
        (self.root / "limits.rs").write_text(functions)
        result = self.scan()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_excessive_cognitive_complexity_fails(self):
        (self.root / "complex.rs").write_text(
            "fn nested(b: bool) { " + "if b { " * 6 + "}" * 7
        )
        result = self.scan()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cognitive=21", result.stdout)
        self.assertIn("complex.rs:1", result.stdout)

    def test_excessive_cyclomatic_complexity_fails(self):
        (self.root / "branches.rs").write_text(
            "fn branches(b: bool) { " + "if b {} " * 10 + "}\n"
        )
        result = self.scan()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cyclomatic=11", result.stdout)

    def test_nested_closure_is_checked_without_aggregating_parent(self):
        (self.root / "closure.rs").write_text(
            "fn parent(b: bool) { let _f = || { " + "if b { " * 6 + "}" * 7 + "; }"
        )
        result = self.scan()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("closure.rs:1", result.stdout)
        self.assertNotIn("parent: cognitive=", result.stdout)

    def test_duplication_across_source_test_and_example_paths_fails(self):
        roots = [self.root / name for name in ("src", "tests", "examples")]
        duplicate = "pub fn repeated(mut x: u32) -> u32 {\n"
        duplicate += "".join(f"    x = x.wrapping_mul({n}).wrapping_add({n + 1});\n" for n in range(20))
        duplicate += "    x\n}\n"
        for folder in roots:
            folder.mkdir()
            (folder / "copy.rs").write_text(duplicate)
        result = self.scan(*roots)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("copy.rs", result.stdout)

    def test_duplication_requires_both_token_and_line_thresholds(self):
        # 5 header + 18*5 let + 4 return + 1 closing tokens = 100, on 10 lines.
        statements = [f"let x{n} = {n};" for n in range(18)]
        boundary = "fn boundary() {\n"
        boundary += "\n".join(" ".join(statements[n:n + 3]) for n in range(0, 18, 3))
        boundary += "\n\nreturn ();\n}\n"
        for source, fails in ((boundary, True),
                              (boundary.replace("return ();", "return;"), False),
                              (boundary.replace("\n\n", "\n"), False)):
            with self.subTest(source=source):
                for name in ("a.rs", "b.rs"):
                    (self.root / name).write_text(source)
                result = self.scan()
                self.assertEqual(result.returncode != 0, fails, result.stdout + result.stderr)

    def test_missing_or_empty_scan_fails(self):
        empty = self.root / "empty"
        empty.mkdir()
        for path in (empty, self.root / "missing"):
            with self.subTest(path=path):
                self.assertNotEqual(self.scan(path).returncode, 0)

    def test_failed_empty_or_malformed_analyzer_output_fails(self):
        # Only fake the external process to exercise failures it cannot reliably produce on demand.
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        analyzer = bin_dir / "rust-code-analysis-cli"
        env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}")
        for body in ("exit 7", "exit 0", "echo '{}'", "echo '{bad json'"):
            with self.subTest(body=body):
                analyzer.write_text("#!/bin/sh\n" + body + "\n")
                analyzer.chmod(0o755)
                self.assertNotEqual(self.scan(env=env).returncode, 0)


class PrePushTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        local_vars = subprocess.check_output(["git", "rev-parse", "--local-env-vars"], text=True).splitlines()
        self.env = {key: value for key, value in os.environ.items() if key not in local_vars}
        self.git("init", "-q", "-b", "main")
        (self.repo / "file").write_text("committed\n")
        self.git("add", "file")
        self.git("-c", "core.hooksPath=/dev/null", "-c", "user.name=Quality Test",
                 "-c", "user.email=quality@example.invalid", "commit", "-qm", "fixture")
        self.head = self.git("rev-parse", "HEAD").strip()
        self.calls = root / "calls"
        bin_dir = root / "bin"
        bin_dir.mkdir()
        nix = bin_dir / "nix"
        nix.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\nexit "${RESULT:-0}"\n')
        nix.chmod(0o755)
        self.env.update(PATH=f"{bin_dir}:{self.env['PATH']}", CALLS=str(self.calls))

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, env=self.env, text=True)

    def hook(self, oid=None):
        return subprocess.run(
            ["bash", str(ROOT / ".githooks/pre-push")], cwd=self.repo, env=self.env,
            input=f"refs/heads/main {oid or self.head} refs/heads/main {'0' * 40}\n",
            text=True, capture_output=True, timeout=10,
        )

    def test_checks_clean_head_and_propagates_failure(self):
        result = self.hook()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text(), "run .#quality\n")
        self.env["RESULT"] = "7"
        self.assertEqual(self.hook().returncode, 7)

    def test_dirty_tracked_or_untracked_code_is_not_substituted_for_commit(self):
        for path in ("file", "untracked.rs"):
            with self.subTest(path=path):
                (self.repo / path).write_text("uncommitted\n")
                result = self.hook()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("clean", result.stderr)
                self.assertFalse(self.calls.exists())
                self.git("checkout", "--", "file")

    def test_pushing_a_different_commit_is_rejected(self):
        result = self.hook("1" * 40)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked-out", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_deletion_only_push_needs_no_checks(self):
        self.assertEqual(self.hook("0" * 40).returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_fixture_git_does_not_modify_an_inherited_repository(self):
        # Only the disposable outer repo can be touched even before the fix.
        with patch.dict(os.environ, GIT_DIR=str(self.repo / ".git"),
                        GIT_WORK_TREE=str(self.repo), GIT_INDEX_FILE=str(self.repo / ".git/index")):
            nested = PrePushTests("test_deletion_only_push_needs_no_checks")
            result = unittest.TestResult()
            nested.run(result)
        self.assertEqual(result.errors + result.failures, [])
        self.assertEqual(self.git("rev-parse", "HEAD").strip(), self.head)
        self.assertEqual(self.git("status", "--porcelain"), "")

    def test_quality_runner_clears_git_context_before_running_tools(self):
        real_bash = shutil.which("bash")
        bin_dir = Path(self.env["PATH"].split(os.pathsep)[0])
        for name in ("python", "cargo", "bash"):
            tool = bin_dir / name
            tool.write_text('#!/bin/sh\ntest -z "${GIT_DIR:-}${GIT_WORK_TREE:-}${GIT_INDEX_FILE:-}"\n')
            tool.chmod(0o755)
        self.env.update(GIT_DIR=str(self.repo / ".git"), GIT_WORK_TREE=str(self.repo),
                        GIT_INDEX_FILE=str(self.repo / ".git/index"))
        result = subprocess.run([real_bash, str(ROOT / "scripts/quality.sh")],
                                cwd=self.repo, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
