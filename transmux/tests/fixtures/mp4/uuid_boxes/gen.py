#!/usr/bin/env python3
"""Splice real `uuid` child boxes into tool-generated ISOBMFF files.

`ffmpeg`/`MP4Box`/Bento4 available here do not emit `uuid` boxes, so the
fixtures are built in two steps: a real muxer produces the file, and this
script inserts `uuid` children whose *payloads* are real, spec-defined
structures — a PlayReady `tfxd` ([MS-SSTR] §2.2.4.4, the extended type this
crate's `smooth` module implements) at `moof` and `traf` level, and a real
Widevine `pssh` body (from `fixtures/cpix/widevine-pssh-from-complex.bin`) at
`moov` level.

Sizes are patched by rebuilding the enclosing containers from their children,
so the result is structurally exact. Verified with `mp4dump` (Bento4) — see
README.md for the recorded output.

Usage:
    python3 gen.py <in.mp4> <out.moov.mp4> <out.segment.mp4>

Deterministic: run twice, byte-identical output.
"""
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[4]
WIDEVINE_PSSH = REPO / "fixtures" / "cpix" / "widevine-pssh-from-complex.bin"

# [MS-SSTR] §2.2.4.4 — the `tfxd` uuid extended type.
TFXD_UUID = bytes(
    [
        0x6D,
        0x1D,
        0x9B,
        0x05,
        0x42,
        0xD5,
        0x44,
        0xE6,
        0x80,
        0xE2,
        0x14,
        0x1D,
        0xAF,
        0xF7,
        0x57,
        0xB2,
    ]
)
# The ISMV/PlayReady `pssh` extended type this fixture renames the spliced
# `moov` uuid to, so the box names the structure it carries.
ISMV_PSSH_UUID = bytes.fromhex("d08a4f1810f34a82b6c832d8aba183d3")


def box(fourcc: bytes, body: bytes) -> bytes:
    return struct.pack(">I", 8 + len(body)) + fourcc + body


def uuid_box(usertype: bytes, body: bytes) -> bytes:
    assert len(usertype) == 16
    return box(b"uuid", usertype + body)


def tfxd(absolute_time: int, duration: int) -> bytes:
    """A `uuid` `tfxd` (§2.2.4.4): FullBox v1, then two 64-bit fields."""
    return uuid_box(TFXD_UUID, struct.pack(">B3xQQ", 1, absolute_time, duration))


def parse_top(data: bytes, start: int = 0, end: int | None = None):
    """Yield (fourcc, start, size) for every box in `data[start:end]`."""
    end = len(data) if end is None else end
    off = start
    while off + 8 <= end:
        size = struct.unpack(">I", data[off : off + 4])[0]
        if size < 8 or off + size > end:
            break
        yield data[off + 4 : off + 8], off, size
        off += size


def build(fourcc: bytes, children: list[bytes], header_extra: bytes = b"") -> bytes:
    """A container box from its children's already-encoded bytes."""
    body = header_extra + b"".join(children)
    return box(fourcc, body)


def container_children(data: bytes, off: int, size: int) -> list[bytes]:
    """The encoded child boxes of the container at `data[off:off+size]`."""
    return [data[koff : koff + ksize] for _, koff, ksize in parse_top(data, off + 8, off + size)]


def patch_size(buf: bytearray, off: int, delta: int) -> None:
    """Grow the box whose header starts at `off` by `delta` bytes."""
    size = struct.unpack(">I", buf[off : off + 4])[0]
    buf[off : off + 4] = struct.pack(">I", size + delta)


def insert_into(data: bytes, path: list[bytes], insert: list[bytes], after: bytes | None):
    """Insert `insert` as children of the container reached by `path`.

    `path[0]` is a top-level four-CC, `path[1:]` walk nested containers. Every
    enclosing box's size is grown by the inserted byte count (each insertion
    happens once, so a single pass down the chain is exact).
    """
    inserted = b"".join(insert)
    delta = len(inserted)
    # Locate the offsets of each level's header.
    offsets: list[int] = []
    level_data = data
    base = 0
    for want in path:
        found = None
        for cc, off, size in parse_top(level_data, base, len(level_data)):
            if cc == want:
                found = (off, size)
                break
        if found is None:
            raise SystemExit(f"box {want!r} not found for path {path!r}")
        offsets.append(found[0])
        base, level_data = found[0] + 8, data
    # Build the innermost container's new children.
    deepest_off, deepest_size = offsets[-1], None
    for cc, off, size in parse_top(data, deepest_off, len(data)):
        del cc
        deepest_size = size
        break
    del deepest_size
    inner_size = struct.unpack(">I", data[deepest_off : deepest_off + 4])[0]
    kids = container_children(data, deepest_off, inner_size)
    at = 0 if after is None else next(
        (i + 1 for i, k in enumerate(kids) if k[4:8] == after), 0
    )
    for j, ins in enumerate(insert):
        kids.insert(at + j, ins)
    rebuilt = build(path[-1], kids)
    out = bytearray(data[:deepest_off] + rebuilt + data[deepest_off + inner_size :])
    # Grow every ancestor (and the rebuilt container itself) by `delta`.
    for off in offsets[:-1]:
        patch_size(out, off, delta)
    return bytes(out)


def patch_trun_data_offsets(data: bytes, delta: int) -> bytes:
    """Add `delta` to every `trun.data_offset` in every `traf` (in place).

    `trun` is a FullBox: version/flags(4) then sample_count(4) then, when
    `data_offset` is present (flag 0x000001), the 4-byte offset. Only the
    `traf`s inside `moof`s are touched.
    """
    out = bytearray(data)
    for cc, moof_off, moof_size in parse_top(data):
        if cc != b"moof":
            continue
        for tcc, traf_off, traf_size in parse_top(data, moof_off + 8, moof_off + moof_size):
            if tcc != b"traf":
                continue
            for rcc, trun_off, _ in parse_top(data, traf_off + 8, traf_off + traf_size):
                if rcc != b"trun":
                    continue
                flags = struct.unpack(">I", data[trun_off + 8 : trun_off + 12])[0] & 0xFFFFFF
                if flags & 0x000001:
                    at = trun_off + 8 + 4 + 4
                    cur = struct.unpack(">i", data[at : at + 4])[0]
                    out[at : at + 4] = struct.pack(">i", cur + delta)
    return bytes(out)


def main() -> int:
    src, out_moov, out_seg = (Path(a) for a in sys.argv[1:4])
    data = src.read_bytes()

    # ---- 1. a `moov`-level uuid carrying a real Widevine pssh body ---------
    pssh_payload = WIDEVINE_PSSH.read_bytes()
    assert len(pssh_payload) >= 20, "widevine pssh payload"
    system_id = pssh_payload[:16]
    data_blob = pssh_payload[16:]
    uuid_pssh_body = (
        struct.pack(">I", 0)  # version 0, flags 0
        + system_id
        + struct.pack(">I", len(data_blob))
        + data_blob
    )
    moov_uuid = uuid_box(ISMV_PSSH_UUID, uuid_pssh_body)
    moov_out = insert_into(data, [b"moov"], [moov_uuid], after=None)
    out_moov.write_bytes(moov_out)

    # ---- 2. uuid children at `moof` and `traf` level ----------------------
    # Insert *after* the existing children so the fragment's moof-relative
    # `trun.data_offset` (which addresses a following `mdat`) keeps resolving:
    # growing a `moof` at its end only moves the `mdat` later by the same
    # amount, and `default-base-is-moof` is relative to the `moof` start.
    # Growing a `moof` moves the `mdat` that follows it, while every
    # `trun.data_offset` stays moof-relative — so each inserted byte must be
    # added to the offset(s) that address data past the insertion point.
    uuid_a = tfxd(1, 2_000_000)
    uuid_b = tfxd(0, 2_000_000)
    seg = insert_into(moov_out, [b"moof"], [uuid_a], after=b"traf")
    seg = patch_trun_data_offsets(seg, len(uuid_a))
    seg = insert_into(seg, [b"moof", b"traf"], [uuid_b], after=b"trun")
    seg = patch_trun_data_offsets(seg, len(uuid_b))
    out_seg.write_bytes(seg)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
