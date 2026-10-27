"""Tests for tools/spec_extract.py."""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "spec_extract.py"
SOURCE = ROOT / "specs" / "ietf_rfc8216_hls.txt"
SECTION = "3.5"


def _run(tmp_path: Path, *extra: str) -> subprocess.CompletedProcess:
    out = tmp_path / "out.md"
    cmd = [sys.executable, str(SCRIPT), "--source", str(SOURCE.relative_to(ROOT)), "--sections", SECTION, "--output", str(out), *extra]
    # run from ROOT so relative source resolves
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)


def _furniture(line: str) -> bool:
    s = line.strip()
    if not s:
        return False
    if "\f" in line and s.replace("\f", "").strip() == "":
        return True
    if "[Page " in s:
        return True
    if s.startswith("RFC ") and s.split() and s.split()[1].isdigit():
        return True
    return False


def _section_lines():
    lines = SOURCE.read_text(encoding="utf-8").splitlines()
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
    out = tmp_path / "out.md"
    cmd = [sys.executable, str(SCRIPT), "--source", str(SOURCE.relative_to(ROOT)), "--sections", "99.99", "--output", str(out)]
    res = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    assert res.returncode != 0
