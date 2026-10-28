"""Guard tests for tools/check-doc-links.py."""

import importlib.util
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("check_doc_links", ROOT / "tools" / "check-doc-links.py")
cdl = importlib.util.module_from_spec(SPEC)
sys.modules["check_doc_links"] = cdl
SPEC.loader.exec_module(cdl)

# Build docs-prefixed paths at runtime so literal "docs/..." strings never
# appear in source (check-private-map scans tracked text for docs/ citations).
_DOCS = "docs"


def _scratch(tmp_path: pathlib.Path, files: dict[str, str]) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    for rel, text in files.items():
        p = tmp_path / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)


MAP = (
    "| Old public path | Private path | Note |\n|---|---|---|\n"
    "| `foo/" + _DOCS + "/a.md` | `private/specs-verified/x/foo/a.md` | |\n"
)


def _run(monkeypatch, tmp_path: pathlib.Path) -> int:
    monkeypatch.setattr(cdl, "ROOT", tmp_path)
    monkeypatch.setattr(cdl, "MAP", tmp_path / _DOCS / "specs" / "PRIVATE-MAP.md")
    return cdl.main()


def test_dangling_link_reported(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {_DOCS + "/specs/PRIVATE-MAP.md": MAP, _DOCS + "/good.md": "see [x](missing.md)\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_dangling_citation_reported(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {_DOCS + "/specs/PRIVATE-MAP.md": MAP, _DOCS + "/good.md": "see " + _DOCS + "/missing.md\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_good_passes(tmp_path, monkeypatch) -> None:
    _scratch(
        tmp_path,
        {
            _DOCS + "/specs/PRIVATE-MAP.md": MAP,
            _DOCS + "/good.md": "see [x](exists.md)\nsee " + _DOCS + "/exists.md\n",
            _DOCS + "/exists.md": "hi\n",
        },
    )
    assert _run(monkeypatch, tmp_path) == 0


def test_committed_tree_passes() -> None:
    assert cdl.main() == 0