#!/usr/bin/env python3
"""Derive the wrapped-v0 and synthetic-v2 fixtures from their real sources.

Run from the repository root:

    python3 transmux/tests/fixtures/audio_srat/gen.py

The script performs the two byte patches the README documents, and nothing
else. It never invents a file: `ipcm_v1_192k.mp4` and `qt_alac_v1.mov` come
from ffmpeg (see the README for the exact commands), and the derived files
differ from them only in the sample-entry rate/version signalling.
"""
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def stsd_entry(data: bytes) -> tuple[int, int]:
    """Return (stsd box offset, sample-entry offset) for the first stsd."""
    at = data.find(b"stsd") - 4
    if at < 0:
        raise SystemExit("no stsd box found")
    return at, at + 16


def patch_ancestors(data: bytearray, at: int, start: int, delta: int) -> None:
    """Add `delta` to the size of every box containing offset `start`."""
    for fourcc in (b"moov", b"trak", b"mdia", b"minf", b"stbl"):
        pos = data.find(fourcc)
        if pos < 0:
            raise SystemExit(f"no {fourcc} box found")
        box = pos - 4
        if not box < at:
            raise SystemExit(f"{fourcc} does not contain the stsd")
        size = struct.unpack(">I", data[box : box + 4])[0]
        data[box : box + 4] = struct.pack(">I", size + delta)


def make_wrapped_v0(src: Path, dst: Path) -> None:
    """Plain AudioSampleEntry form: stsd version 0, no srat, 16.16 samplerate.

    The 16.16 field keeps the value the *encoder* wrote for a rate that does not
    fit it (192000 << 16 truncated to 32 bits = 0xEE000000, i.e. 60928 Hz).
    """
    data = bytearray(src.read_bytes())
    stsd, entry = stsd_entry(bytes(data))
    size = struct.unpack(">I", data[stsd : stsd + 4])[0]
    n = struct.unpack(">I", data[entry : entry + 4])[0]
    if data[entry + 40 : entry + 44] != b"srat":
        raise SystemExit("source is not the v1+srat form")
    e = bytearray(data[entry : entry + n])
    e[16:18] = b"\x00\x00"                     # entry_version -> 0
    e[32:36] = ((192_000 << 16) & 0xFFFF_FFFF).to_bytes(4, "big")
    del e[36:52]                               # drop the srat child
    e[0:4] = struct.pack(">I", len(e))
    newe = bytearray(data[stsd : stsd + 16]) + e
    newe[8] = 0                                # stsd version -> 0
    newe[0:4] = struct.pack(">I", len(newe))
    delta = len(newe) - size
    out = bytearray(data[:stsd]) + newe + bytearray(data[stsd + size :])
    patch_ancestors(out, stsd, stsd, delta)
    dst.write_bytes(out)
    print(f"wrote {dst} (delta {delta})")


def make_synthetic_v2(src: Path, dst: Path) -> None:
    """A QuickTime v2 sound description: same bytes, entry_version = 2.

    No locally-installed tool writes a QuickTime v2 entry (ffmpeg writes v0/v1),
    so this is derived from the real v1 file by patching the version field and
    the `revision_level`/`vendor` bytes that the crate used to discard. It
    exists to pin the round-trip, and the README says so.
    """
    data = bytearray(src.read_bytes())
    _, entry = stsd_entry(bytes(data))
    data[entry + 16 : entry + 18] = (2).to_bytes(2, "big")
    data[entry + 18 : entry + 24] = bytes([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF])
    dst.write_bytes(data)
    print(f"wrote {dst}")


def main() -> None:
    make_wrapped_v0(HERE / "ipcm_v1_192k.mp4", HERE / "ipcm_v0_192k_wrapped.mp4")
    make_synthetic_v2(HERE / "qt_alac_v1.mov", HERE / "qt_alac_v2_synthetic.mov")


if __name__ == "__main__":
    sys.exit(main())
