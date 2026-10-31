#!/usr/bin/env python3
"""Extract verbatim sections from an RFC plain-text file into Markdown.

Usage:
  python3 tools/spec_extract.py --source private/specs-verified/ietf/specs/rfc7798_rtp_hevc.txt \\
      --sections 4.1 4.2 4.3 4.4.1 4.4.2 4.4.3 4.4.4 4.5 5 6 7.1 \\
      --output private/specs-verified/rtp/hevc-rtp/rfc7798-extract.md
"""

import argparse
import re
import shlex
import sys
from pathlib import Path

HEADING_RE = re.compile(r"^(\d+(?:\.\d+)*)\.\s{2,}(\S.*)$")
PAGE_RE = re.compile(r"\[Page \d+\]")
RFC_HEADER_RE = re.compile(r"^RFC\s+\d+")


def parse_args():
    p = argparse.ArgumentParser(description="Extract RFC sections into Markdown.")
    p.add_argument("--source", "--src", "--input", "-s", "-i", dest="source", required=True)
    p.add_argument("--sections", "--section", "-S", dest="sections", nargs="+", required=True)
    p.add_argument("--output", "--out", "-o", dest="output", required=True)
    args = p.parse_args()
    secs = []
    for s in args.sections:
        for part in str(s).split(","):
            part = part.strip()
            if part:
                secs.append(part)
    args.sections = secs
    return args


def is_furniture(line):
    s = line.strip()
    if "\f" in line and s.replace("\f", "").strip() == "":
        return True
    if not s:
        return False
    if PAGE_RE.search(s):
        return True
    if RFC_HEADER_RE.match(s):
        return True
    return False


def rfc_number(source, text):
    m = re.search(r"rfc(\d+)", Path(source).name, re.IGNORECASE)
    if m:
        return m.group(1)
    m = re.search(r"Request for Comments:\s*(\d+)", text)
    if m:
        return m.group(1)
    return "unknown"


def find_headings(lines):
    out = []
    for i, line in enumerate(lines):
        m = HEADING_RE.match(line.rstrip("\n"))
        if m:
            out.append((i, m.group(1), m.group(2).strip(), len(m.group(1).split("."))))
    return out


def strip_furniture(raw_lines):
    out = []
    for line in raw_lines:
        if is_furniture(line):
            continue
        out.append(line.replace("\f", ""))
    return out


def main():
    args = parse_args()
    src_path = Path(args.source)
    if not src_path.is_file():
        print(f"spec_extract: source not found: {args.source}", file=sys.stderr)
        return 1
    text = src_path.read_text(encoding="utf-8")
    lines = text.split("\n")
    headings = find_headings(lines)
    by_number = {}
    for idx, num, title, depth in headings:
        by_number.setdefault(num, []).append((idx, title, depth))

    blocks = []
    for sec in args.sections:
        key = sec.rstrip(".")
        if key not in by_number:
            print(f"spec_extract: unknown section: {sec}", file=sys.stderr)
            return 1
        # If duplicate, use first occurrence.
        start_idx, title, depth = by_number[key][0]
        # find next heading of equal or shallower depth
        end_idx = len(lines)
        for idx, num, t, d in headings:
            if idx <= start_idx:
                continue
            if d <= depth:
                end_idx = idx
                break
        raw = lines[start_idx:end_idx]
        body = strip_furniture(raw)
        # drop trailing empty lines for neatness
        while body and body[-1].strip() == "":
            body.pop()
        if not body:
            print(f"spec_extract: empty section: {sec}", file=sys.stderr)
            return 1
        start_line = start_idx + 1
        end_line = end_idx  # last included original line number
        blocks.append(
            f"## {key}. {title} ({args.source}:{start_line}-{end_line})\n\n"
            f"```text\n" + "\n".join(body) + "\n```\n"
        )

    out_lines = []
    out_lines.append(f"Source file: {args.source}")
    out_lines.append(f"RFC number: {rfc_number(args.source, text)}")
    out_lines.append("Licence: IETF Trust, BCP 78 / RFC 5378 allow reproduction with attribution.")
    out_lines.append(f"Command: {shlex.join(sys.argv)}")
    out_lines.append("")
    out_lines.append("\n".join(blocks))
    Path(args.output).parent.mkdir(parents=True, exist_ok=True)
    Path(args.output).write_text("\n".join(out_lines), encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
