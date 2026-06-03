#!/usr/bin/env python3
"""Shrink TS test fixtures so each crate's tests stay <10 MB (crates.io limit).

Two modes:
  * --strip-es : parse PAT->PMT, drop video/audio/data elementary-stream PIDs
    (incl. PCR PID), keep PAT/PMT and every SI/PSI table. The SI parsers only
    look at tables, so the ES packets are pure weight.
  * --keep-pid PID --max-packets N : keep only one PID, truncated to N packets
    (for BBFrame captures carried on a single private-section PID).

Usage:
    python shrink_fixtures.py --strip-es in.ts out.ts
    python shrink_fixtures.py --keep-pid 0x010E --max-packets 9000 in.ts out.ts
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

PKT = 188


def packets(data: bytes):
    for i in range(0, len(data) - PKT + 1, PKT):
        p = data[i : i + PKT]
        if p[0] == 0x47:
            yield p


def pid_of(p: bytes) -> int:
    return ((p[1] & 0x1F) << 8) | p[2]


def section_payload(p: bytes) -> tuple[bytes, bool]:
    """Return (payload, pusi). Strips adaptation field; if PUSI, skips pointer_field."""
    pusi = bool(p[1] & 0x40)
    afc = (p[3] >> 4) & 0x3
    off = 4
    if afc in (2, 3):
        off += 1 + p[4]
    payload = p[off:]
    if pusi and payload:
        payload = payload[1 + payload[0] :]  # pointer_field
    return payload, pusi


def first_section_for_pid(data: bytes, pid: int) -> bytes | None:
    """Reassemble the first complete section seen on `pid` (good enough for PAT/PMT)."""
    buf = b""
    started = False
    for p in packets(data):
        if pid_of(p) != pid:
            continue
        payload, pusi = section_payload(p)
        if pusi:
            started = True
            buf = payload
        elif started:
            buf += p[4:] if (p[3] >> 4) & 0x3 in (1, 3) else b""
        if started and len(buf) >= 3:
            sec_len = ((buf[1] & 0x0F) << 8) | buf[2]
            if len(buf) >= 3 + sec_len:
                return buf[: 3 + sec_len]
    return None


# MPEG/DVB stream_types that are video or audio (the heavy payload to drop).
# Data/AIT/DSM-CC/subtitle/teletext streams are kept — the SI parsers test them.
VIDEO_TYPES = {0x01, 0x02, 0x10, 0x1B, 0x24, 0x42}
AUDIO_TYPES = {0x03, 0x04, 0x0F, 0x11, 0x1C, 0x81, 0x87}


def es_pids(data: bytes) -> set[int]:
    """PIDs of video/audio elementary streams (by stream_type), via PAT->PMT.

    Only video and audio are dropped; data carousels, AIT, DSM-CC, subtitle and
    teletext streams stay (the SI tests read several of them).
    """
    drop: set[int] = set()
    pat = first_section_for_pid(data, 0x0000)
    if not pat:
        return drop
    sec_len = ((pat[1] & 0x0F) << 8) | pat[2]
    body = pat[8 : 3 + sec_len - 4]  # skip 8-byte header, drop 4-byte CRC
    pmt_pids = []
    for i in range(0, len(body) - 3, 4):
        prog = (body[i] << 8) | body[i + 1]
        pid = ((body[i + 2] & 0x1F) << 8) | body[i + 3]
        if prog != 0:  # program 0 = network PID (NIT), keep it
            pmt_pids.append(pid)
    for pmt_pid in pmt_pids:
        pmt = first_section_for_pid(data, pmt_pid)
        if not pmt:
            continue
        prog_info_len = ((pmt[10] & 0x0F) << 8) | pmt[11]
        sl = ((pmt[1] & 0x0F) << 8) | pmt[2]
        i = 12 + prog_info_len
        end = 3 + sl - 4
        while i + 5 <= end:
            stream_type = pmt[i]
            es_pid = ((pmt[i + 1] & 0x1F) << 8) | pmt[i + 2]
            es_info_len = ((pmt[i + 3] & 0x0F) << 8) | pmt[i + 4]
            if stream_type in VIDEO_TYPES or stream_type in AUDIO_TYPES:
                drop.add(es_pid)
            i += 5 + es_info_len
    return drop


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--strip-es", action="store_true")
    ap.add_argument("--keep-pid", type=lambda s: int(s, 0))
    ap.add_argument("--max-packets", type=int)
    ap.add_argument("inp", type=Path)
    ap.add_argument("out", type=Path)
    a = ap.parse_args()
    data = a.inp.read_bytes()

    if a.strip_es:
        drop = es_pids(data)
        kept = b"".join(p for p in packets(data) if pid_of(p) not in drop)
        print(f"strip-es: dropped PIDs {sorted(hex(x) for x in drop)}")
    elif a.keep_pid is not None:
        out = []
        for p in packets(data):
            if pid_of(p) == a.keep_pid:
                out.append(p)
                if a.max_packets and len(out) >= a.max_packets:
                    break
        kept = b"".join(out)
    else:
        sys.exit("need --strip-es or --keep-pid")

    a.out.write_bytes(kept)
    print(f"{a.inp.name}: {len(data)/1e6:.1f} MB -> {a.out.name}: {len(kept)/1e6:.3f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
