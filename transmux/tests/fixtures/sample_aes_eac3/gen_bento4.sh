#!/usr/bin/env bash
# Independent ENCRYPTOR proof for E-AC-3: Bento4 mp4hls --encryption-mode SAMPLE-AES.
# Run AFTER gen.sh. Requires Bento4 (mp4hls 1.2.0 r641), ffmpeg 8.1.x, python3 + pycryptodome.
set -euo pipefail
cd "$(dirname "$0")"
KEYIV=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f   # fps mode = key||IV, deterministic
IV=101112131415161718191a1b1c1d1e1f
OUT=bento4; rm -rf $OUT; mkdir -p $OUT/ts $OUT/packed; T=$(mktemp -d)
ffmpeg -v error -y -f eac3 -i clear.ec3 -c copy $T/in.mp4
xxd -r -p <<< ${KEYIV:0:32} > $OUT/key.bin
{
mp4hls -f -o $T/ts     --audio-format ts     --segment-duration 2 --encryption-mode SAMPLE-AES --encryption-key $KEYIV --encryption-iv-mode fps --encryption-key-uri ../key.bin $T/in.mp4 >/dev/null 2>&1
mp4hls -f -o $T/packed --audio-format packed --segment-duration 2 --encryption-mode SAMPLE-AES --encryption-key $KEYIV --encryption-iv-mode fps --encryption-key-uri ../key.bin $T/in.mp4 >/dev/null 2>&1
cp $T/ts/media-1/segment-0.ts $OUT/ts/segment-0.ts
sed "s|URI=\"../key.bin\"|URI=\"../key.bin\",IV=0x$IV|" $T/ts/media-1/stream.m3u8 > $OUT/ts/stream.m3u8
cp $T/packed/media-1/segment-0.ec3 $OUT/packed/segment-0.ec3
python3 - $OUT <<'PY'
import sys
d = open(sys.argv[1] + '/packed/segment-0.ec3', 'rb').read()
n = 10 + ((d[6] << 21) | (d[7] << 14) | (d[8] << 7) | d[9])          # ID3v2 tag size (syncsafe)
open(sys.argv[1] + '/packed/bento4_encrypted_es.ec3', 'wb').write(d[n:])
PY
python3 ts_extract_es.py $OUT/ts/segment-0.ts $OUT/ts/bento4_encrypted_es.ec3 0x101   # Bento4: PMT 0x100, audio 0x101, stream_type 0xC2 (+apad setup descriptor)
for f in packed ts; do
  if cmp -s $OUT/$f/bento4_encrypted_es.ec3 enc_reset.ec3;   then echo "Bento4 $f ES == python reference (reset per syncframe): IDENTICAL ($(stat -f%z enc_reset.ec3 2>/dev/null || stat -c%s enc_reset.ec3) bytes)"; else echo "Bento4 $f ES != python reference (reset)"; fi
  if cmp -s $OUT/$f/bento4_encrypted_es.ec3 enc_carried.ec3; then echo "Bento4 $f ES == python reference (carried): IDENTICAL"; else echo "Bento4 $f ES != python reference (carried)"; fi
done
# ffmpeg (independent decryptor) decoding Bento4's TS
ffmpeg -v error -y -allowed_extensions ALL -i $OUT/ts/stream.m3u8 -f framemd5 $OUT/ts/bento4_decoded.framemd5 2>/dev/null || true
python3 cmp_pcm_frames.py clear.framemd5 $OUT/ts/bento4_decoded.framemd5 9
} 2>&1 | tee $OUT/proof.log
rm -rf $T
