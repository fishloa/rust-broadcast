#!/usr/bin/env bash
# Regenerates this directory (except bento4/, see gen_bento4.sh) + proof.log.
# Requires ffmpeg 8.1.x, python3 + pycryptodome.
set -euo pipefail
cd "$(dirname "$0")"
KEY=000102030405060708090a0b0c0d0e0f
IV=101112131415161718191a1b1c1d1e1f
P=9   # syncframes compared; the last ones are sacrificial (ffmpeg leaves the EOF tail undecrypted)

# 12 syncframes of E-AC-3, stereo 48 kHz, 90 kbit/s => 360-byte syncframes: 16 clear leader +
# 21 whole blocks + 8 trailing bytes (exercises the trailing-partial-block rule)
ffmpeg -v error -y -f lavfi -i "sine=frequency=440:duration=0.4:sample_rate=48000" -ac 2 -c:a eac3 -b:a 90k clear.ec3
printf '%s' "$KEY" | xxd -r -p > key.bin
python3 sample_aes_eac3_ref.py clear.ec3 enc_reset.ec3   $KEY $IV --reset-per-syncframe
python3 sample_aes_eac3_ref.py clear.ec3 enc_carried.ec3 $KEY $IV

# ffmpeg cannot MUX encrypted E-AC-3 (its muxer must decode), so mux the clear ES to TS and swap the
# ciphertext into the PES payload (same length). ffmpeg writes stream_type 0x87 + 'EAC3' registration.
ffmpeg -v error -y -f eac3 -i clear.ec3 -c copy -f mpegts clear.ts
python3 ts_swap_es.py clear.ts enc_reset.ec3   enc_reset.ts   0x100
python3 ts_swap_es.py clear.ts enc_carried.ec3 enc_carried.ts 0x100
for n in enc_reset enc_carried; do
  printf '#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI="key.bin",IV=0x%s\n#EXTINF:0.4,\n%s.ts\n#EXT-X-ENDLIST\n' "$IV" "$n" > $n.m3u8
done
ffmpeg -v error -y -i clear.ts -f framemd5 clear.framemd5 2>/dev/null
for n in enc_reset enc_carried; do ffmpeg -v error -y -allowed_extensions ALL -i $n.m3u8 -f framemd5 $n.framemd5 2>/dev/null || true; done
# control: the same encrypted TS WITHOUT the key tag does not decode
ffmpeg -v error -y -i enc_reset.ts -f framemd5 nokey.framemd5 2>/dev/null || true
{
echo "# ffmpeg: $(ffmpeg -version | head -1)"
echo "# syncframes in clear.ec3: $(ffprobe -v error -show_entries packet=size -of csv=p=0 clear.ec3 | wc -l) x 360 bytes; PES packets on the audio PID: 1 (so 'carried' is testable inside one PES)"
echo "# PROOF (first $P frames; ffmpeg does not decrypt the EOF tail):"
python3 cmp_pcm_frames.py clear.framemd5 enc_reset.framemd5   $P
python3 cmp_pcm_frames.py clear.framemd5 enc_carried.framemd5 $P
echo "# all frames:"
python3 cmp_pcm_frames.py clear.framemd5 enc_reset.framemd5
echo "# ES-level check: python reference vs ES extracted from the ffmpeg-tested TS"
python3 ts_extract_es.py enc_reset.ts /dev/stdout 0x100 | cmp - enc_reset.ec3 && echo "enc_reset.ts payload == enc_reset.ec3"
} 2>&1 | tee proof.log
