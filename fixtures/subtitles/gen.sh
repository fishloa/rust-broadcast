#!/bin/sh
# Regenerates every artifact in this directory from the two authored inputs
# (subs.vtt, subs.ttml) with the real producers MP4Box and gpac, plus the
# independent oracles (gpac `inspect`, MP4Box `-raw`) the tests compare against.
# Tool versions are recorded in PROVENANCE.md. Run from this directory.
set -e
cd "$(dirname "$0")"
clean() { sed 's/\x1b\[[0-9;]*m//g'; }
rm -rf mp4box gpac oracle && mkdir -p mp4box/dash gpac/dash oracle

# ---- MP4Box ---------------------------------------------------------------
MP4Box -add subs.vtt:fmt=VTT:lang=en -new mp4box/wvtt_progressive.mp4 >/dev/null 2>&1
MP4Box -frag 4000 -out mp4box/wvtt_fragmented.mp4 mp4box/wvtt_progressive.mp4 >/dev/null 2>&1
MP4Box -dash 4000 -profile live -segment-name 'wvtt_$Number$' -out mp4box/dash/manifest.mpd subs.vtt:lang=en >/dev/null 2>&1
MP4Box -add subs.ttml:lang=en -new mp4box/stpp_progressive.mp4 >/dev/null 2>&1
MP4Box -frag 4000 -out mp4box/stpp_fragmented.mp4 mp4box/stpp_progressive.mp4 >/dev/null 2>&1

# ---- gpac filter graph ----------------------------------------------------
gpac -i subs.vtt -o gpac/wvtt_progressive.mp4 >/dev/null 2>&1
gpac -i subs.vtt -o gpac/wvtt_fragmented.mp4:frag:cdur=4 >/dev/null 2>&1
gpac -i subs.ttml -o gpac/stpp_progressive.mp4 >/dev/null 2>&1
gpac -i subs.vtt:#Language=en -o gpac/dash/manifest.mpd:segdur=4:profile=live:template='gp_$Number$' >/dev/null 2>&1

# ---- oracles: gpac's own parse of each file (one line per sample/packet) ---
for f in mp4box/wvtt_progressive.mp4 mp4box/wvtt_fragmented.mp4 mp4box/stpp_progressive.mp4 \
         mp4box/stpp_fragmented.mp4 gpac/wvtt_progressive.mp4 gpac/wvtt_fragmented.mp4 \
         gpac/stpp_progressive.mp4 mp4box/dash/manifest.mpd gpac/dash/manifest.mpd; do
  o=oracle/$(echo "$f" | tr '/' '_').gpac_inspect.txt
  gpac -i "$f" inspect:deep:full 2>&1 | clean | grep -E '^PID [0-9]+ PCK' | \
    sed -E 's/^PID [0-9]+ PCK ([0-9]+) - full frame dts ([0-9]+) cts ([0-9]+) dur ([0-9]+) sap ([0-9]+).* size ([0-9]+) .* CRC32 (0x[0-9A-Fa-f]+)$/\1 dts=\2 cts=\3 dur=\4 sap=\5 size=\6 crc32=\7/' > "$o"
done
# MP4Box's own WebVTT export of each wvtt track: the cue text as MP4Box reads it back
for f in mp4box/wvtt_progressive.mp4 mp4box/wvtt_fragmented.mp4 gpac/wvtt_progressive.mp4 gpac/wvtt_fragmented.mp4; do
  MP4Box -raw 1 "$f" -out "oracle/$(echo "$f" | tr '/' '_').mp4box_raw.vtt" >/dev/null 2>&1
done
ls -R . | head -60
