# `srat` / `AudioSampleEntryV1` fixtures (audit r05-W31)

Fixtures for the 16.16 sampling-rate field and its `srat` escape hatch
(ISO/IEC 14496-12:2015 §12.2.3, §12.2.3.1 and §12.2.3.2 as amended by
Amd 1:2017), plus QuickTime sound-description round-trip fixtures.

## What the numbers actually are

`AudioSampleEntry.samplerate` is 16.16 fixed point, so its integer part is the
top 16 bits and cannot exceed 65535 Hz. A higher rate **truncates**: the low 32
bits of `rate << 16` are kept and the integer part comes back as `rate & 0xFFFF
<< ...` shifted back, i.e. `rate mod 65536` scaled. Concretely, as measured on
these files:

| Rate    | `rate << 16` | integer part | true rate |
|---------|--------------|--------------|-----------|
| 192000  | `0xEE000000` | 60928        | 192000    |
| 96000   | `0x77000000` | 30464        | 96000     |
| 176400  | `0xB1100000` | 45328        | 176400    |
| 88200   | `0x58880000` | 22664        | 88200     |

## Real files (ffmpeg-written, oracle-checked)

All generated with ffmpeg/ffprobe 8.1 from a lavfi sine:

```sh
ffmpeg -hide_banner -f lavfi -i "sine=sample_rate=192000:duration=0.01" \
  -c:a pcm_s24le -f mp4 -y ipcm_v1_192k.mp4
ffmpeg -hide_banner -f lavfi -i "sine=sample_rate=44100:duration=0.02" \
  -c:a alac -f mov -y qt_alac_v1.mov
for r in 192000 96000 44100; do
  ffmpeg -hide_banner -f lavfi -i "sine=sample_rate=$r:duration=0.02" \
    -c:a flac -f mp4 -y flac_$r.mp4
done
```

- **`ipcm_v1_192k.mp4`** — ffmpeg's correct `AudioSampleEntryV1` form. Its entry
  is `ipcm` with `entry_version = 1`, the 16.16 field left at the *wrapped*
  `0xBB800000` (48000 — ffmpeg's own value; the spec's `1 << 16` placeholder is
  not what ffmpeg writes), and a `srat` child carrying the true `192000`. The
  `stsd` version is 1.
  - `ffprobe -show_entries stream=sample_rate` → **192000**
  - `MP4Box -info` → **"PCM Audio Sample Rate 192000"**
- **`flac_192000.mp4` / `flac_96000.mp4` / `flac_44100.mp4`** — real lossless
  sources. ffmpeg's FLAC path does *not* emit `srat`, so the crate reads the
  wrapped 48000/30464 out of them; the tests retarget the demuxed IR to the
  decoder's true rate and re-mux, which is what puts the `srat` path under test.
  ffprobe reports 192000 / 96000 / 44100 on the sources, and the decoded PCM
  md5 of the crate's own output matches the source's exactly (lossless).
- **`qt_alac_v1.mov`** — a QuickTime **version 1** sound description: `entry_version
  = 1` inside a `stsd` with version 0, `compression_ID = 0xfffe` (-2, QuickTime's
  VBR marker) and a non-zero 8-byte `revision_level`/`vendor` region.
  `MP4Box -info` reads it as ALAC 44100 Hz.

## Derived files (byte patches, script committed)

`gen.py` in this directory performs exactly two patches, from the real files
above, and nothing else:

```sh
python3 transmux/tests/fixtures/audio_srat/gen.py
```

- **`ipcm_v0_192k_wrapped.mp4`** — the same PCM written the *wrapping* way:
  `stsd` version 0, `entry_version` 0, `srat` removed, 16.16 field `0xEE000000`.
  Both files decode to identical PCM
  (`ffmpeg -f md5` → `cdefc380f04a3abb5b3c6575c332889a` for both on the
  0.01 s original), but this one reports **60928** in ffprobe/MP4Box — the
  defect the v1 form avoids.
- **`qt_alac_v2_synthetic.mov`** — a synthetic QuickTime v2 sound description:
  `qt_alac_v1.mov` with `entry_version` patched to 2 and the
  `revision_level`/`vendor` bytes set to non-zero values the crate used to
  discard. **No locally-installed tool writes a QuickTime v2 entry** (ffmpeg
  writes v0/v1), so this is hand-patched from the real v1 file rather than
  captured; the README says so rather than implying an oracle produced it. It
  exists to pin the "write `entry_version` and the reserved bytes back
  verbatim" round trip.

## Oracle evidence

| Tool | `ipcm_v1_192k.mp4` | `ipcm_v0_192k_wrapped.mp4` |
|---|---|---|
| `ffprobe -show_entries stream=sample_rate` | 192000 | 60928 |
| `MP4Box -info` | "PCM Audio Sample Rate 192000" | 60928 |
| `ffmpeg -f md5` (decoded PCM) | `cdefc380f04a3abb5b3c6575c332889a` | `cdefc380f04a3abb5b3c6575c332889a` |

Tool versions: ffmpeg/ffprobe 8.1, MP4Box 2.4.0 (GPAC). Bento4 `mp4dump` 1.6.0.0
was used during development to read the entry offsets; the numbers in this file
are the ffprobe/MP4Box values listed above.
