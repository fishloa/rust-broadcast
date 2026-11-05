#!/usr/bin/env python3
"""Pure-python E-AC-3 HLS Sample-AES reference encryptor (pycryptodome).

usage: sample_aes_eac3_ref.py IN.ec3 OUT.ec3 KEYHEX IVHEX [--reset-per-syncframe] [--leader N]

Syncframe boundaries come ONLY from the wire frmsiz field (ETSI TS 102 366
E.1.3.1.3: bytes 2..3, 0x0B77 sync, frmsiz = ((b2&7)<<8|b3), frame bytes
(frmsiz+1)*2).  Per syncframe:
  * the first --leader bytes (default 16) stay clear;
  * then every WHOLE 16-byte block is AES-128-CBC encrypted;
  * a trailing partial block (< 16 bytes) stays clear;
  * default: the CBC chain is CARRIED from syncframe to syncframe (IV is set once,
    at the start of the stream/PES); --reset-per-syncframe resets to IV each syncframe.
"""
import sys
from Crypto.Cipher import AES

def syncframes(d):
    off = 0
    while off < len(d):
        assert d[off:off + 2] == b'\x0b\x77', f"lost sync at {off}"
        n = (((d[off + 2] & 7) << 8 | d[off + 3]) + 1) * 2
        yield d[off:off + n]
        off += n

def encrypt(d, key, iv, reset=False, leader=16):
    out, chain = bytearray(), iv
    for f in syncframes(d):
        f = bytearray(f)
        body = f[leader:]
        nfull = len(body) // 16 * 16
        if nfull:
            ct = AES.new(key, AES.MODE_CBC, iv if reset else chain).encrypt(bytes(body[:nfull]))
            f[leader:leader + nfull] = ct
            chain = ct[-16:]
        out += f
    return bytes(out)

if __name__ == '__main__':
    a = [x for x in sys.argv[1:] if not x.startswith('--')]
    leader = 16
    if '--leader' in sys.argv:
        leader = int(sys.argv[sys.argv.index('--leader') + 1]); a.remove(str(leader))
    src, dst, key, iv = a[:4]
    open(dst, 'wb').write(encrypt(open(src, 'rb').read(), bytes.fromhex(key), bytes.fromhex(iv),
                                  '--reset-per-syncframe' in sys.argv, leader))
