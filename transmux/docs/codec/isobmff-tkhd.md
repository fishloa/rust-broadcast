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

The F4V text gives no semantics for Width/Height beyond "expressed as a fixed
point 16.16 number". (By the usual meaning of that notation the integer part is
the high 16 bits and the fraction the low 16; the F4V page does not spell this
out.)

## 1b. Apple QuickTime File Format Specification (2001-03-01), "Track Header Atoms"

The ancestor of the ISO box. Source: `private/specs/apple_quicktime_file_format.pdf`
printed pages 41-43 (field list p. 41 figure 2-7; element descriptions pp. 41-43;
Track width/height on page 43, verified on the rendered page);
markdown `private/specs-md/apple_quicktime_file_format.md` lines 1233-1316.
Same field order as the F4V table (Version 1 byte, Flags 3 bytes, the 32-bit
times/IDs/duration of version 0, 36-byte matrix). Differences worth knowing:
QuickTime defines a fourth flag, `Track in poster` = `0x0008` (the F4V table
lists three bits), and only the 32-bit (version 0) layout is drawn. The two
fields that matter here, verbatim:

> **Track width**: A 32-bit fixed-point number that specifies the width of this
> track in pixels.
> **Track height**: A 32-bit fixed-point number that indicates the height of
> this track in pixels.

"In pixels" is not qualified as coded or square pixels; the DVB rule below is
what disambiguates it for DASH content.

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

## 3. Related DVB-DASH statement on `@width`/`@height` — ETSI TS 103 285 V1.4.1, §4.4

Source: same PDF, page 23 (verified on the rendered page); markdown line ~1058.
For video Adaptation Sets `@maxWidth`/`@maxHeight` (or `@width`/`@height` when
all Representations agree) and `@par` "should be present"; and:

> NOTE: The attributes @maxWidth and @maxHeight are expected to be used such
> that they describe the target display size. This means that they may exceed
> the actual largest size of any coded Representation in one Adaptation Set.

Each Representation "shall" carry `@width`/`@height` unless the Adaptation Set
does. Note the text does not say which size (coded or display) a Representation's
`@width`/`@height` hold; only the `@max*` NOTE and the `tkhd` rule above speak of
display size.

## Not verified (honest gaps)

- ISO/IEC 14496-12 §8.3.2 wording (all editions), including the 'iso3'-brand
  "notional square grid" remark that secondary sources quote. Nothing here is
  taken from those secondary sources.
- The DVB rule is stated for H.264 'avc1'/'avc2'; TS 103 285 has no equivalent
  sentence for HEVC (grep: only line 1231 mentions the track header width and
  height). Applying it to HEVC is an extension, not a citation.
