#!/usr/bin/env bash
# Regenerates every file in this directory (except bento4_sample_aes/, see gen_bento4.sh)
# and writes proof.log.  Requires: ffmpeg 8.1.x, python3 + pycryptodome.
set -euo pipefail
cd "$(dirname "$0")"
KEY=000102030405060708090a0b0c0d0e0f
IV=101112131415161718191a1b1c1d1e1f
N=12        # frames; the LAST TWO are sacrificial (ffmpeg never decrypts them, see README)
P=$((N-2))  # frames that are compared

# 1. clear source: 320x180, 12 frames, 4 slices/frame => many NALs of mixed size
ffmpeg -v error -y -f lavfi -i "testsrc=size=320x180:rate=10" -frames:v $N \
  -c:v libx264 -preset medium -bf 0 -g 10 -x264-params "slices=4:scenecut=0:bframes=0:threads=1" \
  -bsf:v h264_mp4toannexb -f h264 raw.h264
python3 prep_clear.py raw.h264 clear.h264 tailprobe_clear.h264 escprobe_clear.h264 $KEY $IV epclear_clear.h264
rm raw.h264
printf '%s' "$KEY" | xxd -r -p > key.bin

mkts() { # mkts NAME  (ES NAME.h264 -> NAME.ts + NAME.m3u8 with SAMPLE-AES key tag)
  ffmpeg -v error -y -r 10 -f h264 -i "$1.h264" -c copy -f mpegts "$1.ts"
  printf '#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI="key.bin",IV=0x%s\n#EXTINF:1.2,\n%s.ts\n#EXT-X-ENDLIST\n' "$IV" "$1" > "$1.m3u8"
}
fmd5() { ffmpeg -v error -y "${@:2}" -f framemd5 "$1" 2>/dev/null; }

python3 sample_aes_ref.py clear.h264 enc_chained.h264 $KEY $IV
python3 sample_aes_ref.py clear.h264 enc_reset.h264   $KEY $IV --reset-per-block
python3 sample_aes_ref.py tailprobe_clear.h264 tailprobe_enc_gt.h264 $KEY $IV --tail gt
python3 sample_aes_ref.py tailprobe_clear.h264 tailprobe_enc_ge.h264 $KEY $IV --tail ge
python3 sample_aes_ref.py escprobe_clear.h264 escprobe_enc.h264 $KEY $IV
python3 sample_aes_ref.py epclear_clear.h264 epclear_enc.h264 $KEY $IV
for n in enc_chained enc_reset tailprobe_enc_gt tailprobe_enc_ge escprobe_enc epclear_enc; do mkts $n; done

fmd5 clear.framemd5 -r 10 -f h264 -i clear.h264
fmd5 tailprobe_clear.framemd5 -r 10 -f h264 -i tailprobe_clear.h264
fmd5 escprobe_clear.framemd5 -r 10 -f h264 -i escprobe_clear.h264
fmd5 epclear_clear.framemd5 -r 10 -f h264 -i epclear_clear.h264
fmd5 epclear_enc.framemd5 -allowed_extensions ALL -i epclear_enc.m3u8
for n in enc_chained enc_reset tailprobe_enc_gt tailprobe_enc_ge; do
  fmd5 $n.framemd5 -allowed_extensions ALL -i $n.m3u8
done

{
echo "# ffmpeg: $(ffmpeg -version | head -1)"
echo "# PROOF: ffmpeg HLS Sample-AES decrypt vs clear decode (first $P of $N frames; last 2 = ffmpeg EOF artifact)"
python3 cmp_framemd5.py clear.framemd5 enc_chained.framemd5 $P || true
python3 cmp_framemd5.py clear.framemd5 enc_reset.framemd5   $P || true
echo "# all $N frames (shows the tail artifact):"
python3 cmp_framemd5.py clear.framemd5 enc_chained.framemd5 || true
echo "# tail-rule probe (last candidate block has exactly 16/15/17 bytes left):"
python3 cmp_framemd5.py clear.framemd5 tailprobe_clear.framemd5 $P || true
python3 cmp_framemd5.py tailprobe_clear.framemd5 tailprobe_enc_gt.framemd5 $P || true
python3 cmp_framemd5.py tailprobe_clear.framemd5 tailprobe_enc_ge.framemd5 $P || true
echo "# EP bytes inside CLEAR regions (epclear): python reference (unescape->encrypt->re-escape) decoded by ffmpeg:"
python3 cmp_framemd5.py epclear_clear.framemd5 epclear_enc.framemd5 $P || true
echo "# emulation-prevention probe: ciphertext block starts 00 00 01 -> encryptor must insert 0x03"
python3 - <<'PY'
import sample_aes_ref as r
c = [n for n in r.split_nals(open('escprobe_clear.h264','rb').read()) if n[0]&31==5][0]
e = [n for n in r.split_nals(open('escprobe_enc.h264','rb').read()) if n[0]&31==5][0]
print(f"first IDR slice: clear escaped len {len(c)}, encrypted escaped len {len(e)}, ciphertext contains 00 00 03 01: {b'\x00\x00\x03\x01' in e[:60]}")
PY
ffmpeg -v error -y -allowed_extensions ALL -i escprobe_enc.m3u8 -c copy -f h264 escprobe_dec_by_ffmpeg.h264 2>/dev/null
python3 compare_es.py escprobe_clear.h264 escprobe_dec_by_ffmpeg.h264 $P || true
echo '# (ffmpeg quirk: after removing an emulation byte from ciphertext its NAL lengths are off by one; content equal modulo that:)'
python3 compare_es.py escprobe_clear.h264 escprobe_dec_by_ffmpeg.h264 $P --len-tolerant || true
ffmpeg -v error -y -allowed_extensions ALL -i enc_chained.m3u8 -c copy -f h264 enc_chained_dec_by_ffmpeg.h264 2>/dev/null
python3 compare_es.py clear.h264 enc_chained_dec_by_ffmpeg.h264 $P || true
} | tee proof.log
