#!/usr/bin/env python3
"""ts_extract_es.py IN.ts OUT.bin PID -- concatenate the PES payload bytes of one PID."""
import sys
d = open(sys.argv[1], 'rb').read(); pid = int(sys.argv[3], 0); out = bytearray()
for p in range(0, len(d), 188):
    pk = d[p:p + 188]
    if pk[0] != 0x47 or (((pk[1] & 0x1f) << 8) | pk[2]) != pid:
        continue
    afc = (pk[3] >> 4) & 3
    off = 4 + (1 + pk[4] if afc & 2 else 0)
    if not afc & 1:
        continue
    if pk[1] & 0x40:
        off += 9 + pk[off + 8]
    out += pk[off:]
open(sys.argv[2], 'wb').write(out)
