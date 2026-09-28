"""Drift guard: every repo path a doc names must exist, so docs/ cannot rot silently."""
import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
DOCS = sorted((ROOT / "docs").rglob("*.md")) + [ROOT / "CLAUDE.md"]
# Backticked repo-relative paths with an extension or a trailing slash. Bare ADR
# ids like `docs/adr/0005` and data-dir files (`publications.json`) are not repo paths.
PATH_RE = re.compile(r"`((?:src|tests|docs|\.claude)/[^`\s:]*(?:\.\w+|/))")


@pytest.mark.parametrize("doc", DOCS, ids=lambda p: str(p.relative_to(ROOT)))
def test_referenced_repo_paths_exist(doc):
    missing = sorted({m for m in PATH_RE.findall(doc.read_text()) if not (ROOT / m).exists()})
    assert not missing, f"{doc.relative_to(ROOT)} names missing paths: {missing}"
