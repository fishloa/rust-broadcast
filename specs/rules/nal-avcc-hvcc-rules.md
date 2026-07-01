# ISO/IEC 14496-15:2017 — AVC/HEVC config records (`avcC`/`hvcC`) + sample entries

Decoder-config records and sample entries a transmux lifts when carrying H.264/H.265 in MP4.
Source: `specs/fulltext/iso_iec_14496-15_avc_hevc_2017_excerpt.md` (vision-transcribed prose +
syntax of the relevant sections; cites by spec § + printed page). The PDF body is image-only —
exact bit-field values cross-checked against FFmpeg `movenc.c` where it matters.

> **#394 (partial):** a **text-layer** edition — **`specs/iso_iec_14496-15_2012_avc_format_TEXTLAYER.pdf`**
> (ISO/IEC 14496-15:2012, "AVC file format") — now grounds **`avcC`** (`AVCDecoderConfigurationRecord`,
> §5.2.4.1, text-searchable). It predates HEVC-in-MP4, so **`hvcC`** is NOT in it: hvcC stays grounded
> on the scanned 2017 ed + FFmpeg `movenc.c` + the ffmpeg-oracle byte-exact round-trip test (#443).
> No free text-layer 2014/2017 (HEVC) edition located.

## NAL sample format — §4.3.3 / §5.3.2

- A **sample = one access unit** (AVC: §5.3.1; HEVC: §8.3.1). Within a sample, each NAL unit is
  **length-prefixed** by `(lengthSizeMinusOne + 1)` bytes (`NALUnitLength`, big-endian); the length
  field counts the NAL header + payload, **not** itself (§5.4.3.2.3 p21). No Annex-B start codes.
- Parameter sets (VPS/SPS/PPS) live in the **config record (sample entry)** and/or in-band depending
  on the sample-entry name (see below).

## AVC decoder config record `avcC` — §5.3.3 (p17–19)

```
aligned(8) class AVCDecoderConfigurationRecord {
  unsigned int(8)  configurationVersion = 1;
  unsigned int(8)  AVCProfileIndication;        // profile_idc per 14496-10
  unsigned int(8)  profile_compatibility;       // the constraint-flags byte between profile_idc/level_idc in SPS
  unsigned int(8)  AVCLevelIndication;          // level_idc
  bit(6) reserved = '111111'b;
  unsigned int(2)  lengthSizeMinusOne;          // NAL length field width - 1
  bit(3) reserved = '111'b;
  unsigned int(5)  numOfSequenceParameterSets;
  for (..SPS..) { unsigned int(16) sequenceParameterSetLength; bit(8*len) sequenceParameterSetNALUnit; }
  unsigned int(8)  numOfPictureParameterSets;
  for (..PPS..) { unsigned int(16) pictureParameterSetLength; bit(8*len) pictureParameterSetNALUnit; }
  if (AVCProfileIndication==100||110||122||144) {       // high-profile ext
    bit(6) reserved='111111'b; unsigned int(2) chroma_format;          // chroma_format_idc
    bit(5) reserved='11111'b;  unsigned int(3) bit_depth_luma_minus8;
    bit(5) reserved='11111'b;  unsigned int(3) bit_depth_chroma_minus8;
    unsigned int(8) numOfSequenceParameterSetExt;
    for (..SPSExt..) { unsigned int(16) sequenceParameterSetExtLength; bit(8*len) sequenceParameterSetExtNALUnit; }
  }
}
```
- `lengthSizeMinusOne` ∈ **{0,1,3}** → 1/2/4-byte NAL length (value 2 not used) (p19).
- `chroma_format`/`bit_depth_*` present **only** for profiles 100/110/122/144; `bit_depth = 8 + minus8`,
  range 0–4 (p19). All SPS in one record must share chroma_format + bit depths, else use a second record.
- SPS/PPS arrays in **ascending parameter-set id** (gaps allowed). Record is **externally framed**
  (size from the containing box). Unknown `configurationVersion` ⇒ don't decode (p17).

## AVC sample entries — §5.4.2 (p20–21)

```
class AVCConfigurationBox extends Box('avcC') { AVCDecoderConfigurationRecord AVCConfig; }
class AVCSampleEntry()  extends VisualSampleEntry('avc1' | 'avc3') { AVCConfigurationBox config; MPEG4ExtensionDescriptorsBox(); /*opt*/ }
class AVC2SampleEntry() extends VisualSampleEntry('avc2' | 'avc4') { AVCConfigurationBox avcconfig; ... }
```
- Box/sample-entry types: `avc1 avc2 avc3 avc4 avcC m4ds btrt` in `stsd` (p20). Optional `btrt`
  (BitRateBox), optional `m4ds` (MPEG-4 extension descriptors for the `esds` path).
- **`avc1`/`avc3` = no extractors/aggregators**; `avc2`/`avc4` require extractor/aggregator toolset
  (Annex A). **`avc1`/`avc2` = parameter sets in the sample entry only**; `avc3`/`avc4` = parameter
  sets may also be **in-band** in the samples (p16, p20).
- If a parameter-set elementary stream (`avcp`, §5.4.3) is used, `numOf{Sequence,Picture}ParameterSets`
  shall both be 0 (p21). `compressorname` recommended `"\012AVC Coding"`.

## HEVC decoder config record `hvcC` — §8.3.3 (p70–72)

```
aligned(8) class HEVCDecoderConfigurationRecord {
  unsigned int(8)  configurationVersion = 1;
  unsigned int(2)  general_profile_space;
  unsigned int(1)  general_tier_flag;
  unsigned int(5)  general_profile_idc;
  unsigned int(32) general_profile_compatibility_flags;
  unsigned int(48) general_constraint_indicator_flags;
  unsigned int(8)  general_level_idc;
  bit(4) reserved='1111'b;   unsigned int(12) min_spatial_segmentation_idc;
  bit(6) reserved='111111'b; unsigned int(2)  parallelismType;
  bit(6) reserved='111111'b; unsigned int(2)  chroma_format_idc;
  bit(5) reserved='11111'b;  unsigned int(3)  bit_depth_luma_minus8;
  bit(5) reserved='11111'b;  unsigned int(3)  bit_depth_chroma_minus8;
  unsigned int(16) avgFrameRate;
  unsigned int(2)  constantFrameRate;
  unsigned int(3)  numTemporalLayers;
  unsigned int(1)  temporalIdNested;
  unsigned int(2)  lengthSizeMinusOne;
  unsigned int(8)  numOfArrays;
  for (j=0; j<numOfArrays; j++) {
    unsigned int(1)  array_completeness;
    bit(1) reserved=0;
    unsigned int(6)  NAL_unit_type;        // restricted to VPS / SPS / PPS / prefix SEI / suffix SEI
    unsigned int(16) numNalus;
    for (i=0; i<numNalus; i++) { unsigned int(16) nalUnitLength; bit(8*nalUnitLength) nalUnit; }
  }
}
```
- The profile/tier/level + `min_spatial_segmentation_idc` + `chroma_format_idc` + bit-depths mirror
  the SPS PTL bytes per 23008-2 (p72). `lengthSizeMinusOne` ∈ **{0,1,3}** (p72).
- `parallelismType` (p71): 0 mixed/unknown · 1 slice · 2 tile · 3 entropy-sync — derivable from
  `tiles_enabled_flag`/`entropy_coding_sync_enabled_flag`.
- `numTemporalLayers` > 1 ⇒ temporally scalable; `temporalIdNested`; `avgFrameRate` in frames/(256 s),
  0 = unspecified; `constantFrameRate` 1 = CFR (p72).
- Arrays carry **VPS/SPS/PPS/prefix SEI/suffix SEI only**; recommended order **VPS, SPS, PPS, prefix
  SEI, suffix SEI** (p70). `array_completeness=1` ⇒ all NALs of that type are in the array, none
  in-band (p72). Unknown `NAL_unit_type` arrays ignored (tolerant).

## HEVC sample entries — §8.4.1 (p73)

```
class HEVCConfigurationBox extends Box('hvcC') { HEVCDecoderConfigurationRecord HEVCConfig; }
class HEVCSampleEntry() extends VisualSampleEntry('hvc1' | 'hev1') { HEVCConfigurationBox config; MPEG4ExtensionDescriptorsBox(); /*opt*/ }
```
- Box/sample-entry types `hvc1 hev1 hvcC` in `stsd` (p73). Optional `btrt`.
- **`hvc1` = parameter sets in the sample entry only** (default `array_completeness=1`); **`hev1` =
  parameter sets may also be in-band** in the samples (default `array_completeness=0`) (p73, §8.4.2 p74).
- `compressorname` recommended `"\013HEVC Coding"`.
- **Sync sample** = sample whose VCL NALs are IDR / CRA / BLA (§8.4.3 p74).

## Semantics that bite
- **`avc1`/`avc2` store parameter sets in the sample entry OR a parameter-set stream — never both**
  (§5.3.2 NOTE 1); `avc3`/`avc4` allow in-band too. HEVC mirrors this: `hvc1` = sample-entry only
  (`array_completeness` mandatory 1), `hev1` = also in-band (default 0) (§8.3.2, §8.4.1).
- **One profile/chroma/bit-depth per record.** If SPSs carry different profiles and the compat flags
  are all zero, the stream must be examined; if no single profile conforms (or chroma/bit-depth/VUI
  colour-space differ), the stream is **split into sub-streams with separate config records / sample
  entries** (§5.3.3, §8.3.3). A transmux must group ES by config and emit one sample entry per group.
- **A new parameter set ⇒ a new sample entry** when params live in the sample entry (`avc1`/`avc2`,
  or HEVC `array_completeness=1`) — params can't be updated in place (§5.4.4, §8.4.2).
- Config-record arrays may also carry **declarative SEI** (e.g. user-data SEI about the whole
  stream); readers ignore unknown/reserved NAL-type arrays (forward-compat tolerance).
- A parameter-set **track** (`avcp`, track-ref `avcp`) is the alternative to in-stream/sample-entry
  params: a sync sample there supplies all params needed from that decode time forward (§5.4.3).

## Implications for our crates
- A TS→MP4 transmux lifts SPS/PPS(/VPS) from the elementary stream into `avcC`/`hvcC` (config in the
  sample entry, **not** inline in samples) and rewrites Annex-B start codes to `lengthSizeMinusOne+1`
  length prefixes per sample.
- Choose sample-entry name by where parameter sets live: `avc1`/`hvc1` (sample entry only) vs
  `avc3`/`hev1` (also in-band). Build the high-profile `avcC` ext only for profile 100/110/122/144.
- `avcC`/`hvcC` are externally framed — serialize length from contents; reserved bits all-ones
  (AVC/HEVC) except the single `bit(1) reserved=0` before `NAL_unit_type` in `hvcC` arrays.
