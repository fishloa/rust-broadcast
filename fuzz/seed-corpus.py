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
