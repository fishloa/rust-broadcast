#!/usr/bin/env python3
"""Independent oracle for ETSI TS 103 752-1 V1.2.1 section 8.3.3 (compact SCTE 35).

Packs Tables 6 and 8 with a plain MSB-first bit packer written ONLY from the
spec tables (page images of V1.2.1 pp.27-28, "No. of Bits" column), sharing no
code with the scte35-splice crate. Prints Rust `const` byte arrays for
`scte35-splice/tests/dvb_ta_compact_vectors.rs`. Re-run to regenerate:

    python3 scte35-splice/tests/fixtures/dvb_ta/gen_compact_vectors.py

Table 6 (compact_time_signal), in order:
  encrypted_packet 1 | encryption_algorithm 6 | cw_index 8 | pts_time 33 |
  segmentation_event_id 32 | segmentation_duration 40 | segmentation_type_id 8 |
  segmentation_upid_length (N) 8 | segmentation_upid N*8 | segments_num 8 |
  segments_expected 8 | if encrypted_packet: E_CRC_32 32
Table 8 (compact_splice_insert), in order:
  encrypted_packet 1 | encryption_algorithm 6 | cw_index 8 | pts_time 33 |
  splice_event_id 32 | duration 33 | unique_program_id 16 | avail_num 8 |
  avails_expected 8 | DAS_descriptor_flag 1 | reserved 6 |
  if DAS_descriptor_flag: descriptor_length (N) 8 | break_num 8 |
      breaks_expected 8 | equivalent_segmentation_type 4 | reserved 4 |
      upid() (N-3)*8
  if encrypted_packet: E_CRC_32 32
Table 5 prefixes message_type (8): 0x00 time_signal, 0x01 splice_insert.
"""


class Bits:
    def __init__(self):
        self.v = 0
        self.n = 0

    def put(self, value, width):
        assert 0 <= value < (1 << width), (value, width)
        self.v = (self.v << width) | value
        self.n += width

    def bytes(self):
        assert self.n % 8 == 0, self.n
        return self.v.to_bytes(self.n // 8, 'big')


def time_signal(enc, alg, cw, pts, evid, dur, typ, upid, num, exp, ecrc=0):
    b = Bits()
    b.put(0x00, 8)
    b.put(enc, 1)
    b.put(alg, 6)
    b.put(cw, 8)
    b.put(pts, 33)
    b.put(evid, 32)
    b.put(dur, 40)
    b.put(typ, 8)
    b.put(len(upid), 8)
    for c in upid:
        b.put(c, 8)
    b.put(num, 8)
    b.put(exp, 8)
    if enc:
        b.put(ecrc, 32)
    return b.bytes()


def splice_insert(enc, alg, cw, pts, evid, dur, upi, avail, avails, reserved, das=None, ecrc=0):
    b = Bits()
    b.put(0x01, 8)
    b.put(enc, 1)
    b.put(alg, 6)
    b.put(cw, 8)
    b.put(pts, 33)
    b.put(evid, 32)
    b.put(dur, 33)
    b.put(upi, 16)
    b.put(avail, 8)
    b.put(avails, 8)
    b.put(1 if das else 0, 1)
    b.put(reserved, 6)
    if das:
        brk, brks, eqt, dres, upid = das
        b.put(3 + len(upid), 8)
        b.put(brk, 8)
        b.put(brks, 8)
        b.put(eqt, 4)
        b.put(dres, 4)
        for c in upid:
            b.put(c, 8)
    if enc:
        b.put(ecrc, 32)
    return b.bytes()


UPID = b'urn:com.broadcaster:112210F47DE98115'
VECTORS = {
    'TS_PLAIN': time_signal(0, 0, 0, 0x1_2345_6789, 0xDEAD_BEEF, 0xAB_CDEF_0123, 0x30, UPID, 1, 2),
    'TS_ENCRYPTED': time_signal(1, 5, 0x7E, 0x1_FFFF_FFFF, 0x0000_0001, 0x01_0000_0000, 0x34, b'', 0xFF, 0x00, 0xCAFE_BABE),
    'SI_PLAIN': splice_insert(0, 0, 0, 0x0_0000_1234, 0x1122_3344, 0x1_0000_0001, 0xBEEF, 7, 9, 0x3F),
    'SI_RESERVED_NONDEFAULT': splice_insert(0, 0, 0, 0x1_0000_0000, 1, 2, 3, 4, 5, 0x2A),
    'SI_WITH_DAS': splice_insert(0, 0, 0, 0x1_2345_6789, 0xDEAD_BEEF, 0x1_5180_0000 & ((1 << 33) - 1), 0x0001, 1, 3, 0x3F, das=(1, 3, 2, 0xF, UPID)),
    'SI_DAS_RESERVED_NONDEFAULT': splice_insert(0, 0, 0, 5, 6, 7, 8, 9, 10, 0x3F, das=(2, 4, 1, 0x5, b'Q')),
    'SI_ENCRYPTED_DAS': splice_insert(1, 63, 0xFF, 0x1_FFFF_FFFF, 0xFFFF_FFFF, 0x1_FFFF_FFFF, 0xFFFF, 255, 255, 0x3F, das=(0, 0, 0, 0, b''), ecrc=0x0102_0304),
}

if __name__ == '__main__':
    for name, data in VECTORS.items():
        print('const %s: [u8; %d] = [' % (name, len(data)))
        for i in range(0, len(data), 12):
            print('    ' + ', '.join('0x%02X' % x for x in data[i:i + 12]) + ',')
        print('];')
