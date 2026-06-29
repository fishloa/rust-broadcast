# Box coverage matrix — transmux crate

Lists every box required for TS→fMP4/CMAF/HLS muxing, its source, and transcription status.

| Box       | Full name                          | Source(s)             | Status            | File                    |
|-----------|------------------------------------|-----------------------|-------------------|-------------------------|
| `ftyp`    | File Type Box                      | [QTFF]                | DONE              | `init-boxes.md`         |
| `moov`    | Movie Container Box                | [QTFF], [W3C-MSE]     | DONE (container)  | `init-boxes.md`         |
| `mvhd`    | Movie Header Box                   | [QTFF]                | DONE              | `init-boxes.md`         |
| `trak`    | Track Container Box                | [QTFF]                | DONE (container)  | `init-boxes.md`         |
| `tkhd`    | Track Header Box                   | [QTFF]                | DONE              | `init-boxes.md`         |
| `mdia`    | Media Container Box                | [QTFF]                | DONE (container)  | `init-boxes.md`         |
| `mdhd`    | Media Header Box                   | [QTFF]                | DONE              | `init-boxes.md`         |
| `hdlr`    | Handler Reference Box              | [QTFF]                | DONE              | `init-boxes.md`         |
| `minf`    | Media Information Container Box    | [QTFF]                | DONE (container)  | `init-boxes.md`         |
| `vmhd`    | Video Media Information Header     | [QTFF]                | DONE              | `init-boxes.md`         |
| `smhd`    | Sound Media Information Header     | [QTFF]                | DONE              | `init-boxes.md`         |
| `dinf`    | Data Information Container Box     | [QTFF]                | DONE (container)  | `init-boxes.md`         |
| `dref`    | Data Reference Box                 | [QTFF]                | DONE              | `init-boxes.md`         |
| `url `    | Data Entry URL Box                 | [QTFF]                | DONE              | `init-boxes.md`         |
| `stbl`    | Sample Table Container Box         | [QTFF]                | DONE (container)  | `init-boxes.md`         |
| `stsd`    | Sample Description Box             | [QTFF]                | DONE              | `init-boxes.md`         |
| `stts`    | Decoding Time-to-Sample Box        | [QTFF]                | DONE              | `init-boxes.md`         |
| `stsc`    | Sample-to-Chunk Box                | [QTFF]                | DONE              | `init-boxes.md`         |
| `stsz`    | Sample Size Box                    | [QTFF]                | DONE              | `init-boxes.md`         |
| `stco`    | Chunk Offset Box (32-bit)          | [QTFF]                | DONE              | `init-boxes.md`         |
| `co64`    | Chunk Offset Box (64-bit)          | [QTFF]                | DONE              | `init-boxes.md`         |
| `mvex`    | Movie Extends Box                  | [W3C-MSE], [3GP-26244]| DONE (ISO 14496-12 §8.8.1, owner-directed reference) | `fragment-boxes.md` |
| `trex`    | Track Extends Box                  | [W3C-MSE]             | DONE (ISO 14496-12 §8.8.3, owner-directed reference) | `fragment-boxes.md` |
| `styp`    | Segment Type Box                   | [3GP-26244] §13.2     | DONE              | `fragment-boxes.md`     |
| `moof`    | Movie Fragment Box                 | [W3C-MSE], [3GP-26244]| DONE (ISO 14496-12 §8.8.4, owner-directed reference) | `fragment-boxes.md` |
| `mfhd`    | Movie Fragment Header Box          | [W3C-MSE]             | DONE (ISO 14496-12 §8.8.5, owner-directed reference) | `fragment-boxes.md` |
| `traf`    | Track Fragment Box                 | [W3C-MSE], [3GP-26244]| DONE (ISO 14496-12 §8.8.6, owner-directed reference) | `fragment-boxes.md` |
| `tfhd`    | Track Fragment Header Box          | [W3C-MSE]             | DONE (ISO 14496-12 §8.8.7, owner-directed reference) | `fragment-boxes.md` |
| `tfdt`    | Track Fragment Decode Time Box     | [3GP-26244] §13.5     | DONE              | `fragment-boxes.md`     |
| `trun`    | Track Fragment Run Box             | [W3C-MSE]             | DONE (ISO 14496-12 §8.8.8, owner-directed reference) | `fragment-boxes.md` |
| `mdat`    | Media Data Box                     | [QTFF], [W3C-MSE]     | DONE              | `fragment-boxes.md`     |
| `sidx`    | Segment Index Box                  | [3GP-26244] §13.4     | DONE              | `sidx.md`               |
| `av01`    | AV1 Sample Entry                   | [AOM-AV1] §2.2        | DONE              | `codec-config.md`       |
| `av1C`    | AV1 Codec Configuration Box        | [AOM-AV1] §2.3        | DONE              | `codec-config.md`       |
| `avc1`    | AVC/H.264 Sample Entry             | ISO/IEC 14496-15 (paid)| GAP — paid-only  | `codec-config.md`       |
| `avcC`    | AVC Decoder Configuration Record   | ISO/IEC 14496-15 (paid)| GAP — paid-only  | `codec-config.md`       |
| `hvc1`    | HEVC/H.265 Sample Entry            | ISO/IEC 14496-15 (paid)| GAP — paid-only  | `codec-config.md`       |
| `hvcC`    | HEVC Decoder Configuration Record  | ISO/IEC 14496-15 (paid)| GAP — paid-only  | `codec-config.md`       |
| `mp4a`    | MPEG-4 Audio Sample Entry          | ISO/IEC 14496-14 (paid)| GAP — paid-only  | `codec-config.md`       |
| `esds`    | Elementary Stream Descriptor Box   | ISO/IEC 14496-14/-1 (paid)| GAP — paid-only | `codec-config.md`      |

---

## Summary counts

| Status        | Count | Boxes                                                                   |
|---------------|-------|-------------------------------------------------------------------------|
| DONE          | 34    | `ftyp`, `moov`, `mvhd`, `trak`, `tkhd`, `mdia`, `mdhd`, `hdlr`, `minf`, `vmhd`, `smhd`, `dinf`, `dref`, `url `, `stbl`, `stsd`, `stts`, `stsc`, `stsz`, `stco`, `co64`, `styp`, `mvex`, `trex`, `moof`, `mfhd`, `traf`, `tfhd`, `tfdt`, `trun`, `mdat`, `sidx`, `av01`, `av1C` |
| GAP — paid-only | 6  | `avc1`, `avcC`, `hvc1`, `hvcC`, `mp4a`, `esds` |

**DONE = 34 boxes; GAP = 6 boxes** (remaining GAP entries are codec-config boxes from ISO/IEC 14496-14 and -15 — paid standards not consulted).

Note: All fragmentation-control boxes (`mvex`, `trex`, `moof`, `mfhd`, `traf`, `tfhd`, `trun`)
are now transcribed using an owner-directed reference to ISO/IEC 14496-12 §8.8 per SOURCES.md.
The remaining 6 GAP boxes (`avc1`, `avcC`, `hvc1`, `hvcC`, `mp4a`, `esds`) are codec-config
entries requiring ISO/IEC 14496-14/-15.

---

## Resolution paths for GAP entries

### Fragment control boxes (`mvex`, `trex`, `moof`, `mfhd`, `traf`, `tfhd`, `trun`) — NOW DONE

These seven boxes are transcribed in `fragment-boxes.md` using an owner-directed reference to
ISO/IEC 14496-12 §8.8.  Field tables derived from the verified transcription in §8.8.1–8.8.8.

### AVC/H.264 codec config (`avc1`, `avcC`)

Defined in ISO/IEC 14496-15.  The public H.264 spec (ITU-T H.264, free from ITU-T) describes
the SPS/PPS NAL unit syntax embedded in `avcC`.

### HEVC/H.265 codec config (`hvc1`, `hvcC`)

Defined in ISO/IEC 14496-15.  ITU-T H.265 (free) covers the VPS/SPS/PPS NAL syntax.

### AAC/MPEG-4 audio (`mp4a`, `esds`)

Defined in ISO/IEC 14496-14 and ISO/IEC 14496-1.  The `AudioSpecificConfig` bit syntax is
in ISO/IEC 14496-3 (MPEG-4 Audio, available via some national bodies at reduced cost, and
partially reproduced in public IETF RFCs for AAC).

---

## Validate-by-golden-byte / ffprobe strategy

For each DONE box, validation approach:

1. **Produce a reference fMP4:** `ffmpeg -i <fixture.ts> -c copy -movflags frag_keyframe+empty_moov+default_base_moof -f mp4 ref.mp4`
2. **Dump box tree:** `mp4dump ref.mp4` or `MP4Box -info ref.mp4` to confirm box presence, sizes, and field values.
3. **Parse PTS/DTS:** `ffprobe -show_packets -select_streams v:0 ref.mp4` to get per-frame timing; cross-validate against `tfdt.base_media_decode_time` and `sidx.earliest_presentation_time`.
4. **Byte-level verification:** `hexdump -C ref.mp4 | head -200` to confirm `ftyp`/`moov`/`moof` header bytes against field tables above.

For GAP boxes, the reference implementation's output (FFmpeg/GPAC) serves as the ground truth until the paid spec is obtained.
