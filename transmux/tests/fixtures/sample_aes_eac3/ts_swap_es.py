#!/usr/bin/env python3
"""ts_swap_es.py CLEAR.ts NEWES.bin OUT.ts PID

Replace the PES payload (elementary-stream bytes) of one PID in a TS with NEWES
(must be exactly as long as the original ES). Needed because ffmpeg cannot mux
ENCRYPTED E-AC-3 (its muxer must decode to learn the sample rate), so the TS is
built from the clear stream and the ES bytes are swapped in place."""
import sys
ts = bytearray(open(sys.argv[1], 'rb').read()); new = open(sys.argv[2], 'rb').read(); pid = int(sys.argv[4], 0)
pos = []
for p in range(0, len(ts), 188):
    pk = ts[p:p + 188]
    if pk[0] != 0x47 or (((pk[1] & 0x1f) << 8) | pk[2]) != pid:
        continue
    afc = (pk[3] >> 4) & 3
    off = 4 + (1 + pk[4] if afc & 2 else 0)
    if not afc & 1:
        continue
    if pk[1] & 0x40:                       # PES start: skip 9-byte header + header_data_length
        assert pk[off:off + 3] == b'\x00\x00\x01'
        off += 9 + pk[off + 8]
    pos.extend(range(p + off, p + 188))
assert len(pos) == len(new), (len(pos), len(new))
for i, b in zip(pos, new):
    ts[i] = b
open(sys.argv[3], 'wb').write(ts)
