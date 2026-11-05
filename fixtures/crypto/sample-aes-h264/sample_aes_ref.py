#!/usr/bin/env python3
"""Pure-python HLS Sample-AES H.264 reference encryptor (pycryptodome).

usage: sample_aes_ref.py IN.h264 OUT.h264 KEYHEX IVHEX [--reset-per-block] [--tail ge|gt]

Rule (Apple "MPEG-2 Stream Encryption Format for HLS", Sample-AES video):
  * NAL types 1 and 5 with more than 48 bytes are protected; all else clear.
  * first 32 bytes of the (unescaped) NAL stay clear (header byte + 31);
  * then 16 bytes encrypted out of every 160 (16 enc, 144 clear);
  * default rule: AES-128-CBC CHAINS across the encrypted blocks of one NAL
    (IV set only at the start of each NAL);
  * --reset-per-block: WRONG variant, IV reset for every 16-byte block;
  * --tail gt (default): a block is encrypted only if MORE than 16 bytes remain
    (an exactly-16-byte tail stays clear); --tail ge: encrypt if >= 16 remain.
Emulation prevention: NAL is unescaped first, encrypted, then 0x03 re-inserted
where 00 00 0[0-3] appears in the ciphertext. Output uses 4-byte start codes.
"""
import re, sys
from Crypto.Cipher import AES

def split_nals(d):
    idx = [m.start() for m in re.finditer(b'\x00\x00\x01', d)]
    out = []
    for i, p in enumerate(idx):
        e = idx[i + 1] if i + 1 < len(idx) else len(d)
        n = d[p + 3:e]
        if i + 1 < len(idx):
            n = n.rstrip(b'\x00') if False else n
            while n.endswith(b'\x00'):  # zero_byte of next 4-byte start code
                n = n[:-1]
        out.append(n)
    return out

def unescape(n):
    return re.sub(b'\x00\x00\x03', b'\x00\x00', n)  # non-overlapping; enough for encoder output

def escape(n):
    out = bytearray(); z = 0
    for b in n:
        if z >= 2 and b <= 3:
            out.append(3); z = 0
        out.append(b)
        z = z + 1 if b == 0 else 0
    if out and out[-1] == 0:  # RBSP may not end in 0x00: append cabac_zero_word guard 03
        out.append(3)
    return bytes(out)

def encrypt_nal(nal, key, iv, reset=False, tail='gt'):
    raw = bytearray(unescape(nal))
    if (raw[0] & 31) not in (1, 5) or len(raw) <= 48:
        return bytes(nal)
    chain = iv
    off = 32
    while off < len(raw):
        rem = len(raw) - off
        if rem > 16 or (tail == 'ge' and rem == 16):
            ct = AES.new(key, AES.MODE_CBC, iv if reset else chain).encrypt(bytes(raw[off:off + 16]))
            raw[off:off + 16] = ct
            chain = ct
        off += 160
    return escape(bytes(raw))

def main():
    a = [x for x in sys.argv[1:] if not x.startswith('--')]
    reset = '--reset-per-block' in sys.argv
    tail = 'gt'
    if '--tail' in sys.argv:
        tail = sys.argv[sys.argv.index('--tail') + 1]
        a.remove(tail)
    src, dst, key, iv = a[:4]
    key, iv = bytes.fromhex(key), bytes.fromhex(iv)
    with open(dst, 'wb') as f:
        for n in split_nals(open(src, 'rb').read()):
            f.write(b'\x00\x00\x00\x01' + encrypt_nal(n, key, iv, reset, tail))

if __name__ == '__main__':
    main()
