#!/usr/bin/env bash
# Independent-tool proof for the CENC fragment layouts. Run after gen.sh. Writes proof.log.
# Requires ffmpeg 8.1.x, Bento4 mp4decrypt, python3.
set -uo pipefail
cd "$(dirname "$0")"
KEY=00112233445566778899aabbccddeeff
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fm() { ffmpeg -v error -y "$@" 2>/dev/null; }
fm -i clear.mp4 -c copy -f framemd5 $T/clear.md5
cmpfm() { python3 - "$1" "$2" <<'PY'
import sys
def h(p): return sorted((l.split(',')[0].strip(), l.split(',')[4].strip(), l.split(',')[5].strip()) for l in open(p) if not l.startswith('#'))
a, b = h(sys.argv[1]), h(sys.argv[2]); print(f"{sum(x == y for x, y in zip(a, b))}/{len(a)} frames")
PY
}
want=$(python3 frag_samples.py clear.mp4 | tr '\n' ';')
{
echo "# $(ffmpeg -version | head -1); Bento4 mp4decrypt: $(mp4decrypt 2>&1 | sed -n 2p)"
echo "# spec-derived plaintext of clear.mp4: $want"
echo "# (A) ffmpeg reads the CLEAR ffmpeg-written forms + our re-addressed clear twins (framemd5 vs clear.mp4):"
for f in clear_ffmpeg_default clear_ffmpeg_base_moof clear_ffmpeg_omit clear_multitrun_base_moof_implicit clear_multitrun_omit_implicit; do
  fm -i $f.mp4 -c copy -f framemd5 $T/x.md5; echo "  $f: ffmpeg -c copy $(cmpfm $T/clear.md5 $T/x.md5); spec-extractor plaintext == clear: $([ "$(python3 frag_samples.py $f.mp4 | tr '\n' ';')" = "$want" ] && echo yes || echo NO)"
done
echo "# (B) encrypted files: ciphertext identical across layouts?  mp4decrypt plaintext == clear?  ffmpeg -decryption_key frames == clear?"
echo "# file                         | ciphertext==enc_base_moof_none | mp4decrypt (Bento4) | ffmpeg decrypt"
ref=$(python3 frag_samples.py enc_base_moof_none.mp4 | tr '\n' ';')
for lay in default base_moof omit; do for sp in none explicit implicit; do
  f=enc_${lay}_$sp
  same=$([ "$(python3 frag_samples.py $f.mp4 | tr '\n' ';')" = "$ref" ] && echo yes || echo NO)
  mp4decrypt --key 1:$KEY --key 2:$KEY $f.mp4 $T/d.mp4 >/dev/null 2>&1
  bento=$([ "$(python3 frag_samples.py $T/d.mp4 2>/dev/null | tr '\n' ';')" = "$want" ] && echo "CORRECT" || echo "WRONG")
  fm -decryption_key $KEY -i $f.mp4 -c copy -f framemd5 $T/ff.md5
  printf '%-30s | %-30s | %-19s | %s\n' "$f.mp4" "$same" "$bento" "$(cmpfm $T/clear.md5 $T/ff.md5)"
done; done
} 2>&1 | tee proof.log
