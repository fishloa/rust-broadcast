"""Tests for tools/spec_extract.py."""

import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "spec_extract.py"
def _private_specs() -> Path:
    env = os.environ.get("RUST_BROADCAST_PRIVATE")
    if env and (Path(env) / "specs-verified").is_dir():
        return Path(env) / "specs-verified"
    return ROOT / "private" / "specs-verified"


PRIVATE_SPECS = _private_specs()
SOURCE = PRIVATE_SPECS / "ietf" / "specs" / "ietf_rfc8216_hls.txt"
SECTION = "3.5"


def _need(path: Path) -> None:
    if not path.is_file():
        pytest.skip(f"private spec text not checked out: {path}")


def _run(tmp_path: Path, *extra: str) -> subprocess.CompletedProcess:
    _need(SOURCE)
    out = tmp_path / "out.md"
    cmd = [sys.executable, str(SCRIPT), "--source", str(SOURCE), "--sections", SECTION, "--output", str(out), *extra]
    # run from ROOT so relative source resolves
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)


def _furniture(line: str) -> bool:
    s = line.strip()
    if "\f" in line and s.replace("\f", "").strip() == "":
        return True
    if not s:
        return False
    if "[Page " in s:
        return True
    if s.startswith("RFC ") and s.split() and s.split()[1].isdigit():
        return True
    return False


def _section_lines():
    _need(SOURCE)
    lines = SOURCE.read_text(encoding="utf-8").split("\n")
    heading_re = re.compile(r"^(\d+(?:\.\d+)*)\.\s{2,}\S")
    heads = [(i, m.group(1)) for i, line in enumerate(lines) for m in [heading_re.match(line)] if m]
    # find start
    start = None
    depth = None
    for i, num in heads:
        if num == SECTION:
            start = i
            depth = num.count(".") + 1
            break
    assert start is not None
    end = len(lines)
    for i, num in heads:
        if i <= start:
            continue
        if num.count(".") + 1 <= depth:
            end = i
            break
    raw = lines[start:end]
    return [ln.replace("\f", "") for ln in raw if not _furniture(ln)]


def test_known_short_section_equals_source_minus_furniture(tmp_path: Path) -> None:
    res = _run(tmp_path)
    assert res.returncode == 0, res.stderr
    out = (tmp_path / "out.md").read_text(encoding="utf-8")
    m = re.search(r"```text\n(.*?)\n```", out, re.S)
    assert m, "no fenced block"
    body = m.group(1)
    expected = "\n".join(_section_lines())
    # trim trailing empty lines in expected
    while expected.endswith("\n"):
        expected = expected[:-1]
    assert body == expected


def test_page_furniture_never_appears(tmp_path: Path) -> None:
    res = _run(tmp_path)
    assert res.returncode == 0, res.stderr
    out = (tmp_path / "out.md").read_text(encoding="utf-8")
    assert "[Page " not in out
    assert "\f" not in out
    assert not any(re.match(r"^RFC \d+ .* \d{4}$", ln.strip()) for ln in out.splitlines())


def test_unknown_section_exits_nonzero(tmp_path: Path) -> None:
    _need(SOURCE)
    out = tmp_path / "out.md"
    cmd = [sys.executable, str(SCRIPT), "--source", str(SOURCE), "--sections", "99.99", "--output", str(out)]
    res = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    assert res.returncode != 0


EXTRACTS = [
    ("rtp/hevc-rtp/rfc7798-extract.md", "specs/rfc7798_rtp_hevc.txt"),
    ("hls/ietf/rfc8216-webvtt-extract.md", "specs/ietf_rfc8216_hls.txt"),
]


def _norm_ws(lines):
    return "\n".join(line.rstrip() for line in lines).strip("\n")


def _parse_extract(path: Path):
    lines = path.read_text(encoding="utf-8").split("\n")
    sections = []
    i = 0
    while i < len(lines):
        m = re.match(r"##\s+(.+?)\s+\(([^)]+):(\d+)-(\d+)\)\s*$", lines[i])
        if m:
            title_part = m.group(1)
            source_rel = m.group(2)
            start = int(m.group(3))
            end = int(m.group(4))
            j = i + 1
            while j < len(lines) and not lines[j].startswith("```text"):
                j += 1
            assert j < len(lines), f"no fenced block for {title_part}"
            j += 1
            body_lines = []
            while j < len(lines) and not lines[j].startswith("```"):
                body_lines.append(lines[j])
                j += 1
            sections.append((title_part, source_rel, start, end, body_lines))
        i += 1
    return sections


def test_every_committed_extract_section_matches_source() -> None:
    for extract_rel, _ in EXTRACTS:
        extract_path = PRIVATE_SPECS / extract_rel
        _need(extract_path)
        sections = _parse_extract(extract_path)
        assert sections, f"no sections in {extract_rel}"
        for title_part, source_rel, start, end, body_lines in sections:
            src_path = PRIVATE_SPECS / "ietf" / source_rel
            _need(src_path)
            src_lines = src_path.read_text(encoding="utf-8").split("\n")
            assert 1 <= start <= end <= len(src_lines), (source_rel, start, end, len(src_lines))
            cited = src_lines[start - 1:end]
            filtered = [ln.replace("\f", "") for ln in cited if not _furniture(ln)]
            while filtered and filtered[-1].strip() == "":
                filtered.pop()

            first_nb = next((ln for ln in cited if ln.strip() != ""), "")
            m = re.match(r"^(\S+)\.\s+(.*)$", title_part)
            assert m is not None, title_part
            num = m.group(1)
            title = m.group(2)
            assert " ".join(first_nb.split()) == f"{num}. {title}", (first_nb, title_part)

            assert _norm_ws(filtered) == _norm_ws(body_lines), (extract_rel, title_part)
