# Codec configuration box syntax â transmux crate reference

Covers the codec-specific sample entry boxes that live inside .

Source key:
- **[AOM-AV1]** â AOM AV1 ISOBMFF Binding specification
    (retrieved 2026-06-29)
- **[QTFF]** â Apple QuickTime File Format specification
  
- **[MP4RA]** â MP4 Registration Authority box registry
  
- **[FFmpeg-movenc]** â FFmpeg 
  
- **[FFmpeg-avc]** â FFmpeg 
  
- **[FFmpeg-hevc]** â FFmpeg 
  
- **[ITU-H264]** â ITU-T H.264 (AVC):  (free)
- **[ITU-H265]** â ITU-T H.265 (HEVC):  (free)

---


## `av01` â AV1 Sample Entry

Source: [AOM-AV1] §§.2

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

## `av1C` â AV1 Codec Configuration Box

Source: [AOM-AV1] §.2.3 (`AV1CodecConfigurationBox` + `AV1CodecConfigurationRecord`)

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
| `marker`                               | 1     | Always `1` â distinguishes this byte from an AV1 OBU header byte.         |
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

## `avc1` â AVC/H.264 Visual Sample Entry

Sources: [FFmpeg-movenc] `mov_write_video_tag()` (line ~2815); structure per ISO/IEC 14496-15
§.5.4 (referenced); SPS/PPS fields per [ITU-H264] §.7.3 (Sequence Parameter Set).

Container: `stsd`.
Extends: `VisualSampleEntry` (see universal sample entry header in `init-boxes.md`).

The `avc1` sample entry carries the `avcC` box containing the AVCDecoderConfigurationRecord,
which embeds the SPS and PPS NAL units needed to decode the AVC (H.264) stream.

Fixed-length VisualSampleEntry header (86 bytes before codec-specific boxes):

| Offset | Size     | Field                          | Description                                                    |
|--------|----------|--------------------------------|----------------------------------------------------------------|
| 0      | 4        | box_size                       | Full box size (includes these 4 bytes).                        |
| 4      | 4        | box_type                       | `avc1` (fourCC).                                               |
| 8      | 4        | reserved                       | 0.                                                             |
| 12     | 2        | reserved                       | 0.                                                             |
| 14     | 2        | data_reference_index           | Always 1 for the first (only) DataEntry.                       |
| 16     | 2        | codec_stream_version           | 0.                                                             |
| 18     | 2        | codec_stream_revision          | 0.                                                             |
| 20     | 4        | vendor                         | Apple vendor string (`"FFMP"`) or 0 for MP4.                   |
| 24     | 4        | temporal_quality               | Reserved (0).                                                  |
| 28     | 4        | spatial_quality                | Reserved (0).                                                  |
| 32     | 2        | width                          | Visual width in pixels.                                        |
| 34     | 2        | height                         | Visual height in pixels.                                       |
| 36     | 4        | horizontal_resolution          | 72 dpi in fixed-point 16.16 (`0x00480000`).                    |
| 40     | 4        | vertical_resolution            | 72 dpi in fixed-point 16.16 (`0x00480000`).                    |
| 44     | 4        | data_size                      | 0.                                                             |
| 48     | 2        | frame_count                    | 1.                                                             |
| 50     | 1        | compressor_name_len            | Length of the `compressorname` string (0-31).                  |
| 51     | 31       | compressorname                 | Zero-padded display name (informative).                        |
| 82     | 2        | depth                          | `0x0018` (24-bit RGB) for video.                               |
| 84     | 2        | color_table_id                 | `0xFFFF` (no color table).                                     |
| 86+    | var      | codec-specific boxes           | `avcC`, optional `colr`, `pasp`, `clap`, etc.                 |

The `avc1` box MUST contain exactly one `avcC` box.

---

## `avcC` â AVC Decoder Configuration Record

Sources: [FFmpeg-movenc] `mov_write_avcc_tag()` (line ~1603); [FFmpeg-avc]
`ff_isom_write_avcc()` (line ~32); ISO/IEC 14496-15 §.5.3.3 (referenced);
SPS fields (profile_idc, level_idc, etc.) per [ITU-H264] §.7.3.2.1.

Container: `avc1` (or `avc2` etc.).
Type: basic box (no version/flags).

```
aligned(8) class AVCDecoderConfigurationRecord {
    unsigned int(8)  configurationVersion = 1;
    unsigned int(8)  AVCProfileIndication;
    unsigned int(8)  profile_compatibility;
    unsigned int(8)  AVCLevelIndication;
    bit(6) reserved = '111111'b;
    unsigned int(2)  lengthSizeMinusOne;
    bit(3) reserved = '111'b;
    unsigned int(5)  numOfSequenceParameterSets;
    for (i=0; i< numOfSequenceParameterSets; i++) {
        unsigned int(16)  sequenceParameterSetLength;
        bit(8*sequenceParameterSetLength) sequenceParameterSetNALUnit;
    }
    unsigned int(8)  numOfPictureParameterSets;
    for (i=0; i< numOfPictureParameterSets; i++) {
        unsigned int(16)  pictureParameterSetLength;
        bit(8*pictureParameterSetLength) pictureParameterSetNALUnit;
    }
    if (profile_idc == 100 || profile_idc == 110 || profile_idc == 122 ||
        profile_idc == 244 || profile_idc ==  44 || profile_idc ==  83 ||
        profile_idc ==  86 || profile_idc == 118 || profile_idc == 128 ||
        profile_idc == 138 || profile_idc == 139 || profile_idc == 134) {
        bit(6) reserved = '111111'b;
        unsigned int(2)  chroma_format_idc;
        bit(5) reserved = '11111'b;
        unsigned int(3)  bit_depth_luma_minus8;
        bit(5) reserved = '11111'b;
        unsigned int(3)  bit_depth_chroma_minus8;
        unsigned int(8)  numOfSequenceParameterSetExt;
        for (i=0; i< numOfSequenceParameterSetExt; i++) {
            unsigned int(16)  sequenceParameterSetExtLength;
            bit(8*sequenceParameterSetExtLength) sequenceParameterSetExtNALUnit;
        }
    }
}
```

Derived from `ff_isom_write_avcc()` ([FFmpeg-avc]:32-143). FFmpeg extracts SPS (nal_type 7) and
PPS (nal_type 8) from the input extradata by scanning for NAL unit start codes, then writes the
hvcC record. The SPS bytes at index 3 (profile), 4 (compatibility), and 5 (level) are copied
directly into the avcC header; these correspond to the `profile_idc`, `constraint_set*_flags`,
and `level_idc` fields of the SPS RBSP as defined in [ITU-H264] §.7.3.2.1.1.

### Field table

| Field                          | Size      | Description                                                   |
|--------------------------------|-----------|---------------------------------------------------------------|
| `configurationVersion`         | 1 byte    | Always `1`.                                                   |
| `AVCProfileIndication`         | 1 byte    | `profile_idc` from the first SPS (e.g. 66=Baseline, 77=Main, 100=High). |
| `profile_compatibility`        | 1 byte    | `constraint_set*_flags` byte from the SPS. Bit 0 = constraint_set0_flag (Baseline), bit 1 = constraint_set1_flag (Main), bit 2 = constraint_set2_flag (Extended), bits 3-5 = constraint_set3/4/5 flags, bits 6-7 reserved (0). |
| `AVCLevelIndication`           | 1 byte    | `level_idc` from the first SPS (e.g. 30=Level 3.0, 40=Level 4.0). |
| `reserved`                     | 6 bits    | `111111`b.                                                     |
| `lengthSizeMinusOne`           | 2 bits    | Length in bytes of the NALUnitLength field minus 1. Typically 3 (4-byte lengths). |
| `reserved`                     | 3 bits    | `111`b.                                                        |
| `numOfSequenceParameterSets`   | 5 bits    | Number of SPS NAL units. Must be >= 1.                         |
| `sequenceParameterSetLength`   | 2 bytes   | Length of each SPS NAL unit (bytes).                           |
| `sequenceParameterSetNALUnit`  | var       | Raw SPS NAL unit bytes (including nal_unit_type byte).         |
| `numOfPictureParameterSets`    | 1 byte    | Number of PPS NAL units. Must be >= 1.                         |
| `pictureParameterSetLength`    | 2 bytes   | Length of each PPS NAL unit (bytes).                           |
| `pictureParameterSetNALUnit`   | var       | Raw PPS NAL unit bytes (including nal_unit_type byte).         |

**Conditional High-profile extensions** (when profile_idc indicates a High profile variant):

| Field                          | Size      | Description                                                   |
|--------------------------------|-----------|---------------------------------------------------------------|
| `reserved`                     | 6 bits    | `111111`b.                                                     |
| `chroma_format_idc`            | 2 bits    | Chroma subsampling format (derived from SPS). 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4. |
| `reserved`                     | 5 bits    | `11111`b.                                                      |
| `bit_depth_luma_minus8`        | 3 bits    | Luma bit depth minus 8 (e.g. 0 = 8-bit, 2 = 10-bit).          |
| `reserved`                     | 5 bits    | `11111`b.                                                      |
| `bit_depth_chroma_minus8`      | 3 bits    | Chroma bit depth minus 8.                                      |
| `numOfSequenceParameterSetExt` | 1 byte    | Number of SPS extension NAL units (nal_type 13).               |
| `sequenceParameterSetExtLength`| 2 bytes   | Length of each SPS extension.                                  |
| `sequenceParameterSetExtNALUnit`| var     | SPS extension NAL unit data.                                   |

### Byte layout of the fixed avcC header

```
Byte 0:  configurationVersion                 = 0x01
Byte 1:  AVCProfileIndication                 (e.g. 0x40 = High, 0x4D = Main)
Byte 2:  profile_compatibility                (constraint flags byte)
Byte 3:  AVCLevelIndication                   (e.g. 0x1E = Level 3.0, 0x28 = Level 4.0)
Byte 4:  [reserved=111111][lengthSizeMinusOne] (e.g. 0xFF for 4-byte NAL lengths)
Byte 5:  [reserved=111][numOfSequenceParameterSets]
Bytes 6-7: SPS length (big-endian u16)
Bytes 8+: SPS NAL unit data
...       numOfPictureParameterSets (1 byte)
...       PPS length (2 bytes) + PPS NAL unit data
```

---

## `hvc1` â HEVC/H.265 Visual Sample Entry

Sources: [FFmpeg-movenc] `mov_write_video_tag()` (line ~2815); ISO/IEC 14496-15 §.8.2
(referenced); VPS/SPS/PPS fields per [ITU-H265] §.7.3 (Video Parameter Set, Sequence Parameter Set).

Container: `stsd`.
Extends: `VisualSampleEntry` (see universal sample entry header in `init-boxes.md`).

The `hvc1` sample entry carries the `hvcC` box containing the HEVCDecoderConfigurationRecord,
which embeds VPS, SPS, and PPS NAL units. The `hvc1` sample entry uses the same
VisualSampleEntry header as `avc1` (86 bytes). The `hvc2` variant differs only in the
`array_completeness` flag semantics.

As implemented in [FFmpeg-movenc] `mov_write_video_tag()`: the codec tag `hvc1` is mapped to
`ff_isom_write_hvcc()` with `ps_array_completeness=1` (all parameter sets present). The
sample entry box type is `hvc1` (fourCC), with fixed header fields identical to `avc1`.

The `hvc1` box MUST contain exactly one `hvcC` box.

---

## `hvcC` â HEVC Decoder Configuration Record

Sources: [FFmpeg-movenc] `mov_write_hvcc_tag()` (line ~1658); [FFmpeg-hevc]
`write_configuration_record()` + `hvcc_write()` (lines ~1290-1403); ISO/IEC 14496-15
§.8.3.3 (referenced); VPS/SPS/PPS fields per [ITU-H265] §.7.3.

Container: `hvc1` (or `hev1`).
Type: basic box (no version/flags).

```
aligned(8) class HEVCDecoderConfigurationRecord {
    unsigned int(8)  configurationVersion = 1;
    unsigned int(2)  general_profile_space;
    unsigned int(1)  general_tier_flag;
    unsigned int(5)  general_profile_idc;
    unsigned int(32) general_profile_compatibility_flags;
    unsigned int(48) general_constraint_indicator_flags;
    unsigned int(8)  general_level_idc;
    bit(4) reserved = '1111'b;
    unsigned int(12) min_spatial_segmentation_idc;
    bit(6) reserved = '111111'b;
    unsigned int(2)  parallelismType;
    bit(6) reserved = '111111'b;
    unsigned int(2)  chromaFormat;
    bit(5) reserved = '11111'b;
    unsigned int(3)  bitDepthLumaMinus8;
    bit(5) reserved = '11111'b;
    unsigned int(3)  bitDepthChromaMinus8;
    unsigned int(16) avgFrameRate;
    unsigned int(2)  constantFrameRate;
    unsigned int(3)  numTemporalLayers;
    unsigned int(1)  temporalIdNested;
    unsigned int(2)  lengthSizeMinusOne;
    unsigned int(8)  numOfArrays;
    for (j=0; j < numOfArrays; j++) {
        bit(1) array_completeness;
        unsigned int(1) reserved = 0;
        unsigned int(6) NAL_unit_type;
        unsigned int(16) numNalus;
        for (i=0; i < numNalus; i++) {
            unsigned int(16) nalUnitLength;
            bit(8*nalUnitLength) nalUnit;
        }
    }
}
```

Derived from `hvcc_write()` ([FFmpeg-hevc]:961-1077). The hvcC record carries a 31-byte fixed
header followed by NAL unit arrays (one each for VPS, SPS, PPS, and optionally SEI). The
profile/tier/level fields (`general_profile_space`, `general_tier_flag`, `general_profile_idc`,
`general_profile_compatibility_flags`, `general_constraint_indicator_flags`, `general_level_idc`)
are parsed from the VPS and SPS NAL units per [ITU-H265] §.7.3.2 (ProfileTierLevel syntax).

### Field table

| Field                               | Size      | Description                                                              |
|-------------------------------------|-----------|--------------------------------------------------------------------------|
| `configurationVersion`              | 1 byte    | Always `1`.                                                              |
| `general_profile_space`             | 2 bits    | Profile space (`0`=no profile space indicator).                          |
| `general_tier_flag`                 | 1 bit     | Tier (`0`=Main, `1`=High).                                              |
| `general_profile_idc`               | 5 bits    | Profile identifier (e.g. 1=Main, 2=Main 10).                            |
| `general_profile_compatibility_flags`| 4 bytes  | Compatibility flags: each bit indicates a profile the stream also conforms to. |
| `general_constraint_indicator_flags` | 6 bytes  | Constraint indicator flags (48 bits: bits 0-47 of general_constraint_indicator). |
| `general_level_idc`                 | 1 byte    | Level identifier (e.g. 120=Level 4, 150=Level 5, 180=Level 6).          |
| `reserved`                          | 4 bits    | `1111`b.                                                                 |
| `min_spatial_segmentation_idc`      | 12 bits   | Minimum spatial segmentation (`0`=unspecified). Max 4096.                |
| `reserved`                          | 6 bits    | `111111`b.                                                               |
| `parallelismType`                   | 2 bits    | `0`=mixed, `1`=slice-based, `2`=tile-based, `3`=wavefront-based.        |
| `reserved`                          | 6 bits    | `111111`b.                                                               |
| `chromaFormat`                      | 2 bits    | Chroma format (`0`=monochrome, `1`=4:2:0, `2`=4:2:2, `3`=4:4:4).       |
| `reserved`                          | 5 bits    | `11111`b.                                                                |
| `bitDepthLumaMinus8`                | 3 bits    | Luma bit depth minus 8 (e.g. `0`=8-bit, `2`=10-bit).                    |
| `reserved`                          | 5 bits    | `11111`b.                                                                |
| `bitDepthChromaMinus8`              | 3 bits    | Chroma bit depth minus 8.                                                |
| `avgFrameRate`                      | 2 bytes   | Average frame rate in 0.256 fps units (0 = unspecified).                 |
| `constantFrameRate`                 | 2 bits    | `0`=not constant, `1`=constant, `2`=CFR within each temporal layer.     |
| `numTemporalLayers`                 | 3 bits    | Number of temporal layers (`0`=unknown).                                 |
| `temporalIdNested`                  | 1 bit     | Whether sub-layer reference constraints apply.                           |
| `lengthSizeMinusOne`                | 2 bits    | NAL unit length field size minus 1. Typically `3` (4-byte lengths).      |
| `numOfArrays`                       | 1 byte    | Number of NAL unit type arrays (typically 3: VPS, SPS, PPS).             |

**Per-array entry:**

| Field                 | Size      | Description                                                   |
|-----------------------|-----------|---------------------------------------------------------------|
| `array_completeness`  | 1 bit     | `1` if all parameter sets are in this record (`hvc1`), `0` (`hev1` or unknown). |
| `reserved`            | 1 bit     | `0`b.                                                         |
| `NAL_unit_type`       | 6 bits    | HEVC NAL unit type for all NAL units in this array (e.g. 32=VPS, 33=SPS, 34=PPS). |
| `numNalus`            | 2 bytes   | Number of NAL units in the array.                             |
| `nalUnitLength`       | 2 bytes   | Length of each NAL unit entry (bytes).                        |
| `nalUnit`             | var       | Raw NAL unit bytes (including NAL header byte).               |

### Byte layout of the fixed hvcC header

```
Byte  0:  configurationVersion                     = 0x01
Byte  1:  [general_profile_space(2)][tier(1)][general_profile_idc(5)]
Bytes 2-5: general_profile_compatibility_flags      (4 bytes, big-endian)
Bytes 6-9: general_constraint_indicator_flags[0:31] (4 bytes, big-endian)
Bytes 10-11: general_constraint_indicator_flags[32:47] (2 bytes, big-endian)
Byte 12:  general_level_idc
Bytes 13-14: [reserved=1111][min_spatial_segmentation_idc]
Byte 15:  [reserved=111111][parallelismType]
Byte 16:  [reserved=111111][chromaFormat]
Byte 17:  [reserved=11111][bitDepthLumaMinus8]
Byte 18:  [reserved=11111][bitDepthChromaMinus8]
Bytes 19-20: avgFrameRate (u16, 0 = unspecified)
Byte 21:  [constantFrameRate][numTemporalLayers][temporalIdNested][lengthSizeMinusOne]
Byte 22:  numOfArrays
Bytes 23+: per-array entries (each: completeness+type(1), numNalus(2), NAL units)
```

---

## `mp4a` â MPEG-4 Audio Sample Entry

Sources: [FFmpeg-movenc] `mov_write_audio_tag()` (line ~1397); ISO/IEC 14496-14 §.5.6.1
(referenced); MPEG-4 Systems descriptors via [FFmpeg-movenc] `mov_write_esds_tag()` (line ~783).

Container: `stsd`.
Extends: `AudioSampleEntry` (see universal sample entry header in `init-boxes.md`).

The `mp4a` sample entry carries the `esds` box containing the ES_Descriptor structure,
which holds the AudioSpecificConfig needed to decode AAC/HE-AAC streams.

### AudioSampleEntry fixed header

As written by `mov_write_audio_tag()` ([FFmpeg-movenc]:1397-1578). For MP4 mode, version 0
is used unless the sample rate exceeds UINT16_MAX.

**Version 0 (standard MP4 audio):**

| Offset | Size | Field                    | Description                                                  |
|--------|------|--------------------------|--------------------------------------------------------------|
| 0      | 4    | box_size                 | Full box size (includes these 4 bytes).                      |
| 4      | 4    | box_type                 | `mp4a` (fourCC, stored big-endian in MP4 mode).             |
| 8      | 4    | reserved                 | 0.                                                           |
| 12     | 2    | reserved                 | 0.                                                           |
| 14     | 2    | data_reference_index     | Always 1.                                                    |
| 16     | 2    | version                  | `0` for Version 0.                                           |
| 18     | 2    | revision_level           | 0.                                                           |
| 20     | 4    | reserved                 | 0.                                                           |
| 24     | 2    | channelCount             | Number of audio channels (e.g. 2 for stereo).                |
| 26     | 2    | sampleSize               | Bits per sample (default 16).                                |
| 28     | 2    | compression_id           | 0 (for MP4).                                                 |
| 30     | 2    | packet_size              | 0 (constant).                                                |
| 32     | 2    | sample_rate              | Sample rate in fixed-point 16.16 (e.g. `0xBB80` = 48000 Hz). If > UINT16_MAX, stored as (rate >> 1) and `srat` box added. |
| 34     | 2    | reserved                 | 0.                                                           |
| 36+    | var  | codec-specific boxes     | `esds`, optional `btrt`, etc.                               |

**Total version 0 AudioSampleEntry: 36 bytes** before codec-specific boxes.

The `mp4a` box MUST contain exactly one `esds` box.

---

## `esds` â Elementary Stream Descriptor Box

Sources: [FFmpeg-movenc] `mov_write_esds_tag()` (line ~783) with `put_descr()` helper
(line ~712); MPEG-4 Systems (ISO/IEC 14496-1) descriptor structure (referenced).

Container: `mp4a` (or other MPEG-4 audio sample entries).
Type: full box (version 0, flags 0).

```
class ESDSBox extends FullBox('esds', 0, 0) {
    ES_Descriptor ESDescriptor;
}
```

The `esds` box contains an MPEG-4 Systems descriptor tree with the following structure:

```
ES_Descriptor (tag 0x03)
  +-- DecoderConfigDescriptor (tag 0x04)
  |     +-- DecoderSpecificInfo (tag 0x05)  [optional]
  +-- SLConfigDescriptor (tag 0x06)
```

The descriptor tree is flattened into a single byte sequence written using the `put_descr()`
function ([FFmpeg-movenc]:712-716), which writes each descriptor in expanded-length encoding:
1 byte tag, up to 4 bytes of length (each byte has high-bit continuation, last byte has
high-bit clear).

### ES_Descriptor

| Field               | Size      | Description                                                  |
|---------------------|-----------|--------------------------------------------------------------|
| tag                 | 1 byte    | `0x03` (ES_Descriptor tag).                                  |
| length (expanded)   | 1-4 bytes | Remaining descriptor length (expanded: high-bit continuation bytes + final byte). |
| ES_ID               | 2 bytes   | Elementary stream ID (set to `track_id` by FFmpeg).          |
| flags               | 1 byte    | `streamDependenceFlag`(1)=0, `URL_Flag`(1)=0, `OCRstreamFlag`(1)=0, `reserved`(5)=0. |

### DecoderConfigDescriptor

| Field                    | Size      | Description                                                      |
|--------------------------|-----------|------------------------------------------------------------------|
| tag                      | 1 byte    | `0x04` (DecoderConfigDescriptor tag).                            |
| length (expanded)        | 1-4 bytes | Remaining descriptor length.                                     |
| objectTypeIndication     | 1 byte    | MPEG-4 object type (e.g. `0x40`=AAC (14496-3), `0x6B`=MP3 (11172-3), `0x21`=H.264/AVC (14496-10), `0x20`=MPEG-4 Visual (14496-2)). |
| streamType               | 1 byte    | `0x15` for audio (`0x11` for video). Bits: `streamType(6)`=5 (audio), `upstream(1)`=0, `reserved(1)`=1. |
| bufferSizeDB             | 3 bytes   | Decoding buffer size in bytes (big-endian).                      |
| maxBitrate               | 4 bytes   | Maximum bitrate in bits/second.                                  |
| avgBitrate               | 4 bytes   | Average bitrate in bits/second.                                  |

### DecoderSpecificInfo (conditional)

Present only when the track has extradata (e.g. AAC AudioSpecificConfig):

| Field                    | Size      | Description                                                      |
|--------------------------|-----------|------------------------------------------------------------------|
| tag                      | 1 byte    | `0x05` (DecoderSpecificInfo tag).                                |
| length (expanded)        | 1-4 bytes | Length of the AudioSpecificConfig data.                          |
| AudioSpecificConfig      | var       | MPEG-4 AudioSpecificConfig (defined in ISO/IEC 14496-3). For AAC-LC, this is 2 bytes: `[AOT(5)][sampleRateIndex(4)][channelConfig(4)]` with potential extends. |

### SLConfigDescriptor

| Field                    | Size      | Description                                                      |
|--------------------------|-----------|------------------------------------------------------------------|
| tag                      | 1 byte    | `0x06` (SLConfigDescriptor tag).                                 |
| length (expanded)        | 1-4 bytes | `1` (constant).                                                  |
| predefined               | 1 byte    | `0x02` (SLConfigPredef = reserved for use in MP4).              |

### Expanded length encoding (`put_descr`)

From [FFmpeg-movenc] `put_descr()` (line ~712):

```c
void put_descr(AVIOContext *pb, int tag, unsigned int size) {
    int i = 3;
    avio_w8(pb, tag);
    for (; i > 0; i--)
        avio_w8(pb, (size >> (7 * i)) | 0x80);  // 3 bytes with high-bit set
    avio_w8(pb, size & 0x7F);                     // 1 byte with high-bit clear
}
```

This always writes 4 length bytes (the maximum), using the most-significant 3 bytes with
the high-bit (`0x80`) set as continuation markers and the least-significant byte with the
high-bit cleared.

### Byte layout example (AAC-LC 2ch 48kHz, typical esds box: ~51 bytes)

```
Byte  0:  box_size        (4 bytes, placeholder)
Byte  4:  box_type = 'esds' (4 bytes)
Byte  8:  version=0 flags=0 (4 bytes)
Byte 12:  tag = 0x03      (ES_Descriptor)
Bytes 13-16: length(expanded) of remaining ES_Descr (e.g. 0x80 0x80 0x80 0x1F = 31)
Byte 17:  ES_ID (high)
Byte 18:  ES_ID (low)
Byte 19:  flags = 0x00
Byte 20:  tag = 0x04      (DecoderConfigDescriptor)
Bytes 21-24: length(expanded) of DecoderConfigDescr (e.g. 0x80 0x80 0x80 0x15 = 21)
Byte 25:  objectTypeIndication = 0x40 (AAC)
Byte 26:  streamType = 0x15 (audio)
Bytes 27-29: bufferSizeDB
Bytes 30-33: maxBitrate
Bytes 34-37: avgBitrate
Byte 38:  tag = 0x05      (DecoderSpecificInfo)
Bytes 39-42: length(expanded) of ASC (e.g. 0x80 0x80 0x80 0x02 = 2)
Bytes 43-44: AudioSpecificConfig (e.g. 0x12 0x10 for AAC-LC 48kHz stereo)
Byte 45:  tag = 0x06      (SLConfigDescriptor)
Bytes 46-49: length(expanded) = 0x80 0x80 0x80 0x01 = 1
Byte 50:  SLConfig = 0x02
```

---

## Codec configuration summary

| Sample entry | Config box | Source                                            | Status |
|-------------|------------|---------------------------------------------------|--------|
| `av01`      | `av1C`     | [AOM-AV1]                                         | DONE   |
| `avc1`      | `avcC`     | [FFmpeg-movenc]+[FFmpeg-avc]+[ITU-H264]           | DONE   |
| `hvc1`      | `hvcC`     | [FFmpeg-movenc]+[FFmpeg-hevc]+[ITU-H265]           | DONE   |
| `mp4a`      | `esds`     | [FFmpeg-movenc]+ISO 14496-1 (ref)+ISO 14496-3 (ref)| DONE   |

All codec-config boxes are now documented from freely available reference implementations and
public specifications. Field tables derived from:

- **avcC**: `mov_write_avcc_tag()` / `ff_isom_write_avcc()` — byte-level extraction of SPS/PPS
  from Annex B extradata into the AVCDecoderConfigurationRecord structure.
- **hvcC**: `mov_write_hvcc_tag()` / `write_configuration_record()` / `hvcc_write()` — extraction
  of VPS/SPS/PPS from HEVC extradata into the HEVCDecoderConfigurationRecord.
- **esds**: `mov_write_esds_tag()` / `put_descr()` — MPEG-4 Systems descriptor tree with ES_Descriptor,
  DecoderConfigDescriptor, DecoderSpecificInfo, and SLConfigDescriptor.
- **avc1/hvc1/mp4a**: `mov_write_video_tag()` / `mov_write_audio_tag()` — the VisualSampleEntry
  and AudioSampleEntry fixed headers as written by FFmpeg.