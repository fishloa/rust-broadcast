#!/usr/bin/env bash
# Independent ENCRYPTOR proof: Bento4 mp4hls --encryption-mode SAMPLE-AES.
# Run AFTER gen.sh.  Requires ffmpeg 8.1.x, Bento4 (mp4hls 1.2.0 r641), python3+pycryptodome.
# Bento4's `fps` IV mode takes key||IV (32 bytes) and is deterministic, but it omits IV=
# from the playlist, so we patch the IV into the playlist for ffmpeg.
set -euo pipefail
cd "$(dirname "$0")"
KEYIV=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
IV=101112131415161718191a1b1c1d1e1f
OUT=bento4_sample_aes
rm -rf $OUT/*; mkdir -p $OUT
xxd -r -p <<< ${KEYIV:0:32} > $OUT/key.bin
{
for name in clear tailprobe_clear escprobe_clear epclear_clear; do
  d=$OUT/$name; mkdir -p $d
  ffmpeg -v error -y -r 10 -f h264 -i $name.h264 -c copy $d/in.mp4
  mp4hls -f -o $d/hls --segment-duration 2 --encryption-mode SAMPLE-AES \
    --encryption-key $KEYIV --encryption-iv-mode fps --encryption-key-uri ../key.bin $d/in.mp4 >/dev/null 2>&1
  [ -f $name.framemd5 ] || ffmpeg -v error -y -r 10 -f h264 -i $name.h264 -f framemd5 $name.framemd5 2>/dev/null
  cp $d/hls/media-1/segment-0.ts $d/segment-0.ts
  sed "s|URI=\"../key.bin\"|URI=\"../key.bin\",IV=0x$IV|" $d/hls/media-1/stream.m3u8 > $d/stream.m3u8
  # H.264 ES of the encrypted TS exactly as Bento4 wrote it
  ffmpeg -v error -y -i $d/segment-0.ts -c copy -f h264 $d/bento4_encrypted.h264
  # ffmpeg (independent decryptor) decodes it
  ffmpeg -v error -y -allowed_extensions ALL -i $d/stream.m3u8 -f framemd5 $d/bento4_decoded.framemd5 2>/dev/null
  rm -rf $d/hls $d/in.mp4
  echo "## $name.h264 -> Bento4 SAMPLE-AES"
  python3 cmp_framemd5.py $name.framemd5 $d/bento4_decoded.framemd5 10 || true
  case $name in
    clear)           ref=enc_chained.h264 ;;
    tailprobe_clear) ref=tailprobe_enc_gt.h264 ;;
    escprobe_clear)  ref=escprobe_enc.h264 ;;
    epclear_clear)   ref=epclear_enc.h264 ;;
  esac
  echo "# Bento4 encrypted ES vs python reference $ref (first 10 pictures):"
  python3 compare_es.py $ref $d/bento4_encrypted.h264 10 --raw || true
  python3 compare_es.py $ref $d/bento4_encrypted.h264 10 || true
done
} 2>&1 | tee $OUT/proof.log
