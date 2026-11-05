#!/usr/bin/env python3
"""relayout.py IN.mp4 OUT.mp4 --layout default|base_moof|omit [--split none|explicit|implicit]

Rewrites the movie-fragment addressing of a fragmented MP4 WITHOUT touching a single
sample byte (mdat is copied verbatim; all crypto boxes tfdt/saiz/senc/... are copied
verbatim except that every `saio` offset is re-pointed at its moved `senc`).
This exists because the independent encryptor we use (Bento4 mp4encrypt) always
re-writes fragments to the default-base-is-moof layout, and ffmpeg's own CENC muxer
mis-encrypts the audio of fragments >= 2 when video+audio are muxed (documented in
README.md), so the only way to get a *correctly encrypted* file in every ISO/IEC
14496-12 §8.8.7 addressing form is to re-address a correctly encrypted one.

Layouts (§8.8.7.1 base data offset):
  default    tfhd carries base_data_offset (flag 0x01) = absolute file offset of its moof;
             trun data_offset is relative to that.
  base_moof  tfhd has default-base-is-moof (0x020000); data_offset relative to the moof start.
  omit       neither flag: first traf's base = moof start; every later traf's base = END
             of the data of the preceding traf (so its first data_offset is 0 here).
Split (§8.8.8): every trun with >= 2 samples becomes two consecutive truns (HAND-SPLIT:
no tool we have emits >1 trun per traf).  `explicit` = the second trun carries its own
data_offset; `implicit` = it omits data_offset and continues after the previous trun's data.
The first trun keeps first_sample_flags; the second uses the tfhd default sample flags.
`mfra` is dropped (its moof offsets would be stale; it is optional).
Correctness is checked independently by frag_samples.py (spec-derived) + mp4decrypt/ffmpeg."""
import struct, sys

def boxes(b):
    o = 0
    while o < len(b):
        n = struct.unpack('>I', b[o:o + 4])[0]
        yield b[o + 4:o + 8], b[o + 8:o + n]
        o += n

def box(t, body): return struct.pack('>I', 8 + len(body)) + t + bytes(body)

def parse_tfhd(kb):
    fl = struct.unpack('>I', kb[:4])[0] & 0xffffff
    tid = kb[4:8]; o = 8; bdo = None
    if fl & 1: bdo = kb[o:o + 8]; o += 8
    return fl, tid, kb[o:]          # rest = sample_desc/default fields, order preserved

def build_tfhd(fl, tid, rest, bdo=None):
    return struct.pack('>I', fl) + tid + (bdo or b'') + rest

def parse_trun(kb):
    vf = struct.unpack('>I', kb[:4])[0]; f = vf & 0xffffff
    n = struct.unpack('>I', kb[4:8])[0]; o = 8; off = fsf = None
    if f & 1: off = struct.unpack('>i', kb[o:o + 4])[0]; o += 4
    if f & 4: fsf = struct.unpack('>I', kb[o:o + 4])[0]; o += 4
    per = 4 * bin(f & 0xf00).count('1')
    ents = [kb[o + i * per:o + (i + 1) * per] for i in range(n)]
    assert o + n * per == len(kb)
    return dict(ver=vf >> 24, flags=f, off=off, fsf=fsf, ents=ents)

def trun_body(t):
    f = t['flags']; b = struct.pack('>I', (t['ver'] << 24) | f) + struct.pack('>I', len(t['ents']))
    if f & 1: b += struct.pack('>i', t['off'] if t['off'] is not None else 0)
    if f & 4: b += struct.pack('>I', t['fsf'])
    return b + b''.join(t['ents'])

def trun_data_len(t):
    f = t['flags']; assert f & 0x200, 'need per-sample sizes'
    idx = bin(f & 0x100).count('1')
    return sum(struct.unpack('>I', e[4 * idx:4 * idx + 4])[0] for e in t['ents'])

def split(t, mode):
    if mode == 'none' or len(t['ents']) < 2: return [t]
    k = len(t['ents']) // 2; f = t['flags']
    a = dict(t, ents=t['ents'][:k])
    b = dict(t, ents=t['ents'][k:], fsf=None,
             flags=((f & ~5) | (1 if mode == 'explicit' else 0)), off=None)
    return [a, b]

def rewrite_moof(body, layout, mode, moof_abs):
    kids = list(boxes(body)); trafs = []
    # ---- parse
    old_pos = 8; parsed = []
    for kt, kb in kids:
        if kt == b'traf':
            tk = []; p = old_pos + 8
            for ct, cb in boxes(kb):
                tk.append((ct, cb, p)); p += 8 + len(cb)
            parsed.append(('traf', tk))
        else:
            parsed.append((kt, kb))
        old_pos += 8 + len(kb)
    # ---- rebuild children with dummy offsets (fixed width fields) to learn sizes
    new = []
    for kind, val in parsed:
        if kind != 'traf':
            new.append((kind, val)); continue
        tk = []; truns = []
        for ct, cb, p in val:
            if ct == b'tfhd':
                fl, tid, rest = parse_tfhd(cb)
                assert not fl & 1, 'input must not already carry base_data_offset'
                fl &= ~0x20000
                if layout == 'default': fl |= 1
                if layout == 'base_moof': fl |= 0x20000
                tk.append(['tfhd', (fl, tid, rest), p])
            elif ct == b'trun':
                for t in split(parse_trun(cb), mode):
                    tk.append(['trun', t, p]); truns.append(t)
            else:
                tk.append([ct, cb, p])
        new.append(('traf', tk))
    def sizeof(ch):
        if ch[0] == 'tfhd':
            fl, tid, rest = ch[1]; return 8 + len(build_tfhd(fl, tid, rest, b'\0' * 8 if fl & 1 else None))
        if ch[0] == 'trun': return 8 + len(trun_body(ch[1]))
        return 8 + len(ch[1])
    pos = 8; senc_new = {}
    for kind, val in new:
        if kind != 'traf': pos += 8 + len(val); continue
        tp = pos + 8
        for ch in val:
            if ch[0] == b'senc': senc_new[id(val)] = (tp, ch[2])
            tp += sizeof(ch)
        pos = tp
    moof_size = pos
    # ---- assign offsets
    data_pos = moof_size + 8; out = bytearray(); prev_end = None
    for kind, val in new:
        if kind != 'traf':
            out += box(kind, val); continue
        # base for this traf
        base = 0 if (layout != 'omit' or prev_end is None) else prev_end
        cbody = bytearray(); cur = data_pos; first = True
        sn = senc_new.get(id(val))
        for ch in val:
            if ch[0] == 'tfhd':
                fl, tid, rest = ch[1]
                cbody += box(b'tfhd', build_tfhd(fl, tid, rest, struct.pack('>Q', moof_abs) if fl & 1 else None))
            elif ch[0] == 'trun':
                t = dict(ch[1])
                if t['flags'] & 1:
                    t['off'] = cur - base
                assert first or True
                cur += trun_data_len(t); first = False
                cbody += box(b'trun', trun_body(t))
            elif ch[0] == b'saio':
                vf = struct.unpack('>I', ch[1][:4])[0]; ver = vf >> 24; f = vf & 0xffffff
                assert not f & 1 and struct.unpack('>I', ch[1][4:8])[0] == 1, 'one saio entry expected'
                w = 8 if ver else 4
                old = int.from_bytes(ch[1][8:8 + w], 'big'); delta = (sn[0] - sn[1]) if sn else 0
                cbody += box(b'saio', ch[1][:8] + (old + delta).to_bytes(w, 'big'))
            else:
                cbody += box(ch[0], ch[1])
        prev_end = cur; data_pos = cur
        out += box(b'traf', cbody)
    return bytes(out), moof_size

def main(src, dst, layout, mode):
    d = open(src, 'rb').read(); out = bytearray()
    for t, body in boxes(d):
        if t == b'mfra': continue
        if t == b'moof':
            nb, _ = rewrite_moof(body, layout, mode, len(out))
            out += box(b'moof', nb)
        else:
            out += box(t, body)
    open(dst, 'wb').write(out)

if __name__ == '__main__':
    a = sys.argv[1:]
    lay = a[a.index('--layout') + 1]; mode = a[a.index('--split') + 1] if '--split' in a else 'none'
    main(a[0], a[1], lay, mode)
