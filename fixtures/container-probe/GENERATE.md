# Regenerating the `container-probe` fixtures

Requires `ffmpeg` (built with `libx264`, `libmp3lame`, `libopus`) and `python3`.
Generated with ffmpeg 8.1.2. Run from the repository root.

Licence and per-file rationale: [`PROVENANCE.md`](PROVENANCE.md).

## Real muxer/encoder output

```bash
OUT=fixtures/container-probe

# M2TS / BDAV — 192-byte stride, 4-byte TP_extra_header prefix per packet.
# The headline case today's detect_container fails.
ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000" \
  -frames:v 25 -c:v libx264 -profile:v main -pix_fmt yuv420p \
  -c:a aac -shortest -f mpegts -mpegts_m2ts_mode 1 "$OUT/m2ts_192.m2ts"

# RIFF/WAVE
ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 1 \
  -c:a pcm_s16le -f wav "$OUT/pcm_s16le.wav"

# Ogg (Opus payload — libvorbis absent from this build; same OggS magic)
ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 1 \
  -c:a libopus -f ogg "$OUT/opus.ogg"

# ASF
ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v msmpeg4v3 -f asf "$OUT/video.asf"

# Elementary streams
ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 1 \
  -c:a aac -f adts "$OUT/aac.adts"

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000" -t 1 \
  -c:a libmp3lame -f mp3 "$OUT/audio.mp3"

ffmpeg -y -hide_banner -loglevel error \
  -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v libx264 -profile:v main -pix_fmt yuv420p \
  -bsf:v h264_mp4toannexb -f h264 "$OUT/h264.annexb"
```

## Derived — mid-packet TS (real data, shifted start offset)

Removes the first 77 bytes of a real capture, producing a file that begins
mid-packet. Every byte is real captured data; only the start offset differs.
Detects as stride 188, phase 111 (= 188 − 77).

```bash
python3 -c "
d = open('fixtures/ts/h264_aac.ts', 'rb').read()
open('fixtures/container-probe/ts_midpacket_phase.ts', 'wb').write(d[77:])
"
```

## Synthetic — 204-byte stride

**Read `PROVENANCE.md`'s entry for this file before relying on it.** The TS
packets are real; the 16 appended parity bytes per packet are zeroed, not
genuine Reed–Solomon parity. Valid only because a probe detects the stride and
never reads the parity.

```bash
python3 -c "
d = open('fixtures/ts/h264_aac.ts', 'rb').read()
out = bytearray()
for i in range(len(d) // 188):
    out += d[i*188:(i+1)*188]
    out += b'\x00' * 16          # RS(204,188) parity position — ZEROED
open('fixtures/container-probe/ts_204_stride_SYNTHETIC.ts', 'wb').write(bytes(out))
"
```

## Verifying the lattice of any TS-family fixture

The check used to confirm each stride/phase claim in `PROVENANCE.md`:

```bash
python3 -c "
import sys
d = open(sys.argv[1], 'rb').read()
for stride in (188, 192, 204, 208):
    for phase in range(stride):
        n = 0
        while phase + n*stride < len(d) and d[phase + n*stride] == 0x47:
            n += 1
        if n >= 8:
            print(f'stride={stride} phase={phase} syncs={n}')
            break
" <file>
```
