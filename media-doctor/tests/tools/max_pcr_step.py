#!/usr/bin/env python3
"""Independent oracle: per-PID max PCR value step in a MPEG-2 TS file.

Deliberately shares no code with `media-doctor` or `mpeg-ts`: it walks the
raw 188-byte packet lattice itself, reads the 6-byte PCR field of every
adaptation field, and reports the largest modular step between consecutive
PCRs on each PID.

Usage: max_pcr_step.py FILE [FILE...]
Prints one line per (file, PID):  FILE pid=0xPPPP pcr_n=N max_step_ms=M discontinuity_indicator_seen=yes/no
"""

import sys

PACKET = 188
SYNC = 0x47
PCR_MODULUS = (1 << 33) * 300
CLOCK_27MHZ = 27_000_000


def max_step_ms(path):
    with open(path, "rb") as fh:
        data = fh.read()
    per_pid = {}
    for off in range(0, len(data) - PACKET + 1, PACKET):
        p = data[off:off + PACKET]
        if p[0] != SYNC:
            continue
        pid = ((p[1] & 0x1F) << 8) | p[2]
        afc = (p[3] >> 4) & 0x03
        if not (afc & 0x02):          # no adaptation field => no PCR
            continue
        af_len = p[4]
        if af_len == 0:
            continue
        flags = p[5]
        if not (flags & 0x10):        # PCR_flag
            continue
        base = ((p[6] << 25) | (p[7] << 17) | (p[8] << 9) | (p[9] << 1) | (p[10] >> 7))
        ext = ((p[10] & 0x01) << 8) | p[11]
        pcr = base * 300 + ext
        disc = bool(flags & 0x80)
        per_pid.setdefault(pid, []).append((pcr, disc))

    out = []
    for pid, seq in sorted(per_pid.items()):
        if len(seq) < 2:
            out.append((pid, len(seq), 0.0, any(d for _, d in seq)))
            continue
        worst = 0
        disc_seen = False
        for (a, ad), (b, bd) in zip(seq, seq[1:]):
            disc_seen = disc_seen or ad or bd
            step = (b - a) % PCR_MODULUS
            # A backward step (i.e. a wrapped/negative difference) is reported
            # as a negative number: 2.3b is defined on the signed difference
            # being outside 0..100 ms.
            signed = step if step <= PCR_MODULUS // 2 else step - PCR_MODULUS
            if abs(signed) > abs(worst):
                worst = signed
        out.append((pid, len(seq), worst * 1000.0 / CLOCK_27MHZ, disc_seen))
    return out


def main(argv):
    for path in argv[1:]:
        try:
            rows = max_step_ms(path)
        except OSError as e:
            print(f"{path}: ERROR {e}")
            continue
        if not rows:
            print(f"{path}: no PCR")
            continue
        for pid, n, step_ms, disc in rows:
            print(
                f"{path} pid=0x{pid:04X} pcr_n={n} max_step_ms={step_ms:.3f} "
                f"discontinuity_indicator_seen={'yes' if disc else 'no'}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
