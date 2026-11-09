#!/usr/bin/env bash
# Regenerates every file here. Requires ffmpeg 8.1.x, Bento4 (mp4encrypt/mp4decrypt/mp4dump), python3.
set -euo pipefail
cd "$(dirname "$0")"
KEY=00112233445566778899aabbccddeeff
KID=0123456789abcdef0123456789abcdef
IV=0123456789abcdef            # mp4encrypt --key id:key:iv  (8-byte IV seed => deterministic output)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
BX="-fflags +bitexact -flags:v +bitexact -flags:a +bitexact -map_metadata -1"

# source: 320x180 10 fps H.264 (GOP 5 => 4 fragments) + 48 kHz stereo AAC => 2 trafs per moof
ffmpeg -v error -y -f lavfi -i "testsrc=size=320x180:rate=10:duration=2" -f lavfi -i "sine=frequency=440:duration=2:sample_rate=48000" \
  -ac 2 -c:v libx264 -preset medium -g 5 -bf 0 -x264-params "scenecut=0:threads=1" -c:a aac -b:a 64k -shortest $BX $T/src.mp4

# (A) the three addressing forms as ffmpeg itself writes them (CLEAR)
ffmpeg -v error -y -i $T/src.mp4 -c copy $BX -movflags frag_keyframe+empty_moov                        clear-ffmpeg-default.mp4
ffmpeg -v error -y -i $T/src.mp4 -c copy $BX -movflags frag_keyframe+empty_moov+default_base_moof      clear-ffmpeg-base-moof.mp4
ffmpeg -v error -y -i $T/src.mp4 -c copy $BX -movflags frag_keyframe+empty_moov+omit_tfhd_offset       clear-ffmpeg-omit.mp4
cp clear-ffmpeg-base-moof.mp4 clear.mp4
python3 frag_samples.py clear.mp4 --dump | grep -v '^track' | awk '{print $NF, $(NF-1)}' > /dev/null
python3 - > clear-samples.txt <<'PY'
import hashlib, frag_samples as fs
tr = fs.samples(open('clear.mp4', 'rb').read())
print('# track  sample_index  size  sha256   (per-sample plaintext hashes of clear.mp4; the decrypt oracle)')
for t in sorted(tr):
    for i, s in enumerate(tr[t]):
        print(t, i, len(s), hashlib.sha256(s).hexdigest())
PY

# (B) independent ENCRYPTOR: Bento4 mp4encrypt, CENC (aes-ctr, cenc scheme), both tracks, AVC NAL-aware subsamples.
#     (Bento4 always writes default-base-is-moof; mfra in its output is stale)
mp4encrypt --method MPEG-CENC --key 1:$KEY:$IV --key 2:$KEY:$IV --property 1:KID:$KID --property 2:KID:$KID clear.mp4 $T/enc-bento4.mp4
# (C) re-address the correctly-encrypted file into every 14496-12:2026 §8.8.7/§8.8.8 form (no sample byte changes)
for lay in default base_moof omit; do for sp in none explicit implicit; do
  python3 relayout.py $T/enc-bento4.mp4 enc-${lay//_/-}-${sp}.mp4 --layout $lay --split $sp
done; done
# hand-split CLEAR twin of one multi-trun file (for clear-side tests of the layouts)
python3 relayout.py clear.mp4 clear-multitrun-base-moof-implicit.mp4 --layout base_moof --split implicit
python3 relayout.py clear.mp4 clear-multitrun-omit-implicit.mp4      --layout omit      --split implicit

# layout evidence: the first moof of each form (tfhd flags: 0x01 base_data_offset present, 0x20000 default-base-is-moof)
{
for f in clear-ffmpeg-default clear-ffmpeg-base-moof clear-ffmpeg-omit enc-default-none enc-base-moof-none enc-omit-none enc-default-explicit enc-default-implicit; do
  echo "=== $f.mp4"
  mp4dump --verbosity 3 $f.mp4 | awk '/\[moof\]/{c++} c==1' \
    | grep -E '^\s*\[(moof|traf|tfhd|trun|saio|senc)\]|track ID|base data offset|data offset|sample count|first sample flags' | grep -v 'sample info count'
done
} > layouts.txt 2>&1
