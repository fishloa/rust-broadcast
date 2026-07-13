# Codec configuration box syntax — transmux crate reference

Covers the codec-specific sample entry boxes that live inside `stsd`.

Source key:
- **[AOM-AV1]** — AOM AV1 ISOBMFF Binding specification  
  `https://aomediacodec.github.io/av1-isobmff/`  (retrieved 2026-06-29)
- **[QTFF]** — Apple QuickTime File Format specification  
  `https://developer.apple.com/tutorials/data/documentation/quicktime-file-format/<Slug>.md`
- **[MP4RA]** — MP4 Registration Authority box registry  
  `https://mp4ra.org/registered-types/boxes`

Codec-specific boxes for `avc1`/`avcC` (H.264), `hvc1`/`hvcC` (H.265/HEVC), and
`mp4a`/`esds` (AAC/MPEG-4 audio) are **DONE** — curated from real, vendored ISO/IEC
14496-15/-14/-3/-1 text (not FFmpeg-only), living in `specs/rules/` and
`transmux/docs/codec/` rather than this file. See the summary table at the bottom
for the exact doc pointers; this file keeps only the `av01`/`av1C` transcription
in full since those two were curated here originally.

---

## `av01` — AV1 Sample Entry

Source: [AOM-AV1] §2.2, §2.3

Container: `stsd`.  
Extends: `VisualSampleEntry` (see universal sample entry header in `init-boxes.md`).

```
class AV1SampleEntry extends VisualSampleEntry('av01') {
    AV1CodecConfigurationBox config;
}
```

The `av01` sample entry inherits all fields of `VisualSampleEntry`, which are defined in
ISO/IEC 14496-12 §12.  The fields shared with QuickTime's `VisualSampleEntry` include width,
height, horizontal/vertical resolution, compressorname, depth, and pre-defined fields.

Key constraints from [AOM-AV1]:
- `width` and `height` SHALL equal `max_frame_width_minus_1 + 1` and
  `max_frame_height_minus_1 + 1` from the AV1 Sequence Header OBU.
- `compressorname` (32-byte field): informative; recommended value is `"AOM Coding"`.
- Must contain exactly one `av1C` box.

---

## `av1C` — AV1 Codec Configuration Box

Source: [AOM-AV1] §2.3 (`AV1CodecConfigurationBox` + `AV1CodecConfigurationRecord`)

Container: `av01`.  
Type: basic box (no version/flags).

```
class AV1CodecConfigurationBox extends Box('av1C') {
    AV1CodecConfigurationRecord av1Config;
}

aligned(8) class AV1CodecConfigurationRecord {
    unsigned int(1)  marker = 1;
    unsigned int(7)  version = 1;
    unsigned int(3)  seq_profile;
    unsigned int(5)  seq_level_idx_0;
    unsigned int(1)  seq_tier_0;
    unsigned int(1)  high_bitdepth;
    unsigned int(1)  twelve_bit;
    unsigned int(1)  monochrome;
    unsigned int(1)  chroma_subsampling_x;
    unsigned int(1)  chroma_subsampling_y;
    unsigned int(2)  chroma_sample_position;
    unsigned int(3)  reserved = 0;
    unsigned int(1)  initial_presentation_delay_present;
    if (initial_presentation_delay_present) {
        unsigned int(4) initial_presentation_delay_minus_one;
    } else {
        unsigned int(4) reserved = 0;
    }
    unsigned int(8)  configOBUs[];
}
```

| Field                                  | Bits  | Description                                                                |
|----------------------------------------|-------|----------------------------------------------------------------------------|
| `marker`                               | 1     | Always `1` — distinguishes this byte from an AV1 OBU header byte.         |
| `version`                              | 7     | Always `1` for this version of the record.                                 |
| `seq_profile`                          | 3     | AV1 sequence profile. SHALL equal `seq_profile` from the Sequence Header OBU. |
| `seq_level_idx_0`                      | 5     | Level index for operating point 0. SHALL equal `seq_level_idx[0]` from the Sequence Header OBU. |
| `seq_tier_0`                           | 1     | Tier for operating point 0. SHALL equal `seq_tier[0]` from the Sequence Header OBU. |
| `high_bitdepth`                        | 1     | SHALL equal `high_bitdepth` from the Sequence Header OBU.                  |
| `twelve_bit`                           | 1     | SHALL equal `twelve_bit` from the Sequence Header OBU, or `0` if absent.  |
| `monochrome`                           | 1     | SHALL equal `mono_chrome` from the Sequence Header OBU.                    |
| `chroma_subsampling_x`                 | 1     | SHALL equal `subsampling_x` from the Sequence Header OBU.                  |
| `chroma_subsampling_y`                 | 1     | SHALL equal `subsampling_y` from the Sequence Header OBU.                  |
| `chroma_sample_position`               | 2     | SHALL equal `chroma_sample_position` from the Sequence Header OBU.         |
| `reserved`                             | 3     | SHALL be zero.                                                             |
| `initial_presentation_delay_present`   | 1     | If `1`, the next field is present.                                         |
| `initial_presentation_delay_minus_one` | 4     | Number of samples (minus one) that must be decoded before starting presentation of the first sample. Absent when `initial_presentation_delay_present == 0`. |
| `configOBUs`                           | var   | Zero or more OBUs in Low Overhead Bitstream Format (`obu_has_size_field = 1`). Contains at most one Sequence Header OBU (if present, must be first). |

**Byte layout of the first two bytes:**

```
Byte 0: [marker=1][version=1 (7 bits)] => 0x81
Byte 1: [seq_profile (3)][seq_level_idx_0 (5)]
Byte 2: [seq_tier_0][high_bitdepth][twelve_bit][monochrome]
         [chroma_subsampling_x][chroma_subsampling_y][chroma_sample_position (2)]
Byte 3: [reserved (3)][initial_presentation_delay_present][reserved/delay_minus_one (4)]
Bytes 4+: configOBUs (optional)
```

---

## `avc1`/`avcC` — AVC/H.264 Sample Entry + Decoder Configuration Record

**DONE.** Full field syntax curated from the real, vendored ISO/IEC 14496-15:2012/2017
text (`avc1` §5.4, `avcC`/`AVCDecoderConfigurationRecord` §5.3.3), cross-checked against
the FFmpeg `movenc.c`/`avc.c` reference writer and a byte-exact real-fixture round-trip:

- `specs/rules/nal-avcc-hvcc-rules.md` — curated syntax + semantics (vision-read scanned
  PDF, since the 14496-15 body is image-only).
- `transmux/docs/codec/avcC-hvcC-14496-15.md` — implementation-facing transcription,
  triple-grounded (ISO text + `movenc.c` + real oracle bytes).
- Implemented in `transmux/src/*avc*`/`sps.rs` (#374), value-verified against a real
  `ffmpeg`-muxed fixture (#443).

---

## `hvc1`/`hvcC` — HEVC/H.265 Sample Entry + Decoder Configuration Record

**DONE.** `hvc1`/`hvc2` (ISO/IEC 14496-15 §8, HEVC Video Stream Definition) and `hvcC`
(`HEVCDecoderConfigurationRecord`, §8.3.3) recovered via OCR from the scanned 14496-15:2017
4th edition (no free HEVC text-layer edition exists), verbatim field-by-field confirmed
against both the ISO text and the FFmpeg `movenc.c`/`hevc.c` reference writer, plus a real
byte-exact `hevc_frag.mp4`/`hevc_main.mp4` fixture oracle (#394, #456).

- `specs/rules/nal-avcc-hvcc-rules.md`, `transmux/docs/codec/avcC-hvcC-14496-15.md` (same
  files as `avcC`, above — both codecs share one curated doc).

---

## `mp4a`/`esds` — MPEG-4 Audio Sample Entry + Elementary Stream Descriptor

**DONE.** `mp4a` (ISO/IEC 14496-14:2003 §5.6.1, `MP4AudioSampleEntry`) and `esds`
(`ES_Descriptor`/`DecoderConfigDescriptor`, ISO/IEC 14496-1:2010) curated from the real
vendored text; the `AudioSpecificConfig` (ASC) carried inside `esds` — AOT table, sampling-
frequency/channel-configuration tables — curated from ISO/IEC 14496-3:2001 §1.5.1.1/§1.6.

- `specs/rules/mp4-esds-rules.md` — `esds`/`ES_Descriptor` syntax.
- `specs/rules/aac-asc-rules.md` — AAC `AudioSpecificConfig` syntax + tables.
- `transmux/docs/codec/esds-mp4a.md`, `transmux/docs/codec/es-descriptor-14496-1.md`,
  `transmux/docs/codec/heaac-asc.md` — implementation-facing transcriptions.

---

## Codec configuration summary

| Sample entry | Config box | Source                                    | Status |
|-------------|------------|--------------------------------------------|--------|
| `av01`      | `av1C`     | [AOM-AV1]                                   | DONE   |
| `avc1`      | `avcC`     | ISO/IEC 14496-15 (vendored, vision-read)    | DONE   |
| `hvc1`      | `hvcC`     | ISO/IEC 14496-15 (vendored, OCR-recovered)  | DONE   |
| `mp4a`      | `esds`     | ISO/IEC 14496-14/-3/-1 (vendored)           | DONE   |
