# Display geometry: spec transcription notes

Companion to [`DESIGN.md`](DESIGN.md). Only what the design depends on is
transcribed. Every table here was checked against the **rendered PDF page**
(`pdftoppm -r 90`), not only the markdown transcription, because the
`private/specs-md` text of several of these tables is mangled (rows of
H.264 Table E-1 1..7 are merged into one paragraph; TS 101 154 Table B.5 has
split cells). Page numbers are PDF page indices in `private/specs/*.pdf`.

| Source | File | Pages checked |
|---|---|---|
| ITU-T H.264 (V15, 08/2024) | `itu_t_h264_avc.pdf` | 455 (printed 431): §E.2.1, Table E-1 |
| ITU-T H.265 (V11, 01/2026) | `itu_t_h265_hevc.pdf` | 449 (printed 431): Table E.1 |
| ETSI TS 101 154 V2.10.1 (2026-05) | `etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf` | 235-237: Tables B.1-B.5; 238-241 (text layer): Table B.7; 243-246 (text layer): Table B.11, §B.8a |

> **Sections 1-5 moved.** The transcribed H.264/H.265 VUI tables (§1 sample aspect ratio / Table E-1, §2 video
> signal type and colour description, §3 cropping / conformance and default display windows, §4 parameter-set
> activation) and the TS 101 154 Annex B AFD / bar-data tables (§5) are (c) ITU-T / ETSI and live only in the
> private companion repo: `private/specs-verified/mpeg/display-geometry/SPEC-NOTES.md` (old path
> `docs/display-geometry/SPEC-NOTES.md`, see `docs/specs/PRIVATE-MAP.md`). Sections 6-9 below are this
> project's own findings and stay public; `DESIGN.md` references to §1-§5 mean the private copy.

## 6. ISO BMFF visual sample entry boxes

Only what `transmux/docs/codec/isobmff-visual-ext.md` already transcribes
(ISO/IEC 14496-12:2015 §12.1.4.2/§12.1.4.3/§12.1.5): `pasp` (hSpacing,
vSpacing u32, both positive, only the ratio matters), `clap` (eight u32 N/D
fractions, centre-relative), `colr` (`nclx`: colour_primaries u16,
transfer_characteristics u16, matrix_coefficients u16, full_range_flag u1 +
7 reserved). §12.1.5.1: "If colour information is supplied in both this box,
and also in the video bitstream, this box takes precedence, and over-rides the
information in the bitstream." The transcription starts mid-sentence: the
§12.1.4.1 definition (box placement, and whether `pasp`/`clap` take precedence
over VUI) and §8.3.2 (`tkhd` width/height = visual presentation size) are
**not** transcribed. See the gaps list in DESIGN.md §12.

## 7. Manifests

- DVB-DASH, ETSI TS 103 285 V1.4.1 (md lines 1045-1072): for video Adaptation
  Sets `@par` "should be present"; for each video Representation `@sar` "should
  be present or inherited"; "for which the picture aspect ratio is other than
  16:9, at least one of the attributes @par and @sar shall be present or
  inherited". The attribute *syntax* is ISO/IEC 23009-1, not transcribed.
- HLS, RFC 8216 §4.3.4.2 / draft-pantos-hls-rfc8216bis: `RESOLUTION` is "the
  optimal pixel resolution at which to display all the video in the Variant
  Stream" (no SAR attribute exists). rfc8216bis `VIDEO-RANGE` (SDR/HLG/PQ) is
  derived from the TransferCharacteristics code point ([CICP] 1, 6, 13, 14, 15
  => SDR; 18 => HLG; see the draft for PQ).

## 8. DVB component descriptor (EN 300 468 §6.2.8, Table 26)

Carried in SDT and EIT (EN 300 468 V1.20.1 Table 12, md line 2392), not the
PMT. "When used in the SDT and EIT these component stream characteristics ...
describe the editorial characteristics of the event or service. These are
intended to be used by receivers to inform the user" (md line 2700). Table 26
rows for H.264 SD video distinguish "4:3 aspect ratio", "16:9 aspect ratio",
"> 16:9 aspect ratio" per frame rate. It is event-granular, not frame-accurate.
Already parsed by `dvb-si` (`descriptors/component.rs`).

## 9. Colour tables: markdown garbling found in Phase 1b

Read from the **rendered** pages, not the md (details per table in
`transmux/docs/codec/vui-colour-tables.md`):

- **H.264 Table E-3 (md)**: each row 1..=12 is one multi-line cell in the PDF
  (primary/green/blue/red/white with x, y); the md splits them into separate
  rows, merges one row's informative remark into the next, and for row 11
  emits `red 0.680 0.320` / `white 0.314 0.351` as a bare paragraph between
  rows 11 and 12. No code point is lost.
- **H.264 Table E-4 (md)**: `2 Unspecified` and `3 Reserved` lost their pipes
  and merged into row 1's prose, so a naive md scrape **misses two defined
  rows**.
- **H.264 Table E-5 (md)**: the header lost its pipes (`Value Matrix
  Informative remark`) and rows 0..=7, 9..=15 merged into one paragraph; only
  16, 17 and 18..=255 survive as table rows. A naive scrape would treat 8
  (YCgCo) and, worse, the BT.601/709 rows as absent.
- **H.265 Tables E.3/E.5 (md)**: the header row is repeated mid-table at the
  page break; E.5's YCgCo-R equation references differ from H.264's (H.265
  numbering). Neither affects a label.
- **Naming**: H.265 §E.2.1 spells the middle colour element `matrix_coeffs`,
  H.264 `matrix_coefficients`; same `u(8)` field and same table.
- **Row differences**: only `video_format` 6 and 7 differ in meaning between
  the specs (H.264 "Reserved" with no interpretation; H.265 "interpret as 5").
  Tables E-3/E-4/E-5 and E.3/E.4/E.5 define the same rows.

