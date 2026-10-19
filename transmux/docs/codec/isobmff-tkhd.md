# Track Header Box (`tkhd`) — width / height

Transcription for display-geometry follow-up "tkhd presentation size"
(`docs/display-geometry/DESIGN.md` §8 Phase 4). **ISO/IEC 14496-12 §8.3.2 itself
is not available in this workspace** (paid; the `specs/iso_iec_*` local copies
are gone — see `specs/INCOMING-MANIFEST.md`). Two sources that ARE available
(both in the private submodule, both verified below against the rendered PDF
page, not only the markdown) pin down what this crate needs:

## 1. Box layout — Adobe F4V File Format Specification v10.1, §2.5.1

Source: `private/specs/adobe_flv_f4v_v10_1.pdf` page 18 (printed page 12);
markdown `private/specs-md/adobe_flv_f4v_v10_1.md` lines 495-545. The F4V spec
transcribes the ISO box and ends with "For more information, see section 8.3.2
of ISO/IEC 14496-12:2008." Layout (verified against the rendered page; the
markdown matches it):

| Field | Type | Comment |
|---|---|---|
| Header | BOXHEADER | BoxType = 'tkhd' (0x746B6864) |
| Version | UI8 | Either 0 or 1 |
| Flags | UI24 | Bit 0 = track enabled; bit 1 = part of the presentation; bit 2 = considered when previewing |
| CreationTime | UI32 (v0) / UI64 (v1) | seconds since midnight, 1904-01-01 UTC |
| ModificationTime | UI32 (v0) / UI64 (v1) | same epoch |
| TrackID | UI32 | the track's unique identifier |
| Reserved | UI32 | 0 |
| Duration | UI32 (v0) / UI64 (v1) | in the Movie Header timescale |
| Reserved | UI32[2] | 0 |
| Layer | SI16 | front-to-back ordering, expected 0 |
| AlternateGroup | SI16 | 0 |
| Volume | SI8.8 | 0x0100 for audio, otherwise 0 |
| Reserved | UI16 | 0 |
| TransformMatrix | SI32[9] | {0x00010000,0,0, 0,0x00010000,0, 0,0,0x40000000} |
| **Width** | **UI16.16** | width as a fixed point 16.16 number |
| **Height** | **UI16.16** | height as a fixed point 16.16 number |

The F4V text gives no semantics for Width/Height beyond the 16.16 encoding
(integer part in the high 16 bits, fraction in the low 16).

## 2. Semantics for DVB-DASH — ETSI TS 103 285 V1.4.1 (2023-09), §5.1.2

Source: `private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf`
page 27 (verified on the rendered page); markdown
`private/specs-md/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.md` line 1231.
Applies where the 'avc1'/'avc2' sample entries are used and the initialization
segment is common to the Adaptation Set:

> For video Representations, the width and height values in the track header
> box **shall have the nominal display size in square pixels after decoding,
> H.264/AVC cropping, and rescaling.**

That is the normative statement this crate relies on: `tkhd` width/height are a
**presentation size on a square-pixel grid** — cropped, then rescaled for the
sample aspect ratio — NOT the coded size. For SAR `w:h` that means
`width = cropped_width × w / h`, `height = cropped_height` (horizontal
rescale, per the ISO note quoted in `isobmff-visual-ext.md` that "normally,
the horizontal dimension of the video is scaled"). The profile does not state
a rounding rule for a non-integer product; that is a free implementation
choice (16.16 can carry the fraction).

## Not verified (honest gaps)

- ISO/IEC 14496-12 §8.3.2 wording (all editions), including the 'iso3'-brand
  "notional square grid" remark that secondary sources quote. Nothing here is
  taken from those secondary sources.
- The DVB rule is stated for H.264 'avc1'/'avc2'; TS 103 285 has no equivalent
  sentence for HEVC (grep: only line 1231 mentions the track header width and
  height). Applying it to HEVC is an extension, not a citation.
