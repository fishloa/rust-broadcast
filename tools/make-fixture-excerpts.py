#!/usr/bin/env python3
"""Deterministic excerpt maker for the large real-capture fixtures (#1263).

The full captures (france-tnt-uhf32.ts 187 MB, hotbird-mhp.ts 352 MB) do not
fit a plain-git private repo, and no test needs their video payload. This tool
derives small excerpts that keep every byte the tests read exactly as captured.

  python3 tools/make-fixture-excerpts.py si-audio  SRC.ts OUT.ts
  python3 tools/make-fixture-excerpts.py si        SRC.ts OUT.ts
  python3 tools/make-fixture-excerpts.py si-lean   SRC.ts OUT.ts
  python3 tools/make-fixture-excerpts.py prefix N  SRC.ts OUT.ts   (N in bytes)
  python3 tools/make-fixture-excerpts.py window START N SRC.ts OUT.ts (bytes)

Packet filters (modes `si-audio` / `si`), applied packet by packet, order kept:
  * PAT/PMT-listed PIDs are classified by PMT stream_type: video =
    {0x01,0x02,0x10,0x1B,0x24,0x20,0x21,0x42}; audio = {0x03,0x04,0x0F,0x11,0x81}.
    `si-audio` drops video; `si` drops video AND audio; `si-lean` is `si`
    without the PES-start rule below (AF-interesting packets only). Every other PID
    (PSI/SI, EIT/TOT/SDT, teletext, subtitles, data carousels, private data,
    unreferenced PIDs) is kept byte-exact.
  * A dropped-class packet is STILL kept when it is "interesting":
      - its adaptation field carries any of PCR/OPCR/splicing_point/
        transport_private_data/extension (flags 0x10|0x08|0x04|0x02|0x01), so
        PCR clocks, AF-extension evidence and damaged-AF cases survive; or
      - it starts a PES packet (payload_unit_start_indicator = 1), so the real
        PES headers (PTS/DTS flags, stuffing, extension bits) survive.
  * Kept packets are copied unmodified, INCLUDING continuity_counter, so the
    stream keeps the original CC gaps where packets were removed. Nothing is
    re-stamped. Consumers needing video must use a `prefix` excerpt.

`prefix N`: the first N bytes truncated to a whole TS packet, video intact.
`window START N`: N bytes (whole packets) beginning at the first PAT packet
(PID 0, payload_unit_start) at or after byte START, video intact, so the window
opens at a PSI boundary like a fresh capture would.
All modes are pure functions of (source bytes, arguments): no timestamps, no
randomness. The tool prints the source SHA-256, packet counts and output
SHA-256 so PROVENANCE.md can record them.
"""
import hashlib
import sys

TS = 188
VIDEO = {0x01, 0x02, 0x10, 0x1B, 0x24, 0x20, 0x21, 0x42}
AUDIO = {0x03, 0x04, 0x0F, 0x11, 0x81}
AF_INTERESTING = 0x10 | 0x08 | 0x04 | 0x02 | 0x01


def pid_of(p):
    return ((p[1] & 0x1F) << 8) | p[2]


def af_flags(p):
    """Adaptation-field flags byte, or None when the packet has no usable AF."""
    if (p[3] >> 4) & 0x2 and p[4] >= 1:
        return p[5]
    return None


def classify(data):
    """PID -> stream_type from every PMT in the capture (first mapping wins)."""
    pmt_pids = set()
    types = {}
    bufs = {}
    n = len(data) // TS
    for i in range(n):
        p = data[i * TS:(i + 1) * TS]
        if p[0] != 0x47:
            continue
        pid = pid_of(p)
        if pid != 0 and pid not in pmt_pids:
            continue
        if not (p[3] >> 4) & 0x1:
            continue
        off = 4 + (1 + p[4] if (p[3] >> 4) & 0x2 else 0)
        pl = p[off:]
        if p[1] & 0x40:
            if not pl:
                continue
            ptr = pl[0]
            bufs[pid] = bytearray(pl[1 + ptr:])
        elif pid in bufs:
            bufs[pid] += pl
        else:
            continue
        b = bufs[pid]
        while len(b) >= 3 and b[0] != 0xFF:
            ln = ((b[1] & 0x0F) << 8) | b[2]
            if len(b) < 3 + ln:
                break
            sec = bytes(b[:3 + ln])
            del b[:3 + ln]
            if sec[0] == 0x00 and pid == 0:
                for j in range(8, len(sec) - 4, 4):
                    prog = (sec[j] << 8) | sec[j + 1]
                    if prog != 0:
                        pmt_pids.add(((sec[j + 2] & 0x1F) << 8) | sec[j + 3])
            elif sec[0] == 0x02 and pid != 0:
                pil = ((sec[10] & 0x0F) << 8) | sec[11]
                j = 12 + pil
                while j + 5 <= len(sec) - 4:
                    st = sec[j]
                    epid = ((sec[j + 1] & 0x1F) << 8) | sec[j + 2]
                    esl = ((sec[j + 3] & 0x0F) << 8) | sec[j + 4]
                    types.setdefault(epid, st)
                    j += 5 + esl
    return types


def main():
    mode = sys.argv[1]
    start = 0
    if mode == "prefix":
        n_bytes, src, dst = int(sys.argv[2]), sys.argv[3], sys.argv[4]
    elif mode == "window":
        start, n_bytes, src, dst = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], sys.argv[5]
    else:
        src, dst = sys.argv[2], sys.argv[3]
    data = open(src, "rb").read()
    src_sha = hashlib.sha256(data).hexdigest()
    out = bytearray()
    stats = {"in": len(data) // TS, "out": 0}
    if mode in ("prefix", "window"):
        if mode == "window":
            i = (start + TS - 1) // TS
            while i * TS + TS <= len(data):
                p = data[i * TS:(i + 1) * TS]
                if p[0] == 0x47 and pid_of(p) == 0 and p[1] & 0x40:
                    break
                i += 1
            start = i * TS
            print("window starts at byte", start, "packet", i)
        out = bytearray(data[start: start + n_bytes // TS * TS])
        stats["out"] = len(out) // TS
    else:
        types = classify(data)
        drop = set(pid for pid, st in types.items() if st in VIDEO)
        pes_start = mode != "si-lean"
        if mode in ("si", "si-lean"):
            drop |= set(pid for pid, st in types.items() if st in AUDIO)
        elif mode != "si-audio":
            sys.exit("unknown mode " + mode)
        kept_special = 0
        for i in range(len(data) // TS):
            p = data[i * TS:(i + 1) * TS]
            if p[0] == 0x47 and pid_of(p) in drop:
                f = af_flags(p)
                special = (f is not None and f & AF_INTERESTING) or (pes_start and p[1] & 0x40)
                if not special:
                    continue
                kept_special += 1
            out += p
            stats["out"] += 1
        stats["dropped_pids"] = sorted(drop)
        stats["kept_from_dropped_pids"] = kept_special
    open(dst, "wb").write(out)
    print("source sha256", src_sha)
    print("source packets", stats["in"], "excerpt packets", stats["out"])
    print("excerpt bytes", len(out), "sha256", hashlib.sha256(out).hexdigest())
    for k in ("dropped_pids", "kept_from_dropped_pids"):
        if k in stats:
            print(k, stats[k])


if __name__ == "__main__":
    main()
