---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: Annex C Dolby Vision streams within the ISO base media file format (C.1-C.3, C.6, C.7)
pdf_pages: 236-240
printed_pages: Annex C pp. 8-12
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/annex-c-dolby-vision-isobmff.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 Annex C - Dolby Vision in ISOBMFF (demuxer notes)

Original-wording notes; verbatim in the private path above. A demuxer reading a Dolby Vision init segment needs the
`dvcC` box and the two sample-entry types below. The MPD side (`@codecs`, `@dependencyId`, profile/level strings)
is in [ch10-uhd-extensions.md](./ch10-uhd-extensions.md) (10.4.3) and
[annex-d-dolby-vision-signalling.md](./annex-d-dolby-vision-signalling.md). SEI payload syntax (Annex E, F) is not
covered.

## Boxes (C.2, C.3; PDF 236-238)

| Item | Value |
|---|---|
| Configuration box | type `dvcC`, class `DOVIConfigurationBox extends Box('dvcC')`, holds one `DOVIDecoderConfigurationRecord` named `DOVIConfig` |
| Container of `dvcC` | the Dolby Vision HEVC sample entries (`dvhe`, `dvh1`); mandatory, exactly one |
| Sample entries | `dvhe` (class `DolbyVisionHEVCSampleEntry`, extends `HEVCSampleEntry('dvhe')`) and `dvh1` (`DolbyVisionHVC1SampleEntry`, extends `HEVCSampleEntry('dvh1')`), each carrying `DOVIConfigurationBox config`; live in `stsd`; mandatory; one or more of the same type allowed |
| `Compressorname` | recommended value is a Pascal string of 11 bytes spelling the compressor name `DOVI Coding` (length byte 0x0B) |

`DOVIDecoderConfigurationRecord` (byte-aligned) fields in order:

| Field | Bits | Meaning / required value |
|---|---|---|
| `dv_version_major` | 8 | major version of the Dolby Vision spec; 1 for streams compliant with this text |
| `dv_version_minor` | 8 | minor version; 0 |
| `dv_profile` | 7 | Dolby Vision profile id (table in the Dolby signalling annex, here Annex D) |
| `dv_level` | 6 | Dolby Vision level id (same annex) |
| `dv_metadata_present_flag` | 1 | 1 = track contains the supplemental enhancement information of 10.4.2.2 |
| `el_present_flag` | 1 | 1 = track contains the EL HEVC substream |
| `bl_present_flag` | 1 | 1 = track contains the BL HEVC substream |
| `reserved` | 5 x 32 | constant 0 |

## Files and dual-track layout (C.6, C.7; PDF 238-240)

- File brand: `dby1` should be in `compatible_brands`; `major_brand` is an ISO brand such as `iso6` (C.6).
- A DV stream may be stored as one file with separate BL and EL tracks, each with its own sample description (C.7).
- EL track constraints (C.7.1): sample entry `dvhe` or `dvh1`, containing both `hvcC` and `dvcC`; `handler_type` =
  `vide`; video media header in `minf`; a `tref` of type `vdep` in the EL track points at the BL track;
  `dv_profile`/`dv_level` set from the encoded stream; `dv_metadata_present_flag` = 1; `el_present_flag` 0 or 1;
  `bl_present_flag` = 0.
- Sample table nesting of the EL track (Table 33, non-exhaustive): `stbl` contains `stsd` (with `dvhe`/`dvh1`, which
  hold `hvcC` and `dvcC`), `stts`, `stsc`, `stsz`/`stz2`, `stco`/`co64`.
- Movie fragments (C.7.2.1): BL and EL samples go in separate fragments, interleaved BL then EL; every BL
  `moof`/`mdat` pair is immediately followed by an EL pair with the same sample count and identical composition
  timestamps.
- `tfra` (C.7.2.2): per 14496-12 §8.8.10, and the `time` of each random-access sample must be identical in the BL
  and EL tracks.

## Spec notes (source oddities)

- C.2.1's container list ends with a dangling "or" (p. 8).
- Cross-references use the Dolby document's numbering ("Annex B", "Table B.1/B.2", "Section A.7.1", "Section A.3",
  "Section 3.1"), not this IOP's (Annex D, C.7.1, C.3, C.2).
- C.7.1 names the entries `DolbyVisionHEVCVisualSampleEntry` / `...HVC1VisualSampleEntry`, C.2/C.3 omit "Visual".
- Annex C numbering skips C.4 and C.5.
