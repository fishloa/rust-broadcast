#!/bin/sh
# Local-only E-AC-3 dependent-substream artifacts for transmux's
# `eac3_real_dependent_substreams` test. NOTHING here is committed (see
# PROVENANCE.md): this script fetches the stream and derives everything else with
# the real tools into the git-ignored `.test-streams/eac3-dep/`.
# Run from anywhere; needs curl, ffmpeg, MP4Box and gpac.
set -e
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
D="$ROOT/.test-streams/eac3-dep"
mkdir -p "$D"
cd "$D"
clean() { sed 's/\x1b\[[0-9;]*m//g'; }

# The real stream: Dolby Digital Plus 7.1 (AC-3 core + dependent E-AC-3 substream).
[ -f the_great_wall_7.1.eac3 ] || curl -fsS -O https://fate-suite.ffmpeg.org/eac3/the_great_wall_7.1.eac3

# MP4 from two real producers (`ec-3` sample entry with a `dec3` box).
ffmpeg -y -loglevel error -i the_great_wall_7.1.eac3 -c copy -f mp4 gw_ffmpeg.mp4
MP4Box -add the_great_wall_7.1.eac3 -new gw_mp4box.mp4 >/dev/null 2>&1

# MPEG-2 TS from two real producers, laid out differently:
#  - ffmpeg: PMT stream_type 0x87 + registration "EAC3", one PES per access unit
#    (`-pes_payload_size` has no effect on this audio stream);
#  - gpac: stream_type 0x06 + DVB enhanced_AC-3 descriptor 0x7A + registration,
#    PES packets of uneven sizes (9376, 8824, 269, 4622, ... bytes) so access units
#    straddle PES boundaries.
ffmpeg -y -loglevel error -i the_great_wall_7.1.eac3 -c copy -f mpegts gw_ffmpeg.ts
gpac -i the_great_wall_7.1.eac3 -o gw_gpac.ts >/dev/null 2>&1

# Independent oracle: gpac's own parse of MP4Box's file, one line per sample.
gpac -i gw_mp4box.mp4 inspect:deep:full 2>&1 | clean | grep -E '^PID [0-9]+ PCK' | \
  sed -E 's/^PID [0-9]+ PCK ([0-9]+) - full frame dts ([0-9]+) cts ([0-9]+) dur ([0-9]+) sap ([0-9]+).* size ([0-9]+) .* CRC32 (0x[0-9A-Fa-f]+)$/\1 dts=\2 cts=\3 dur=\4 sap=\5 size=\6 crc32=\7/' > gw_mp4box.gpac_inspect.txt
ls -la
