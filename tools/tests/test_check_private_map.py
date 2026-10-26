"""Guard tests for tools/check-private-map.py (private spec move)."""

import importlib.util
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("check_private_map", ROOT / "tools" / "check-private-map.py")
cpm = importlib.util.module_from_spec(SPEC)
sys.modules["check_private_map"] = cpm
SPEC.loader.exec_module(cpm)


def test_committed_tree_passes() -> None:
    assert cpm.main() == 0


def _scratch(tmp_path: pathlib.Path, files: dict[str, str]) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    for rel, text in files.items():
        p = tmp_path / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, check=True)


MAP = (
    "| Old public path | Private path | Note |\n|---|---|---|\n"
    "| `foo/docs/a.md` | `private/specs-verified/x/foo/a.md` | |\n"
)


def _run(monkeypatch, tmp_path: pathlib.Path) -> int:
    monkeypatch.setattr(cpm, "ROOT", tmp_path)
    monkeypatch.setattr(cpm, "MAP", tmp_path / "docs" / "specs" / "PRIVATE-MAP.md")
    monkeypatch.setattr(cpm, "BASELINE", tmp_path / "tools" / "private-map-baseline.txt")
    return cpm.main()


def test_link_to_moved_path_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"docs/specs/PRIVATE-MAP.md": MAP, "foo/README.md": "see [a](docs/a.md)\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_old_path_reappearing_fails(tmp_path, monkeypatch) -> None:
    _scratch(tmp_path, {"docs/specs/PRIVATE-MAP.md": MAP, "foo/docs/a.md": "x\n"})
    assert _run(monkeypatch, tmp_path) == 1


def test_unmapped_citation_fails_and_mapped_passes(tmp_path, monkeypatch) -> None:
    _scratch(
        tmp_path,
        {"docs/specs/PRIVATE-MAP.md": MAP, "foo/src/lib.rs": "//! see docs/a.md and docs/nope.md\n"},
    )
    assert _run(monkeypatch, tmp_path) == 1
    _scratch(tmp_path, {"foo/src/lib.rs": "//! see docs/a.md\n"})
    assert _run(monkeypatch, tmp_path) == 0
