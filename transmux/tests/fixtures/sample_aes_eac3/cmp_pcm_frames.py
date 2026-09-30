#!/usr/bin/env python3
"""cmp_pcm_frames.py REF.framemd5 OTHER.framemd5 [N] -- identical decoded audio frames (first N)."""
import sys
def h(p): return [l.split(',')[-1].strip() for l in open(p) if not l.startswith('#')]
a, b = h(sys.argv[1]), h(sys.argv[2]); n = int(sys.argv[3]) if len(sys.argv) > 3 else len(a)
a, b = a[:n], b[:n]; ok = sum(x == y for x, y in zip(a, b))
print(f"{sys.argv[2]}: {ok}/{n} decoded audio frames identical to {sys.argv[1]}  [{''.join('=' if x == y else 'X' for x, y in zip(a, b))}]")
