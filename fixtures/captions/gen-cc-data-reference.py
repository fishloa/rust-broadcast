"""Independent oracle: extract ATSC A/53 GA94 cc_data() from the H.264 video of a TS.

usage: ga94.py FILE.ts  -> prints `<pts_90k> <hex cc_data()>` per SEI in decode order.
Spec: ITU-T H.264 7.3.2.3 (SEI), Annex D.2.? user_data_registered_itu_t_t35 (payloadType 4),
ATSC A/53 Part 4 6.2.3 / ANSI/SCTE 128-1 (GA94, user_data_type_code 0x03).
"""
import sys

data = open(sys.argv[1], 'rb').read()
assert data[0] == 0x47
# PAT/PMT -> video pid
pids = {}


def sections(pid):
    buf = b''
    for i in range(0, len(data) - 187, 188):
        p = data[i:i + 188]
        if p[0] != 0x47:
            continue
        pp = ((p[1] & 0x1F) << 8) | p[2]
        if pp != pid:
            continue
        pusi = p[1] & 0x40
        afc = (p[3] >> 4) & 3
        off = 4
        if afc & 2:
            off += 1 + p[4]
        pl = p[off:]
        if pusi:
            pl = pl[1 + pl[0]:]
            buf = b''
        buf += pl
        yield buf


pat = next(sections(0))
prog_pid = ((pat[8 + 2] & 0x1F) << 8) | pat[8 + 3]
# find first program != 0
j = 8
while j + 4 <= 3 + ((pat[1] & 0xF) << 8 | pat[2]) - 4:
    pn = (pat[j] << 8) | pat[j + 1]
    pp = ((pat[j + 2] & 0x1F) << 8) | pat[j + 3]
    if pn != 0:
        prog_pid = pp
        break
    j += 4
pmt = next(sections(prog_pid))
slen = ((pmt[1] & 0xF) << 8) | pmt[2]
pil = ((pmt[10] & 0xF) << 8) | pmt[11]
k = 12 + pil
end = 3 + slen - 4
vpid = None
while k < end:
    st = pmt[k]
    ep = ((pmt[k + 1] & 0x1F) << 8) | pmt[k + 2]
    il = ((pmt[k + 3] & 0xF) << 8) | pmt[k + 4]
    if st == 0x1B and vpid is None:
        vpid = ep
    k += 5 + il
assert vpid is not None, 'no AVC video'

# collect PES for vpid
pes = []
cur = None
for i in range(0, len(data) - 187, 188):
    p = data[i:i + 188]
    pp = ((p[1] & 0x1F) << 8) | p[2]
    if pp != vpid:
        continue
    pusi = p[1] & 0x40
    afc = (p[3] >> 4) & 3
    off = 4
    if afc & 2:
        off += 1 + p[4]
    pl = p[off:] if afc & 1 else b''
    if pusi:
        if cur is not None:
            pes.append(cur)
        cur = bytearray()
    if cur is not None:
        cur += pl
if cur is not None:
    pes.append(cur)

out = []
for b in pes:
    b = bytes(b)
    assert b[:3] == b'\x00\x00\x01'
    flags = b[7]
    hl = b[8]
    pts = None
    if flags & 0x80:
        q = b[9:14]
        pts = ((q[0] >> 1) & 7) << 30 | (q[1] << 22) | ((q[2] >> 1) << 15) | (q[3] << 7) | (q[4] >> 1)
    es = b[9 + hl:]
    # annex B split
    i = 0
    nals = []
    starts = []
    while i + 3 <= len(es):
        if es[i:i + 3] == b'\x00\x00\x01':
            starts.append(i + 3)
            i += 3
        else:
            i += 1
    for n, s in enumerate(starts):
        e = (starts[n + 1] - 3) if n + 1 < len(starts) else len(es)
        nal = es[s:e]
        while nal.endswith(b'\x00'):
            nal = nal[:-1]
        nals.append(nal)
    for nal in nals:
        if not nal or (nal[0] & 0x1F) != 6:
            continue
        # unescape
        rb = bytearray()
        z = 0
        for c in nal[1:]:
            if z >= 2 and c == 3:
                z = 0
                continue
            rb.append(c)
            z = z + 1 if c == 0 else 0
        pos = 0
        while pos < len(rb) and rb[pos:pos + 1] != b'\x80':
            pt = 0
            while rb[pos] == 0xFF:
                pt += 255
                pos += 1
            pt += rb[pos]
            pos += 1
            ps = 0
            while rb[pos] == 0xFF:
                ps += 255
                pos += 1
            ps += rb[pos]
            pos += 1
            pay = bytes(rb[pos:pos + ps])
            pos += ps
            if pt == 4 and pay[:3] == b'\xb5\x00\x31' and pay[3:7] == b'GA94' and pay[7] == 0x03:
                out.append((pts, pay[8:]))
for pts, cc in out:
    print(pts, cc.hex())
