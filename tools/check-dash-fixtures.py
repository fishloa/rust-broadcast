#!/usr/bin/env python3
"""Re-verify the committed DASH fixtures under fixtures/dash/.

Checks, in order: every file is listed in SHA256SUMS with a matching digest (and nothing listed is missing);
every .mpd/.mpp/.xml is well-formed XML; and in every directory that holds media, every file an MPD references
(BaseURL chain, SegmentTemplate, SegmentList, single-file BaseURL) exists. Stdlib only.

    check-dash-fixtures.py            verify, exit 1 on any finding
    check-dash-fixtures.py --update   rewrite SHA256SUMS from the files on disk
"""

import argparse
import hashlib
import pathlib
import re
import sys
import xml.etree.ElementTree as ET
from urllib.parse import urljoin, urlsplit

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_DIR = ROOT / "fixtures" / "dash"
SUMS = "SHA256SUMS"
MPD_NS = "{urn:mpeg:dash:schema:mpd:2011}"
XML_SUFFIXES = {".mpd", ".mpp", ".xml"}
MEDIA_SUFFIXES = {".mp4", ".m4s"}
SCHEME_BASE = "file:///"
IDENTIFIER = re.compile(r"\$(RepresentationID|Number|Bandwidth|Time|SubNumber)(?:%0(\d+)d)?\$|\$\$")


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def disk_files(base: pathlib.Path) -> list[str]:
    return sorted(p.relative_to(base).as_posix() for p in base.rglob("*") if p.is_file() and p.name != SUMS)


def sums_text(base: pathlib.Path) -> str:
    return "".join(f"{sha256(base / rel)}  {rel}\n" for rel in disk_files(base))


def update(base: pathlib.Path) -> None:
    (base / SUMS).write_text(sums_text(base))


def check_sums(base: pathlib.Path) -> list[str]:
    sums = base / SUMS
    if not sums.is_file():
        return [f"{SUMS} missing in {base}"]
    listed: dict[str, str] = {}
    errs = []
    for n, line in enumerate(sums.read_text().splitlines(), 1):
        m = re.fullmatch(r"([0-9a-f]{64})  (\S.*)", line)
        if not m:
            errs.append(f"{SUMS}:{n}: malformed line")
            continue
        listed[m.group(2)] = m.group(1)
    on_disk = set(disk_files(base))
    for rel, digest in sorted(listed.items()):
        if rel not in on_disk:
            errs.append(f"{rel}: listed in {SUMS} but missing on disk")
        elif sha256(base / rel) != digest:
            errs.append(f"{rel}: checksum mismatch")
    for rel in sorted(on_disk - set(listed)):
        errs.append(f"{rel}: not listed in {SUMS}")
    return errs


def parse_xml(path: pathlib.Path) -> tuple[ET.Element | None, str | None]:
    try:
        return ET.parse(path).getroot(), None
    except ET.ParseError as e:
        return None, f"{path.name}: not well-formed XML ({e})"


def child(el: ET.Element, tag: str) -> list[ET.Element]:
    return el.findall(MPD_NS + tag)


def first_base_url(el: ET.Element) -> str | None:
    for b in child(el, "BaseURL"):
        return (b.text or "").strip()
    return None


def effective(levels: list[ET.Element], tag: str, attr: str) -> str | None:
    for lvl in levels:
        for t in child(lvl, tag):
            if attr in t.attrib:
                return t.attrib[attr]
    return None


def template_regex(template: str, rep_id: str | None, bandwidth: str | None) -> re.Pattern[str]:
    out, pos = [], 0
    for m in IDENTIFIER.finditer(template):
        out.append(re.escape(template[pos:m.start()]))
        name, width = m.group(1), m.group(2)
        if name is None:
            out.append(re.escape("$"))
        elif name == "RepresentationID" and rep_id is not None:
            out.append(re.escape(rep_id))
        elif name == "Bandwidth" and bandwidth is not None:
            out.append(re.escape(bandwidth))
        else:
            out.append(r"\d{%s,}" % (width or 1))
        pos = m.end()
    out.append(re.escape(template[pos:]))
    return re.compile("".join(out))


def resolve(prefix: str, bases: list[str], ref: str) -> str | None:
    url = SCHEME_BASE + prefix
    for b in bases + [ref]:
        url = urljoin(url, b)
    parts = urlsplit(url)
    if parts.scheme != "file":
        return None
    return parts.path.lstrip("/")


def representation_refs(prefix: str, mpd: ET.Element) -> list[tuple[str, str, str | None, str | None]]:
    refs: list[tuple[str, str, str | None, str | None]] = []
    mpd_base = first_base_url(mpd)
    for period in child(mpd, "Period"):
        for aset in child(period, "AdaptationSet"):
            for rep in child(aset, "Representation"):
                levels = [rep, aset, period]
                chain = [b for b in (mpd_base, first_base_url(period), first_base_url(aset), first_base_url(rep)) if b is not None]
                if any(urlsplit(b).scheme not in ("", "file") for b in chain):
                    continue
                rid, bw = rep.attrib.get("id"), rep.attrib.get("bandwidth")
                templates = [effective(levels, "SegmentTemplate", a) for a in ("initialization", "media")]
                lists = [lvl for lvl in levels if child(lvl, "SegmentList")]
                if any(t is not None for t in templates):
                    for tmpl in templates:
                        if tmpl is not None:
                            refs.append(("template", resolve(prefix, chain, tmpl) or "", rid, bw))
                elif lists:
                    seg_list = child(lists[0], "SegmentList")[0]
                    for el in child(seg_list, "Initialization"):
                        if "sourceURL" in el.attrib:
                            refs.append(("file", resolve(prefix, chain, el.attrib["sourceURL"]) or "", rid, bw))
                    for el in child(seg_list, "SegmentURL"):
                        if "media" in el.attrib:
                            refs.append(("file", resolve(prefix, chain, el.attrib["media"]) or "", rid, bw))
                elif chain and not chain[-1].endswith("/"):
                    refs.append(("file", resolve(prefix, chain, "") or "", rid, bw))
    return refs


def check_references(base: pathlib.Path, mpd_path: pathlib.Path, mpd: ET.Element) -> list[str]:
    errs = []
    mpd_dir = mpd_path.parent.relative_to(base).as_posix()
    prefix = "" if mpd_dir == "." else mpd_dir + "/"
    files = disk_files(base)
    where = mpd_path.relative_to(base)
    for kind, path, rid, bw in representation_refs(prefix, mpd):
        if kind == "file":
            if path not in files:
                errs.append(f"{where}: referenced file {path} is missing")
        else:
            rx = template_regex(path, rid, bw)
            if not any(rx.fullmatch(f) for f in files):
                errs.append(f"{where}: no file matches template {path} (Representation {rid})")
    return errs


def dirs_with_media(base: pathlib.Path) -> set[pathlib.Path]:
    return {p.parent for p in base.rglob("*") if p.is_file() and p.suffix in MEDIA_SUFFIXES}


def verify(base: pathlib.Path) -> list[str]:
    errs = check_sums(base)
    media_dirs = dirs_with_media(base)
    for p in sorted(base.rglob("*")):
        if not p.is_file() or p.suffix not in XML_SUFFIXES:
            continue
        root, err = parse_xml(p)
        if err:
            errs.append(err)
        elif p.suffix == ".mpd" and p.parent in media_dirs:
            errs.extend(check_references(base, p, root))
    return errs


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--update", action="store_true", help="rewrite SHA256SUMS from the files on disk")
    ap.add_argument("--dir", type=pathlib.Path, default=DEFAULT_DIR, help="fixture directory (default: fixtures/dash)")
    args = ap.parse_args(argv)
    if args.update:
        update(args.dir)
        print(f"wrote {args.dir / SUMS}")
        return 0
    errs = verify(args.dir)
    for e in errs:
        print(e, file=sys.stderr)
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
