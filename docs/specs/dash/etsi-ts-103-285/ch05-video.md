---
source: private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
document: ETSI TS 103 285 V1.4.1 (2023-09)
clauses: 5 DASH specific aspects for video (5.1 AVC, 5.2 HEVC, 5.3 VVC, 5.4 AVS3)
pdf_pages: 27-40 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch05-video.md
verified_against_render: true
notes_original_wording: true
---

# TS 103 285 - clause 5 (notes): video

Original-wording notes. Codec string grammars restate RFC 6381 / ISO/IEC 14496-15 (see
`transmux/docs/codec/avcC-hvcC-14496-15.md`); only the DVB additions are noted. Luminance-resolution Tables 25-27a
(referenced from 5.1.1/5.2.3) are in clause 10.3, not here.

## 5.1 H.264/AVC (pp. 27-29)
- Profiles (5.1.1): follow TS 101 154 clause L.3.2; an MPD with AVC shall include content compatible with
  `avc_hd_50_level40` or `avc_hd_60_level40` unless targeting devices known to handle other aspect ratios.
- Packaging (5.1.2): ISO/IEC 14496-15; segments shall start at SAP 1 or 2; players shall handle both `avc1` and `avc3`
  (parameter sets in init segment vs inband). Content should use inband sets (`avc3`/`avc4`); `avc1`/`avc2` allowed
  but then the init segment shall be shared by all Representations of a set, track-header width/height shall be the
  nominal display size in square pixels (after cropping/scaling), and the init segment shall carry all SPS/PPS for the
  whole Period. With `avc3`/`avc4`, each coded video sequence should repeat its SPS/PPS in its first access unit; a PPS
  with a given id shall not change within a sequence. A shared init segment means one `stsd` with one entry per
  Representation, selected through the sample description index in the track fragment header.
- Codec string (5.1.3): `@codecs` should be given for dvb-dash:2014 and shall be given for dvb-dash:2017; format per
  RFC 6381 3.3: `avc` + {1,2,3,4}, then profile, constraint-flags and level as three hex bytes (profile/constraints/level
  from the SPS/AVCDecoderConfigurationRecord). Values need only be sufficient, not minimal; on an AdaptationSet the
  string must hold for every Representation. Examples (Table 1; avc1 shown, avc3 identical with the prefix changed):

| Profile | Level | `avc1` string |
|---|---|---|
| Constrained Baseline | 2.1 / 3.0 | avc1.42c015 / avc1.42c01e |
| Main | 3.0 / 3.1 | avc1.4d401e / avc1.4d401f |
| High | 3.0 / 3.1 / 3.2 / 4.0 | avc1.64001e / avc1.64001f / avc1.640020 / avc1.640028 |

- 5.1.4: values in the AVCSampleEntry shall bound (never be exceeded by) all Representations using that init segment.

## 5.2 HEVC (pp. 29-33)
- 5.2.1: players shall support `hvc1` and `hev1`; SAP 1 or 2 at segment start; with `hev1` each coded video sequence
  should carry its parameter sets in its first access unit and a PPS shall not change inside a sequence.
- 5.2.2 codec string: same should/shall rule as AVC per profile; grammar from 14496-15 clause A.3 (spec notes a bug
  fixed by a corrigendum and reproduces the corrected form): `hvc1|hev1` . [profile space letter A/B/C or none]
  profile_idc . reversed-bit-order compatibility flags (hex, leading zeros omittable) . tier letter `L`/`H` + level_idc
  (decimal) . constraint bytes (hex, dot-separated, trailing zero bytes omittable). Absence of a constraint in the
  string does not mean the media lacks it. Examples (Table 2): Main 3.1/4.1/5.1 = `hvc1.1.6.L93.00`, `.L123.00`,
  `.L153.00` (constraint byte `B0` = progressive_source, frame_only, non_packed gives `...L93.B0` etc.); Main 10 =
  `hvc1.2.4.L93/L123/L153` with `.00` or `.B0`; `hev1` variants only change the prefix.
- 5.2.3: follow TS 101 154 clause L.3.3; include content for at least one HEVC conformance point.
- 5.2.4 SEI placement (should): `hev1` -> SEI inside samples per TS 101 154 L.3.3.8; `hvc1` -> SEI in the decoder
  configuration record, after all VPS/SPS/PPS arrays, applying to all sequences; only declarative whole-stream SEI
  belongs in sample entries.
- 5.2.5 colour signalling (AdaptationSet level only): EssentialProperty/SupplementalProperty with schemes
  `urn:mpeg:mpegB:cicp:ColourPrimaries`, `...MatrixCoefficients`, `...TransferCharacteristics`; `@value` is the integer
  from ISO/IEC 23091-2 Tables 3/4/5; with EssentialProperty it shall equal the VUI field; no descriptor means BT.709.
  Essential = player must support it; Supplemental is defined only for TransferCharacteristics and means "describes
  transfer better than the Essential one; prefer it if understood".
- 5.2.6 HLG10 (bitstream per TS 101 154 L.3.3, L.3.3.6): unless the MPD is only for BT.2020-capable players, signal
  profile dvb-dash:2017 plus Essential ColourPrimaries=9, MatrixCoefficients=9, TransferCharacteristics=14, plus
  Supplemental TransferCharacteristics=18 (HLG). Backwards compatible with BT.2020 players; BT.2020-only MPDs may use
  dvb-dash:2014 and drop the three Essentials (Supplemental still advised).
- 5.2.7 PQ10 (L.3.3.6, L.3.3.10): profile dvb-dash:2017 plus Essential ColourPrimaries=9, MatrixCoefficients=9,
  TransferCharacteristics=16. If SEI carries HDR dynamic mapping, add Supplemental `urn:dvb:dash:hdr-dmi` with value
  `ST2094-10` (TS 101 154 L.3.3.10.4.2), `SL-HDR2` (L.3.3.10.4.3) or `ST2094-40` (L.3.3.10.4.4); several may coexist;
  when present, mapping data shall exist for every frame of every Representation in the set.
- 5.2.8 temporal layers: a Representation shall not contain a temporal layer whose frame rate exceeds the signalled
  one. Where temporal layers are used each Representation shall carry a Supplemental descriptor
  `urn:dvb:dash:highest_temporal_id:2017` with the highest TemporalId in decimal (needed because SPS profile/level
  flags may cover all layers). `@frameRate` then refers to the highest layer. Layers in separate Representations are
  not supported.

## 5.3 VVC (pp. 34-36)
- 5.3.1: players shall support `vvc1` and `vvi1`; SAP 1/2; same parameter-set rules as HEVC with `vvi1`.
- 5.3.2: Representations conform to 23009-1 8.12.4.2 (CMAF DASH profile), CMAF VVC baseline tracks (23000-19 Annex M),
  a TS 101 154 clause 5.15 conformance point and clause 4; AdaptationSets to 8.12.4.3, Annex M switching-set rules and
  clause 4. Players shall decode the 5.15 conformance points and CMAF baseline VVC (with 10.3 constraints).
  Conformance points: `vvc_uhd1_hdr` (TS 101 154 5.15.2), `vvc_uhd1_hdr_hfr` (5.15.3), `vvc_uhd2_hdr` (5.15.4), and
  5.15.5 for the clause titled `vvc_uhd2_hdr_hfr` whose body text names `vvc_uhd2_hdr` (spec inconsistency, p. 35).
- 5.3.3 codec string per 23000-19 clause M.4; examples (Table 3a): Main 10 levels 3.1/4.1/5.1/6.1 = `vvc1.1.L51.CQA`,
  `L67`, `L83`, `L99` (`vvi1` variants likewise).
- 5.3.4 SEI: `vvi1` -> in samples per TS 101 154 5.15.1; `vvc1` -> in configuration record after VPS/SPS/PPS arrays
  (14496-15 11.2.4.2). 5.3.5-5.3.7: colour/HLG10/PQ10 signalling as 5.2.5-5.2.7; bitstream side per TS 101 154
  5.15.1.10.x. 5.3.8: temporal layers as 5.2.8.

## 5.4 AVS3 (pp. 36-40)
- 5.4.1: no extra manifest constraints. 5.4.2: Representations follow 23009-1 8.12.4.2, the AVS3 CMAF track format
  (5.4.5), a TS 101 154 AVS3 conformance point and clause 4; AdaptationSets follow 8.12.4.3, the AVS3 CMAF switching
  set rules (AVS T/AI 109.6) and clause 4. Points: `avs3_uhd1_hdr` (TS 101 154 5.16.3), `avs3_uhd1_hdr_hfr` (5.16.4),
  `avs3_uhd2_hdr` (5.16.5), `avs3_uhd2_hdr_hfr` (5.16.6).
- 5.4.3 codec string per AVS T/AI 109.6 Annex A; Table 3b: High 10-bit levels 6.0.60 / 10.0.120 / 10.4.120 =
  `avs3.32.44` / `avs3.32.68` / `avs3.32.69`.
- 5.4.4: sample entry is `avs3` (AVS T/AI 109.6); a sample is one coded picture; configuration record per 109.6 5.2.2;
  stream structure (sequence header, pictures in decoding order) is background only.
- 5.4.5: CMAF AVS3 track = brand `cmfc` plus brand `avs3`; uses AVS3SampleEntry (109.6 5.3); with several sample
  entries, profile/level/precision fields of the sequence header and range/primaries/transfer/matrix fields of the
  sequence display extension shall not change.
- 5.4.6 colour: same mechanism with schemes `urn:avs3:p6:2022:ColourPrimaries|MatrixCoefficients|TransferCharacteristics`,
  values from AVS T/AI 109.2 Tables 50-52, matching the sequence display extension; Table 3c has the same
  Essential/Supplemental semantics as Table 3.
- 5.4.7 HLG10 (TS 101 154 5.16.2.5.2): dvb-dash:2017 + Essential ColourPrimaries=9, MatrixCoefficients=8,
  TransferCharacteristics=14 (no Supplemental 18, unlike HEVC; Matrix 8 vs 9 is as printed). 5.4.8 PQ10 (5.16.2.5):
  9 / 8 / 12; dynamic-mapping Supplemental `urn:dvb:dash:hdr-dmi` with `ST2094-10` (5.16.2.5.4.3), `SL-HDR2`
  (5.16.2.5.4.4), `ST2094-40` (5.16.2.5.4.5); same all-frames rule.
