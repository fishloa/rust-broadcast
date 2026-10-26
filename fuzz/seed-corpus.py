#!/usr/bin/env python3
"""Regenerate fuzz seed corpora from the repository's tracked fixtures.

`fuzz/corpus/` is gitignored (see `fuzz/.gitignore`), so seeds cannot simply be
committed. This script rebuilds them from fixtures that *are* tracked, which
keeps the generator under review while the generated blobs stay out of git.

Why this exists at all, for `container_probe`: that crate's whole job is
recognising real file headers, and libFuzzer starting from an empty corpus will
essentially never synthesise a valid MXF partition-pack key, an EBML magic
sequence, or a self-consistent ADTS frame-length chain. Without seeds the smoke
run explores the "nothing matched" path and little else -- it exercises the
crate's exits, not its logic. Seeding with real fixture heads puts the fuzzer
inside each prober's decision tree, where the interesting arithmetic lives (the
VINT width shift, the ISOBMFF largesize walk, the BER length decode).

The head sizes straddle the decision boundaries the probers actually branch on:
below every header minimum, one byte short of a full key, mid-chain (the
`Insufficient` path), and past the strong-lattice threshold.

Usage:
    python3 fuzz/seed-corpus.py            # regenerate every corpus
    python3 fuzz/seed-corpus.py --check    # exit 1 if any source fixture is missing

Idempotent: writes the same names and bytes every run, so re-running never
grows the corpus. Fixtures absent from a shallow/public clone are skipped with a
warning rather than failing, so this stays a no-op where the files do not exist.
"""

from __future__ import annotations

import argparse
import pathlib
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent

# Sizes are chosen per boundary, not as round numbers:
#   3    - shorter than every prober's header minimum (the Insufficient floor)
#   15   - one byte short of the 16-byte MXF key / ASF GUID
#   64   - a header decodes but no frame chain reaches its threshold
#   512  - a TS lattice clears 188*2 but not the 8-sync strong threshold
#   4096 - past every strong threshold and the default probe budget's working set
HEAD_SIZES = (3, 15, 64, 512, 4096)

CORPORA: dict[str, tuple[str, ...]] = {
    "container_probe": (
        "fixtures/ts/h264_aac.ts",
        "fixtures/mp4/h264_high.mp4",
        "fixtures/mp4/cmaf/av_frag.mp4",
        "fixtures/mkv/h264_aac.mkv",
        "fixtures/mxf/op1a_mpeg2_pcm.mxf",
        "fixtures/ps/h264_ac3.ps",
        "fixtures/flv/av.flv",
        "fixtures/container-probe/pcm_s16le.wav",
        "fixtures/container-probe/opus.ogg",
        "fixtures/container-probe/video.asf",
        "fixtures/container-probe/aac.adts",
        "fixtures/container-probe/audio.mp3",
        "fixtures/container-probe/h264.annexb",
    ),
}


# Whole-file seeds (an MPD is XML: a head is just a truncated document). `dash_address` decodes
# its bytes as numbers, so the same MPDs only seed the neighbourhood of "mostly zero"; its own
# structure comes from libFuzzer's mutations.
WHOLE_FILE_GLOBS: dict[str, tuple[str, ...]] = {
    "dash_mpd_parse": (
        "fixtures/dash/**/*.mpd",
        "transmux/tests/golden/*.mpd",
        "multimux/tests/golden/dash_mpd*.golden",
        "multimux/tests/golden/ll_dash_mpd.golden",
    ),
    "dash_address": ("fixtures/dash/**/*.mpd",),
}


def seed_whole(target: str, patterns: tuple[str, ...], check_only: bool) -> int:
    out = REPO / "fuzz" / "corpus" / target
    files = sorted({p for pat in patterns for p in REPO.glob(pat)})
    if check_only:
        return len(files)
    out.mkdir(parents=True, exist_ok=True)
    for path in files:
        name = "_".join(path.relative_to(REPO).parts)
        (out / name).write_bytes(path.read_bytes())
    return len(files)


# `broadcast_client` takes no file bytes: its input is an 8-byte Config header (two bits per knob
# selecting a row of each knob table, 0xAA = the documented defaults) followed by 32-byte steering
# chunks. The TS payloads are compiled into the target (four real fixtures plus a PMT-flipped
# copy, source index 4), so the seeds are scripted scenarios over those, not fixture heads.
CHUNK = 32
DEFAULT_HEADER = bytes([0xAA] * 8)
EXTREME_HEADER = bytes(8)
# b2 doubles as the clock selector (b2 % 24: 0 jumps 20 s, 4..23 advance one step) and, for the
# player arm, as its action selector (b2 % 5).
CLOCK_JUMP = 0
CLOCK_STEP = 4
PLAYER_POSITION = 5
PLAYER_RENDERED = 7
SERVE = 0
RECONNECT = 4
PLAYER = 10
IDLE = 9
RTSP_PUMP = 0
RTSP_FRAMES = 2
RTSP_RUN = 8
IDS_REAL = 0b0000011
IDS_WITH_UNKNOWN = 0b1100011
IDS_ONLY_UNKNOWN = 0b1100000
SOURCES = 5
PMT_FLIP_SOURCE = 4


def step(pick: int, b1: int = 0, b2: int = CLOCK_STEP, b3: int = 0, b4: int = 0, b6: int = 0) -> bytes:
    chunk = bytearray(CHUNK)
    chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[6] = pick, b1, b2, b3, b4, b6
    return bytes(chunk)


def script(header: bytes, *steps: bytes) -> bytes:
    return header + b"".join(steps)


def scenarios() -> dict[str, bytes]:
    serve_each = [
        piece
        for src in range(SOURCES)
        for piece in (step(IDLE, b2=CLOCK_JUMP), step(SERVE, b6=src))
    ]
    reconnect_other = [
        piece
        for src in range(SOURCES)
        for piece in (step(SERVE, b6=src), step(RECONNECT), step(IDLE, b2=CLOCK_JUMP))
    ]
    pmt_flip = [
        step(SERVE, b6=PMT_FLIP_SOURCE),
        step(PLAYER, b2=PLAYER_RENDERED, b3=IDS_REAL),
        step(PLAYER, b2=PLAYER_POSITION, b3=1, b4=8),
        step(PLAYER, b2=PLAYER_POSITION, b3=1, b4=64),
    ]
    unknown_ids = [
        step(SERVE, b6=0),
        step(PLAYER, b2=PLAYER_RENDERED, b3=IDS_ONLY_UNKNOWN),
        step(PLAYER, b2=PLAYER_RENDERED, b3=IDS_WITH_UNKNOWN),
        step(PLAYER, b2=PLAYER_POSITION, b3=1, b4=32),
        step(IDLE, b2=CLOCK_JUMP),
        step(SERVE, b6=1),
    ]
    rtsp = [
        step(RTSP_PUMP),
        step(RTSP_PUMP),
        *[step(RTSP_FRAMES, b1=n) for n in range(24)],
        step(RTSP_RUN, b1=16),
        step(PLAYER, b2=PLAYER_RENDERED, b3=IDS_WITH_UNKNOWN),
    ]
    return {
        "serve_each_fixture": script(DEFAULT_HEADER, *serve_each),
        "serve_each_fixture_extreme_config": script(EXTREME_HEADER, *serve_each),
        "reconnect_to_other_stream": script(DEFAULT_HEADER, *reconnect_other),
        "pmt_flip_explicit_declaration": script(DEFAULT_HEADER, *pmt_flip),
        "unknown_rendered_ids": script(DEFAULT_HEADER, *unknown_ids),
        "rtsp_play_and_packet_runs": script(DEFAULT_HEADER, *rtsp),
    }


def seed_scripts(target: str, check_only: bool) -> int:
    if check_only:
        return 0
    out = REPO / "fuzz" / "corpus" / target
    out.mkdir(parents=True, exist_ok=True)
    blobs = scenarios()
    for name, blob in blobs.items():
        (out / f"scenario_{name}").write_bytes(blob)
    return len(blobs)


# `dash_client_static`: 4-byte header (b0: window-1 in bits 0-1, failure cap row in bits 2-3, bit 4
# pins the origin; b1: 0 = the fuzz bytes are the MPD, 1..=2 = the compiled-in fixture MPD; b2/b3:
# length of an arbitrary MPD) then 2-byte steering chunks (mode, pick). Mode 0..=4 answers with the
# real file, 5 truncates it, 6 flips a byte, 7 sends arbitrary bytes, 8..=11 fail.
DASH_CLIENT_FIXTURE_MPDS = (1, 2)
DASH_CLIENT_STEPS = 48


def dash_client_scenarios() -> dict[str, bytes]:
    out: dict[str, bytes] = {}
    for sel in DASH_CLIENT_FIXTURE_MPDS:
        for window in (0, 3):
            header = bytes([window, sel, 0, 0])
            clean = b"".join(bytes([0, n % 7]) for n in range(DASH_CLIENT_STEPS))
            out[f"mpd{sel}_window{window + 1}_clean"] = header + clean
            damaged = b"".join(bytes([n % 12, n * 5 % 256]) for n in range(DASH_CLIENT_STEPS))
            out[f"mpd{sel}_window{window + 1}_damaged"] = header + damaged
            flaky = b"".join(bytes([9 if n % 5 == 2 else 0, 0]) for n in range(DASH_CLIENT_STEPS))
            out[f"mpd{sel}_window{window + 1}_flaky"] = bytes([window | 0b1000, sel, 0, 0]) + flaky
    return out


def seed_dash_client(check_only: bool) -> int:
    if check_only:
        return 0
    out = REPO / "fuzz" / "corpus" / "dash_client_static"
    out.mkdir(parents=True, exist_ok=True)
    blobs = dash_client_scenarios()
    for name, blob in blobs.items():
        (out / f"scenario_{name}").write_bytes(blob)
    return len(blobs)


def seed(target: str, sources: tuple[str, ...], check_only: bool) -> tuple[int, list[str]]:
    out = REPO / "fuzz" / "corpus" / target
    if not check_only:
        out.mkdir(parents=True, exist_ok=True)
    written = 0
    missing: list[str] = []
    for rel in sources:
        path = REPO / rel
        if not path.exists():
            missing.append(rel)
            continue
        if check_only:
            continue
        data = path.read_bytes()
        stem = pathlib.Path(rel).stem
        ext = pathlib.Path(rel).suffix.lstrip(".")
        for size in HEAD_SIZES:
            blob = data[:size]
            if not blob:
                continue
            # Name by actual length, not requested: a fixture shorter than the
            # requested head would otherwise write two names for one blob.
            (out / f"{stem}_{ext}_{len(blob)}").write_bytes(blob)
            written += 1
    return written, missing


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--check",
        action="store_true",
        help="report missing source fixtures and exit non-zero; write nothing",
    )
    args = ap.parse_args()

    all_missing: list[str] = []
    scripted = seed_scripts("broadcast_client", args.check)
    if not args.check:
        print(f"broadcast_client: {scripted} scripted scenario seeds")
    dash_client = seed_dash_client(args.check)
    if not args.check:
        print(f"dash_client_static: {dash_client} scripted scenario seeds")
    for target, patterns in WHOLE_FILE_GLOBS.items():
        count = seed_whole(target, patterns, args.check)
        if not args.check:
            print(f"{target}: {count} whole-file seeds")
    for target, sources in CORPORA.items():
        written, missing = seed(target, sources, args.check)
        all_missing += missing
        if not args.check:
            print(f"{target}: {written} seeds from {len(sources) - len(missing)} fixtures")
        for m in missing:
            print(f"  warning: fixture not present, skipped: {m}", file=sys.stderr)

    if args.check and all_missing:
        print(f"{len(all_missing)} source fixture(s) missing", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
