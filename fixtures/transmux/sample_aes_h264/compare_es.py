#!/usr/bin/env python3
"""compare_es.py REF.h264 OTHER.h264 [P]

Compare the slice NALs (types 1/5), emulation-prevention-normalised ,
of the first P pictures (all if omitted). ffmpeg's Sample-AES decryptor emits
NALs unescaped, so equality is only meaningful modulo 0x03 insertion.
Exit 0 when identical.
--len-tolerant: see eq().  --raw: compare escaped wire bytes (checks emulation prevention too)."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sample_aes_ref as r

def slices(p, P):
    out, pic = [], -1
    for n in r.split_nals(open(p, 'rb').read()):
        t = n[0] & 31
        if t in (1, 5):
            if n[1] & 0x80:
                pic += 1
            if P is None or pic < P:
                out.append(n if raw else r.unescape(n))
    return out

raw = '--raw' in sys.argv
if raw: sys.argv.remove('--raw')
tol = '--len-tolerant' in sys.argv
if tol: sys.argv.remove('--len-tolerant')
P = int(sys.argv[3]) if len(sys.argv) > 3 else None
a, b = slices(sys.argv[1], P), slices(sys.argv[2], P)
def eq(x, y):
    if not tol:
        return x == y
    # ffmpeg length-bookkeeping quirk after removing an emulation byte from
    # ciphertext: content equal over the common prefix, length off by <= 1
    return abs(len(x) - len(y)) <= 1 and x[:min(len(x), len(y)) - 1] == y[:min(len(x), len(y)) - 1]
same = sum(eq(x, y) for x, y in zip(a, b))
print(f"{sys.argv[2]}: {same}/{len(a)} slice NALs equal to {sys.argv[1]} (other has {len(b)})")
sys.exit(0 if same == len(a) == len(b) else 1)
