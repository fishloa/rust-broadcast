#!/usr/bin/env python3
"""frag_samples.py FILE.mp4 [--dump]   (spec-derived sample extractor, ISO/IEC 14496-12 §8.8.7/§8.8.8)

Prints one line per track: `track <id>: <n> samples sha256=<hash over all sample bytes>` and
(--dump) each sample's (moof#, traf#, trun#, file offset, size).  Base data offset rules (§8.8.7.1):
  * tfhd base_data_offset present (0x01)  -> that absolute offset
  * else default-base-is-moof (0x20000)   -> start of the enclosing moof
  * else (omit case): first traf -> start of the moof; later trafs -> END OF THE DATA of the
    preceding traf (i.e. where the previous traf's last trun ended)
  trun data_offset present (0x01) -> base + data_offset; absent -> continues right after the
  previous trun's data (first trun: the base itself).
Works on clear and on CENC-encrypted files (the sample bytes are then ciphertext)."""
import struct, sys, hashlib

def boxes(b, base=0):
    o = 0
    while o < len(b):
        n = struct.unpack('>I', b[o:o + 4])[0]
        yield b[o + 4:o + 8], b[o + 8:o + n], base + o, n
        o += n

def samples(d, dump=False):
    tracks = {}
    for t, body, moof_pos, _ in boxes(d):
        if t != b'moof':
            continue
        prev_end = None; mi = moof_pos
        ti = -1
        for ct, cb, _, _ in boxes(body):
            if ct != b'traf':
                continue
            ti += 1
            fl = tid = None; bdo = None; dsize = 0; nrun = 0; cursor = None
            for kt, kb, _, _ in boxes(cb):
                if kt == b'tfhd':
                    fl = struct.unpack('>I', kb[:4])[0] & 0xffffff; tid = struct.unpack('>I', kb[4:8])[0]; o = 8
                    if fl & 1: bdo = struct.unpack('>Q', kb[o:o + 8])[0]; o += 8
                    if fl & 2: o += 4
                    if fl & 8: o += 4
                    if fl & 0x10: dsize = struct.unpack('>I', kb[o:o + 4])[0]; o += 4
                    if bdo is not None: base = bdo
                    elif fl & 0x20000: base = moof_pos
                    else: base = moof_pos if prev_end is None else prev_end
                    cursor = base
                elif kt == b'trun':
                    f = struct.unpack('>I', kb[:4])[0] & 0xffffff; n = struct.unpack('>I', kb[4:8])[0]; o = 8
                    if f & 1:
                        cursor = base + struct.unpack('>i', kb[o:o + 4])[0]; o += 4
                    if f & 4: o += 4
                    for i in range(n):
                        if f & 0x100: o += 4
                        sz = dsize
                        if f & 0x200: sz = struct.unpack('>I', kb[o:o + 4])[0]; o += 4
                        if f & 0x400: o += 4
                        if f & 0x800: o += 4
                        tracks.setdefault(tid, []).append(d[cursor:cursor + sz])
                        if dump: print(f"  moof@{mi} traf{ti} trun{nrun} track{tid} off={cursor} size={sz}")
                        cursor += sz
                    nrun += 1
            prev_end = cursor
    return tracks

if __name__ == '__main__':
    tr = samples(open(sys.argv[1], 'rb').read(), '--dump' in sys.argv)
    for k in sorted(tr):
        print(f"track {k}: {len(tr[k])} samples sha256={hashlib.sha256(b''.join(tr[k])).hexdigest()[:16]}")
