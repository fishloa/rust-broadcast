"""Drift guard for the generated workspace crate index.

`tools/gen-crate-index.py` writes the crate tables in docs/CRATES.md and
README.md from the manifests, ci.yml's no_std list and docs/crates-index.toml.
These tests fail when the committed files are stale, when a workspace member is
not classified, and when the generator's own consistency checks stop biting.
"""

import importlib.util
import pathlib
import shutil
import sys

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("gen_crate_index", ROOT / "tools" / "gen-crate-index.py")
gen = importlib.util.module_from_spec(SPEC)
sys.modules["gen_crate_index"] = gen  # @dataclass resolves the module by name
SPEC.loader.exec_module(gen)


def test_committed_index_is_current() -> None:
    """The committed blocks equal what the generator produces now."""
    stale = [str(p.relative_to(ROOT)) for p, text in gen.expected_files(ROOT).items() if p.read_text() != text]
    assert not stale, f"stale generated crate index in {stale}; run `python3 tools/gen-crate-index.py`"


def test_check_mode_exits_zero() -> None:
    assert gen.main(["--check"]) == 0


def test_every_member_is_indexed() -> None:
    """Guard the guard: the index covers the real member list, not a subset."""
    _, crates = gen.load(ROOT)
    members = gen.workspace_members(ROOT)
    assert len(members) > 50, f"expected the full workspace, found {len(members)} members"
    assert len(crates) == len(members)


def test_no_std_list_is_parsed_from_ci() -> None:
    no_std = gen.ci_no_std_crates(ROOT)
    assert "broadcast-common" in no_std and "dvb-si" in no_std
    assert "multimux" not in no_std  # genuinely std-only, per ci.yml


def _copy_inputs(tmp: pathlib.Path) -> pathlib.Path:
    """Copy just the files the generator reads into a scratch tree."""
    shutil.copy(ROOT / "Cargo.toml", tmp / "Cargo.toml")
    (tmp / "docs").mkdir()
    for name in ("crates-index.toml", "CRATES.md"):
        shutil.copy(ROOT / "docs" / name, tmp / "docs" / name)
    shutil.copy(ROOT / "README.md", tmp / "README.md")
    (tmp / ".github" / "workflows").mkdir(parents=True)
    shutil.copy(ROOT / ".github" / "workflows" / "ci.yml", tmp / ".github" / "workflows" / "ci.yml")
    for member in gen.workspace_members(ROOT):
        (tmp / member / "src").mkdir(parents=True)
        shutil.copy(ROOT / member / "Cargo.toml", tmp / member / "Cargo.toml")
    return tmp


def test_unclassified_member_fails(tmp_path: pathlib.Path) -> None:
    """A new workspace member with no crates-index.toml entry must fail --check."""
    root = _copy_inputs(tmp_path)
    toml = root / "docs" / "crates-index.toml"
    text = toml.read_text()
    start = text.index("[crate.ule]")
    end = text.index("[crate.", start + 1)
    toml.write_text(text[:start] + text[end:])
    with pytest.raises(gen.IndexError_, match="ule"):
        gen.load(root)


def test_version_bump_makes_index_stale(tmp_path: pathlib.Path) -> None:
    """Changing a manifest fact must change the generated output."""
    root = _copy_inputs(tmp_path)
    manifest = root / "ule" / "Cargo.toml"
    text = manifest.read_text()
    old = gen._load_toml(manifest)["package"]["version"]
    manifest.write_text(text.replace(f'version = "{old}"', 'version = "99.0.0"', 1))
    files = gen.expected_files(root)
    assert files[root / "docs" / "CRATES.md"] != (root / "docs" / "CRATES.md").read_text()
    assert "99.0.0" in files[root / "docs" / "CRATES.md"]


def test_publish_false_cannot_claim_crates_io(tmp_path: pathlib.Path) -> None:
    root = _copy_inputs(tmp_path)
    toml = root / "docs" / "crates-index.toml"
    text = toml.read_text()
    start = text.index("[crate.test-alloc]")
    block_end = text.index("[crate.", start + 1)
    block = text[start:block_end].replace("crates_io = false", "crates_io = true")
    toml.write_text(text[:start] + block + text[block_end:])
    with pytest.raises(gen.IndexError_, match="publish = false"):
        gen.load(root)
