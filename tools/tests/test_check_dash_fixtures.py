"""Drift guard for `fixtures/dash/`: checksums, XML well-formedness, referenced files.

`tools/check-dash-fixtures.py` re-verifies the committed DASH fixtures. The first test runs it on the real tree;
the others build a tiny tree in a temp dir and break it one way at a time, so each check is shown to fail when it
should (a checker that passes everything would also pass the first test).
"""

import hashlib
import importlib.util
import pathlib
import sys

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("check_dash_fixtures", ROOT / "tools" / "check-dash-fixtures.py")
chk = importlib.util.module_from_spec(SPEC)
sys.modules["check_dash_fixtures"] = chk
SPEC.loader.exec_module(chk)

NS = 'xmlns="urn:mpeg:dash:schema:mpd:2011"'

TEMPLATE_MPD = f"""<?xml version="1.0"?>
<MPD {NS} type="static">
 <Period>
  <AdaptationSet>
   <SegmentTemplate media="seg-$RepresentationID$-$Number%03d$.m4s" initialization="init-$RepresentationID$.m4s" startNumber="1"/>
   <Representation id="7" bandwidth="1000"/>
  </AdaptationSet>
 </Period>
</MPD>
"""


def write_sums(base: pathlib.Path) -> None:
    lines = []
    for p in sorted(base.rglob("*")):
        if p.is_file() and p.name != "SHA256SUMS":
            lines.append(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(base).as_posix()}")
    (base / "SHA256SUMS").write_text("\n".join(lines) + "\n")


@pytest.fixture
def tree(tmp_path: pathlib.Path) -> pathlib.Path:
    d = tmp_path / "set"
    d.mkdir()
    (d / "m.mpd").write_text(TEMPLATE_MPD)
    (d / "init-7.m4s").write_bytes(b"init")
    (d / "seg-7-001.m4s").write_bytes(b"seg")
    write_sums(tmp_path)
    return tmp_path


def test_committed_fixtures_verify() -> None:
    assert chk.verify(ROOT / "fixtures" / "dash") == []


def test_cli_check_exits_zero() -> None:
    assert chk.main([]) == 0


def test_clean_tree_passes(tree: pathlib.Path) -> None:
    assert chk.verify(tree) == []


def test_modified_file_fails_checksum(tree: pathlib.Path) -> None:
    (tree / "set" / "seg-7-001.m4s").write_bytes(b"tampered")
    errs = chk.verify(tree)
    assert any("checksum" in e and "seg-7-001.m4s" in e for e in errs), errs


def test_unlisted_file_fails(tree: pathlib.Path) -> None:
    (tree / "set" / "extra.bin").write_bytes(b"x")
    errs = chk.verify(tree)
    assert any("not listed" in e and "extra.bin" in e for e in errs), errs


def test_listed_but_missing_file_fails(tree: pathlib.Path) -> None:
    (tree / "set" / "seg-7-001.m4s").unlink()
    errs = chk.verify(tree)
    assert any("missing" in e and "seg-7-001.m4s" in e for e in errs), errs


def test_malformed_xml_fails(tree: pathlib.Path) -> None:
    (tree / "set" / "m.mpd").write_text("<MPD><Period></MPD>")
    write_sums(tree)
    errs = chk.verify(tree)
    assert any("not well-formed" in e and "m.mpd" in e for e in errs), errs


def test_unreferenced_media_template_match_fails(tree: pathlib.Path) -> None:
    (tree / "set" / "seg-7-001.m4s").unlink()
    (tree / "set" / "seg-9-001.m4s").write_bytes(b"other representation")
    write_sums(tree)
    errs = chk.verify(tree)
    assert any("no file matches" in e and "seg-" in e and "Representation 7" in e for e in errs), errs


def test_missing_initialization_fails(tree: pathlib.Path) -> None:
    (tree / "set" / "init-7.m4s").unlink()
    write_sums(tree)
    errs = chk.verify(tree)
    assert any("no file matches" in e and "init-" in e and "Representation 7" in e for e in errs), errs


def test_segment_list_and_base_url_references(tree: pathlib.Path) -> None:
    sub = tree / "list"
    sub.mkdir()
    (sub / "l.mpd").write_text(
        f"""<MPD {NS}><BaseURL>../set/</BaseURL><Period><AdaptationSet>
<Representation id="1"><SegmentList><Initialization sourceURL="init-7.m4s"/><SegmentURL media="seg-7-001.m4s"/>
<SegmentURL media="seg-7-002.m4s"/></SegmentList></Representation></AdaptationSet></Period></MPD>"""
    )
    (sub / "pad.m4s").write_bytes(b"media-bearing directory")
    write_sums(tree)
    errs = chk.verify(tree)
    assert any("seg-7-002.m4s" in e and "missing" in e for e in errs), errs
    assert not any("init-7.m4s" in e or "seg-7-001.m4s" in e for e in errs), errs


def test_single_file_base_url_must_exist(tree: pathlib.Path) -> None:
    sub = tree / "od"
    sub.mkdir()
    (sub / "o.mpd").write_text(
        f"""<MPD {NS}><Period><AdaptationSet><Representation id="1"><BaseURL>v.mp4</BaseURL>
<SegmentBase indexRange="0-9"/></Representation></AdaptationSet></Period></MPD>"""
    )
    (sub / "pad.m4s").write_bytes(b"media-bearing directory")
    write_sums(tree)
    errs = chk.verify(tree)
    assert any("v.mp4" in e and "missing" in e for e in errs), errs


def test_directory_without_media_is_not_reference_checked(tree: pathlib.Path) -> None:
    sub = tree / "docs-only"
    sub.mkdir()
    (sub / "x.mpd").write_text(TEMPLATE_MPD)
    write_sums(tree)
    assert chk.verify(tree) == []


def test_update_rewrites_sums(tree: pathlib.Path) -> None:
    (tree / "set" / "seg-7-001.m4s").write_bytes(b"changed on purpose")
    assert chk.verify(tree)
    chk.update(tree)
    assert chk.verify(tree) == []
