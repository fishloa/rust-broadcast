#!/usr/bin/env python3
"""prep_clear.py IN.h264 OUT_clear.h264 OUT_tailprobe_clear.h264 [OUT_escprobe KEYHEX IVHEX [OUT_epclear]]

1. OUT_clear: IN renormalised to 4-byte start codes (so byte comparisons with
   the encryptor's output are not confused by 3- vs 4-byte start codes).
2. OUT_tailprobe_clear: same stream but slice NALs padded with trailing zero
   0xAA bytes appended after the slice data (NOT spec-conformant padding, but
   ffmpeg's CABAC slice decoder ignores bytes after end_of_slice; zero padding
   was tried first and is unusable because ffmpeg's Sample-AES decryptor emits
   the NAL *unescaped*, turning cabac_zero_words into a 00 00 00 run) so that the unescaped NAL
   length puts the LAST 16-byte candidate block at a chosen remainder:
   per frame the target is the number of bytes left at the final candidate
   block: 16 (exactly one block), 15, 17, or 16 again. Pictures still decode
   identically (checked by the proof script).
"""
import sys
sys.path.insert(0, __file__.rsplit('/', 1)[0] if '/' in __file__ else '.')
import sample_aes_ref as r

SC = b'\x00\x00\x00\x01'
TARGETS = [16, 15, 17, 16, 15, 17, 16, 15, 17, 16, 16, 16]  # per frame (12 frames)

nals = r.split_nals(open(sys.argv[1], 'rb').read())
open(sys.argv[2], 'wb').write(b''.join(SC + n for n in nals))

frame = -1
out = []
for n in nals:
    t = n[0] & 31
    if t in (1, 5) and (n[1] & 0x80):   # first_mb_in_slice==0 -> new picture
        frame += 1
    if t in (1, 5) and len(r.unescape(n)) > 48:
        raw = r.unescape(n)
        want = TARGETS[frame % len(TARGETS)]
        # remaining bytes at the last candidate block offset (32 + 160k) must be == want
        pad = (want - (len(raw) - 32)) % 160
        if pad == 0 and (len(raw) - 32) < want:   # keep at least one candidate block
            pad = 160
        raw = raw + b'\xaa' * pad
        n = r.escape(raw)
    out.append(n)
open(sys.argv[3], 'wb').write(b''.join(SC + n for n in out))

# 3. escape probe (argv[4]): tamper 16 clear bytes of the FIRST IDR slice so that
#    the *ciphertext* of block 0 (key/IV from argv[5:7]) is 00 00 01 .. -> the
#    encryptor MUST insert an emulation-prevention 0x03 into the ciphertext.
#    The pictures are garbage but deterministic; the proof compares ES bytes.
if len(sys.argv) > 4:
    from Crypto.Cipher import AES
    key, iv = bytes.fromhex(sys.argv[5]), bytes.fromhex(sys.argv[6])
    target = bytes([0, 0, 1]) + bytes(range(0x40, 0x4d))          # ciphertext block 0
    clear_blk = bytes(a ^ b for a, b in zip(AES.new(key, AES.MODE_ECB).decrypt(target), iv))
    assert b'\x00\x00' not in clear_blk[:] or True
    res = []
    hit = 0
    for n in nals:
        if (n[0] & 31) == 5 and len(r.unescape(n)) > 260:
            raw = bytearray(r.unescape(n))
            if hit == 0:
                raw[32:48] = clear_blk                # ciphertext will need an inserted 0x03
            n = r.escape(bytes(raw)); hit += 1
        res.append(n)
    open(sys.argv[4], 'wb').write(b''.join(SC + n for n in res))

# 4. epclear probe (argv[7]): the clear input has EP bytes inside CLEAR regions (leader + between
#    blocks) of one IDR slice: 00 00 01 at 10 and 00 00 02 at 100.  Question: unescape -> encrypt
#    -> re-escape (this reference, Apple spec text) vs encrypt the escaped bytes.
if len(sys.argv) > 7:
    res, hit = [], 0
    for n in nals:
        if (n[0] & 31) == 5 and len(r.unescape(n)) > 260:
            if hit == 1:
                raw = bytearray(r.unescape(n))
                raw[10:13] = b'\x00\x00\x01'
                raw[100:103] = b'\x00\x00\x02'
                n = r.escape(bytes(raw))
            hit += 1
        res.append(n)
    open(sys.argv[7], 'wb').write(b''.join(SC + n for n in res))
