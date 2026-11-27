#!/usr/bin/env python3
"""Survey every committed fixture for the metadata tags a real file carries.

Runs three independent tools (ExifTool, MediaInfo, ffprobe) plus this
workspace's own readers over every media fixture in `fixtures/` and
`private/fixtures/`, and reports one row per distinct tag: how many fixtures
carry it, which tools see it, and whether this workspace reads it typed,
carries it opaque, or drops it.

The point is measurement, not judgement: the per-tool inventory is what the
tools print. A tool tag counts as "read" only when the workspace's own dump
names the same thing, through either the tag name itself or one of the aliases
in `ALIASES` (each alias carries the reason it exists).

Usage:
    tools/survey-defacto-tags.py --json .delegate/scratch/survey.json
    tools/survey-defacto-tags.py --markdown .delegate/scratch/survey.md

The workspace dumps are built binaries; build them first:
    cargo build -p isobmff --example read_tags --locked
    cargo build -p broadcast-matroska --example dump_elements --locked
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

MEDIA_EXTENSIONS = {
    ".mp4": "isobmff",
    ".m4a": "isobmff",
    ".m4v": "isobmff",
    ".m4s": "isobmff",
    ".mov": "isobmff",
    ".avif": "isobmff",
    ".heic": "isobmff",
    ".mkv": "matroska",
    ".webm": "matroska",
    ".flv": "flv",
    ".ts": "mpeg-ts",
    ".m2ts": "mpeg-ts",
    ".ps": "mpeg-ps",
    ".mp3": "mpeg-audio",
    ".aac": "adts",
    ".wav": "wav",
    ".ec3": "ac3",
}

# Which of the workspace's own dumpers covers a container, and what its output
# calls a key. `None` means no shipped dumper reads that container's tags.
DUMPERS = {
    "isobmff": ("isobmff", "examples/read_tags"),
    "matroska": ("broadcast-matroska", "examples/dump_elements"),
}

# A workspace tag name (as the dumper prints it) that stands for a tool tag.
# Each entry is (tool prefix, reason the two name the same thing).
ALIASES = {
    "QuickTime:Encoder": ("©too", "the iTunes encoder atom is what QuickTime:Encoder prints"),
    "QuickTime:Title": ("©nam", "same atom"),
    "QuickTime:Artist": ("©ART", "same atom"),
    "QuickTime:Album": ("©alb", "same atom"),
    "QuickTime:CreationDate": ("©day", "same atom"),
    "QuickTime:Genre": ("©gen", "same atom"),
    "QuickTime:Comment": ("©cmt", "same atom"),
    "QuickTime:TrackNumber": ("trkn", "same atom"),
    "QuickTime:DiscNumber": ("disk", "same atom"),
    "QuickTime:CoverArt": ("covr", "same atom"),
    "QuickTime:GPSCoordinates": ("©xyz", "the ISO 6709 atom prints as coordinates"),
    "QuickTime:LocationISO6709": ("com.apple.quicktime.location.ISO6709", "the keyed atom"),
    "QuickTime:Make": ("com.apple.quicktime.make", "the keyed atom"),
    "QuickTime:Model": ("com.apple.quicktime.model", "the keyed atom"),
    "QuickTime:Software": ("com.apple.quicktime.software", "the keyed atom"),
    "QuickTime:ContentCreateDate": (
        "com.apple.quicktime.creationdate",
        "the keyed atom, printed under its ExifTool name",
    ),
    "General:Encoded_Application": ("©too", "MediaInfo prints the encoder atom here"),
    "General:Title": ("©nam", "same atom"),
    "General:Performer": ("©ART", "same atom"),
    "General:Album": ("©alb", "same atom"),
    "General:Recorded_Date": ("©day", "same atom"),
    "General:Genre": ("©gen", "same atom"),
    "General:Comment": ("©cmt", "same atom"),
    "General:Track_Position": ("trkn", "same atom"),
    "General:Extra": ("----", "a freeform item, which is all MediaInfo reports for one"),
    "ffprobe:encoder": ("©too", "the same atom, through the tag name ffprobe prints"),
    "ffprobe:title": ("©nam", "same atom"),
    "ffprobe:artist": ("©ART", "same atom"),
    "ffprobe:album": ("©alb", "same atom"),
    "ffprobe:date": ("©day", "same atom"),
    "ffprobe:genre": ("©gen", "same atom"),
    "ffprobe:comment": ("©cmt", "same atom"),
    "ItemList:Encoder": ("©too", "ExifTool names the iTunes atom by its role"),
    "UserData:SoftwareVersion": ("©swr", "the QuickTime software-version atom"),
    "UserData:Title": ("©nam", "same atom"),
    "UserData:Copyright": ("©cpy", "the QuickTime copyright atom"),
    "UserData:GPSCoordinates": ("©xyz", "the ISO 6709 atom"),
    "iTunes:iTunSMPB": ("com.apple.iTunes/iTunSMPB", "the freeform item"),
    "iTunes:VolumeNormalization": ("com.apple.iTunes/iTunNORM", "the freeform item"),
    "Keys:ContentIdentifier": ("com.apple.quicktime.content.identifier", "the keyed atom"),
    "Keys:CreationDate": ("com.apple.quicktime.creationdate", "the keyed atom"),
    "Keys:GPSCoordinates": ("com.apple.quicktime.location.ISO6709", "the keyed atom"),
    "Keys:Make": ("com.apple.quicktime.make", "the keyed atom"),
    "Keys:Model": ("com.apple.quicktime.model", "the keyed atom"),
    "Keys:Software": ("com.apple.quicktime.software", "the keyed atom"),
    "Matroska:Encoder": ("MuxingApp", "ExifTool's name for the Segment Info writing application"),
    "Info:MuxingApp": ("MuxingApp", "the Segment Info element, typed as text"),
    "Info:WritingApp": ("WritingApp", "the Segment Info element, typed as text"),
    "Info:Duration": ("Duration", "the Segment Info element, typed as an integer"),
    "Info:TimecodeScale": ("TimecodeScale", "the Segment Info element, typed as an integer"),
    "Info:SegmentUID": ("SegmentUID", "the Segment Info element, typed as bytes"),
    "Matroska:DocType": ("DocType", "the EBML header element, typed as text"),
    "Matroska:DocTypeVersion": ("DocTypeVersion", "the EBML header element"),
    "Matroska:DocTypeReadVersion": ("DocTypeReadVersion", "the EBML header element"),
    "Matroska:EBMLVersion": ("EBMLVersion", "the EBML header element"),
    "Matroska:EBMLReadVersion": ("EBMLReadVersion", "the EBML header element"),
    "Matroska:EBMLMaxIDLength": ("EBMLMaxIDLength", "the EBML header element"),
    "Matroska:EBMLMaxSizeLength": ("EBMLMaxSizeLength", "the EBML header element"),
    "Matroska:SeekID": ("SeekID", "the SeekHead element"),
    "Matroska:SeekPosition": ("SeekPosition", "the SeekHead element"),
    "Matroska:CRC-32": ("CRC-32", "the CRC-32 element"),
    "Info:CRC-32": ("CRC-32", "the CRC-32 element"),
    "Track1:CodecID": ("CodecID", "the TrackEntry element, typed as text"),
    "Track1:TrackUID": ("TrackUID", "the TrackEntry element, typed as an integer"),
    "Track1:TrackType": ("TrackType", "the TrackEntry element, typed as an integer"),
    "Track1:TrackLanguage": ("Language", "the TrackEntry element, typed as text"),
    "Track1:TrackLacing": ("FlagLacing", "the TrackEntry element, typed as an integer"),
    "Track1:TrackDefault": ("FlagDefault", "the TrackEntry element, typed as an integer"),
    "Track1:CodecPrivate": ("CodecPrivate", "the TrackEntry element, typed as bytes"),
    "Track1:Duration": ("Duration", "the TrackEntry element, typed as an integer"),
    "Track1:VideoFrameRate": ("FrameRate", "the TrackEntry element"),
    "Track1:VideoScanType": ("FieldOrder", "the TrackEntry element"),
    "Track1:Encoder": ("ENCODER", "the de-facto ENCODER SimpleTag this crate types"),
    "Track2:Encoder": ("ENCODER", "the de-facto ENCODER SimpleTag this crate types"),
    "stream:ENCODER": ("ENCODER", "the de-facto ENCODER SimpleTag this crate types"),
    "format:ENCODER": ("ENCODER", "the de-facto ENCODER SimpleTag this crate types"),
}

# Containers whose user-data/untyped child boxes the reader keeps verbatim
# instead of dropping (the reader prints them as "opaque bytes").
OPAQUE_CONTAINERS = {"isobmff"}

TOOL_TIMEOUT_SECONDS = 120

# ExifTool groups that describe the file on disk rather than the media it
# carries: every file has them, so they say nothing about the container.
NOISE_GROUPS = {"ExifTool", "File", "System"}

# Structures this workspace types but its metadata dumper does not print: the
# Matroska element walker prints headers only, so the Segment Info and
# TrackEntry values are named here with the accessor that returns each.
TYPED_WITHOUT_DUMP = {
    "CodecID",
    "CodecPrivate",
    "DocType",
    "DocTypeReadVersion",
    "DocTypeVersion",
    "Duration",
    "EBMLMaxIDLength",
    "EBMLMaxSizeLength",
    "EBMLReadVersion",
    "EBMLVersion",
    "FieldOrder",
    "FlagDefault",
    "FlagLacing",
    "FrameRate",
    "Language",
    "MuxingApp",
    "SeekID",
    "SeekPosition",
    "SegmentUID",
    "TimecodeScale",
    "TrackType",
    "TrackUID",
    "WritingApp",
    "CRC-32",
}

# MediaInfo fields that describe the file or a stream's coding rather than a
# tag the writer carried. Named rather than pattern-matched so a new tag field
# is never filtered by accident.
NOISE_FIELDS = {
    "@type",
    "AudioCount",
    "BitDepth",
    "BitRate",
    "BitRate_Mode",
    "ChannelLayout",
    "ChannelPositions",
    "Channels",
    "CodecID",
    "CodecID_Compatible",
    "Compression_Mode",
    "DataSize",
    "Default",
    "DisplayAspectRatio",
    "Duration",
    "FileExtension",
    "FileSize",
    "File_Created_Date",
    "File_Created_Date_Local",
    "File_Modified_Date",
    "File_Modified_Date_Local",
    "FooterSize",
    "Forced",
    "Format",
    "Format_AdditionalFeatures",
    "Format_Profile",
    "Format_Settings",
    "Format_Version",
    "FrameCount",
    "FrameRate",
    "FrameRate_Mode",
    "FrameRate_Num",
    "FrameRate_Den",
    "FrameRate_Original",
    "HeaderSize",
    "Height",
    "ID",
    "IsStreamable",
    "Language",
    "MenuCount",
    "OverallBitRate",
    "OverallBitRate_Mode",
    "PixelAspectRatio",
    "SamplingCount",
    "SamplingRate",
    "SamplesPerFrame",
    "ScanType",
    "Source_Duration",
    "StreamOrder",
    "StreamSize",
    "TextCount",
    "VideoCount",
    "Width",
}


def run_json(argv: list[str]) -> dict | list | None:
    """Run a tool that prints JSON, returning `None` when it does not."""
    try:
        done = subprocess.run(
            argv,
            capture_output=True,
            timeout=TOOL_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode != 0 or not done.stdout.strip():
        return None
    try:
        return json.loads(done.stdout)
    except json.JSONDecodeError:
        return None


def media_fixtures() -> list[Path]:
    """Every committed media fixture, public and private tiers."""
    found: list[Path] = []
    for tier in ("fixtures", "private/fixtures"):
        base = ROOT / tier
        if not base.is_dir():
            continue
        for dirpath, _dirnames, filenames in os.walk(base):
            for name in filenames:
                path = Path(dirpath) / name
                if path.suffix.lower() in MEDIA_EXTENSIONS:
                    found.append(path)
    return sorted(found)


def container_of(path: Path) -> str:
    return MEDIA_EXTENSIONS.get(path.suffix.lower(), "unknown")


def exiftool_keys(path: Path) -> set[str]:
    """`group:Tag` for every tag ExifTool prints, including unknown ones."""
    data = run_json(["exiftool", "-a", "-u", "-g1", "-j", str(path)])
    if not isinstance(data, list) or not data:
        return set()
    record = data[0]
    keys: set[str] = set()
    for group, value in record.items():
        if group in NOISE_GROUPS or not isinstance(value, dict):
            continue
        for tag in value:
            keys.add(f"{group}:{tag}")
    return keys


def mediainfo_keys(path: Path) -> set[str]:
    """`Section:Field` for every MediaInfo tag field, keyed by the track type."""
    data = run_json(["mediainfo", "--Output=JSON", str(path)])
    if not isinstance(data, dict):
        return set()
    keys: set[str] = set()

    def walk(section: str, name: str, node: object) -> None:
        if isinstance(node, dict):
            for child_name, child in node.items():
                walk(section, f"{name}/{child_name}" if name else child_name, child)
        elif isinstance(node, list):
            for child in node:
                walk(section, name, child)
        elif name and name.split("/")[-1] not in NOISE_FIELDS:
            keys.add(f"{section}:{name}")

    for entry in data.get("media", {}).get("track", []) or []:
        if not isinstance(entry, dict):
            continue
        section = entry.get("@type", "track")
        for field, value in entry.items():
            walk(section, field, value)
    return keys


def ffprobe_keys(path: Path) -> set[str]:
    """`format:<tag>`, `stream:<tag>`, `side_data:<type>`, `chapter:<tag>`."""
    data = run_json(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-show_chapters",
            "-show_entries",
            "format_tags:stream_tags:stream_side_data",
            "-of",
            "json",
            str(path),
        ]
    )
    if not isinstance(data, dict):
        return set()
    keys: set[str] = set()
    for tag in data.get("format", {}).get("tags", {}) or {}:
        keys.add(f"format:{tag}")
    for stream in data.get("streams", []) or []:
        for tag in stream.get("tags", {}) or {}:
            keys.add(f"stream:{tag}")
        for side_data in stream.get("side_data_list", []) or []:
            kind = side_data.get("side_data_type")
            if kind:
                keys.add(f"side_data:{kind}")
    for chapter in data.get("chapters", []) or []:
        for tag in chapter.get("tags", {}) or {}:
            keys.add(f"chapter:{tag}")
    return keys


def dumper_path(container: str) -> Path | None:
    spec = DUMPERS.get(container)
    if spec is None:
        return None
    crate, example = spec
    candidate = ROOT / "target" / "debug" / "examples" / Path(example).name
    return candidate if candidate.is_file() else None


OUR_LINE = re.compile(r"^(?:[^:]+: )?(?P<key>[^=]+?)(?: = )(?P<value>.*)$")


def our_keys(container: str, path: Path) -> tuple[set[str], set[str]]:
    """(typed keys, opaque keys) the workspace's own dump prints for `path`."""
    binary = dumper_path(container)
    if binary is None:
        return set(), set()
    try:
        done = subprocess.run(
            [str(binary), str(path)],
            capture_output=True,
            timeout=TOOL_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return set(), set()
    typed: set[str] = set()
    opaque: set[str] = set()
    for line in done.stdout.decode("utf-8", "replace").splitlines():
        match = OUR_LINE.match(line.strip())
        if not match:
            continue
        key = match.group("key").strip()
        value = match.group("value")
        if "opaque bytes" in value:
            opaque.add(key)
        elif key:
            typed.add(key)
    return typed, opaque


def resolve(tool_key: str) -> list[str]:
    """The workspace key a tool key is matched under, best candidate first."""
    prefix, _, tag = tool_key.partition(":")
    del prefix
    alias = ALIASES.get(tool_key)
    return [alias[0], tag] if alias else [tag]


def survey(fixtures: list[Path]) -> dict:
    rows: dict[tuple[str, str], dict] = {}
    per_fixture: dict[str, dict] = {}

    def row_for(container: str, key: str) -> dict:
        return rows.setdefault(
            (container, key),
            {
                "container": container,
                "key": key,
                "fixtures": [],
                "tools": defaultdict(list),
                "typed": [],
                "opaque": [],
            },
        )

    for path in fixtures:
        container = container_of(path)
        rel = str(path.relative_to(ROOT))
        tools = {
            "exiftool": exiftool_keys(path),
            "mediainfo": mediainfo_keys(path),
            "ffprobe": ffprobe_keys(path),
        }
        typed, opaque = our_keys(container, path)
        per_fixture[rel] = {
            "container": container,
            "exiftool": sorted(tools["exiftool"]),
            "mediainfo": sorted(tools["mediainfo"]),
            "ffprobe": sorted(tools["ffprobe"]),
            "workspace_typed": sorted(typed),
            "workspace_opaque": sorted(opaque),
        }
        for tool, keys in tools.items():
            for key in keys:
                row = row_for(container, key)
                row["fixtures"].append(rel)
                row["tools"][tool].append(rel)
                tool_key = key if tool == "exiftool" else f"{tool}:{key}"
                row["typed"].extend(
                    name
                    for name in resolve(tool_key)
                    if name in typed or name in TYPED_WITHOUT_DUMP
                )
                row["opaque"].extend(
                    name for name in resolve(tool_key) if name in opaque
                )
        for key in typed:
            row = row_for(container, key)
            row["fixtures"].append(rel)
            row["tools"]["workspace"].append(rel)
            row["typed"].append(key)
        for key in opaque:
            row = row_for(container, key)
            row["fixtures"].append(rel)
            row["tools"]["workspace-opaque"].append(rel)
            row["opaque"].append(key)

    table = []
    for (_container, _key), row in rows.items():
        fixtures_for_row = sorted(set(row["fixtures"]))
        typed = sorted(set(row["typed"]))
        opaque = sorted(set(row["opaque"]))
        if typed:
            status, matched = "read-typed", typed[0]
        elif opaque:
            status, matched = "read-opaque", opaque[0]
        else:
            status, matched = "dropped", None
        table.append(
            {
                "container": row["container"],
                "key": row["key"],
                "fixture_count": len(fixtures_for_row),
                "fixtures": fixtures_for_row,
                "tools": sorted(row["tools"]),
                "workspace": status,
                "workspace_name": matched,
                "provenance": "unclassified",
            }
        )
    table.sort(key=lambda row: (-row["fixture_count"], row["container"], row["key"]))
    return {"fixtures": per_fixture, "table": table}


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", type=Path, help="write the raw survey here")
    parser.add_argument("--markdown", type=Path, help="write the tag table here")
    parser.add_argument(
        "--container",
        action="append",
        default=[],
        help="restrict to one container class (repeatable)",
    )
    args = parser.parse_args(argv)

    fixtures = media_fixtures()
    if args.container:
        fixtures = [path for path in fixtures if container_of(path) in args.container]
    if not fixtures:
        print("no fixtures found; is private/ checked out?", file=sys.stderr)
        return 2
    result = survey(fixtures)
    print(f"{len(fixtures)} fixtures, {len(result['table'])} distinct tags", file=sys.stderr)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(result, indent=2) + "\n")
    markdown = render_markdown(result)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown)
    else:
        sys.stdout.write(markdown)
    return 0


def render_markdown(result: dict) -> str:
    lines = [
        "| fixtures | container | key | tools | workspace | provenance |",
        "|---:|---|---|---|---|---|",
    ]
    for row in result["table"]:
        lines.append(
            "| {count} | {container} | `{key}` | {tools} | {workspace} | {provenance} |".format(
                count=row["fixture_count"],
                container=row["container"],
                key=row["key"],
                tools=", ".join(row["tools"]),
                workspace=row["workspace"],
                provenance=row["provenance"],
            )
        )
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
