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


def test_ui_docs_describe_search_and_application_owned_ai_flow():
    claude = (ROOT / "CLAUDE.md").read_text()
    gallery = (ROOT / "docs/features/gallery-ui.md").read_text()
    ai = (ROOT / "docs/features/ai-metadata.md").read_text()
    config = (ROOT / "docs/features/config.md").read_text()

    assert "nix run .#ui" in claude
    assert "runtime subset" in gallery
    assert "Empty/0, 25, 50, 75, or 100" in gallery
    assert "defaults to 25" in gallery and "all configured `publicable` paths selected" in gallery
    assert "publicable" in config and "runtime" in config
    assert "both title and description" in ai
    assert "publication remains SCHEDULED and retryable" in ai
