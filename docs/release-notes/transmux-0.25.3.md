# transmux 0.25.3

_Released 2026-10-12._

### Fixed

- `TsDemux`/`StreamingTsDemux`: field-coded (PAFF) H.264 now yields **one sample per complementary field pair** instead of one per field (#1161). Broadcast PAFF (TF1, M6, Gulli, arte, ...) sends one PES per field; ISO/IEC 14496-15 samples are whole access units, and a per-field sample decodes to half-filled frames. The second field (opposite parity, same `frame_num`, both reference or both non-reference, not an IDR; ITU-T H.264 §3.35-3.37) is merged into the pending first field's sample, which keeps the first field's PTS/DTS (its AUD is dropped, so the sample holds one AUD). An unpaired field, or a second field arriving more than one second after the first (discontinuity), is emitted alone. MBAFF (`field_pic_flag = 0`), progressive and PAFF-free streams are unchanged. The only golden hash that moved is `ts/gulli-opengop.ts`, a real PAFF capture (293 fields -> 147 samples). Verified bit-exact against `ffmpeg` on spec-built TFF/BFF fixtures (`tests/fixtures/paff/`).

### Added

- `transmux::slice_header`: bit-exact H.264 slice-header prefix (§7.3.3), PPS prefix (§7.3.2.2) and field-pair classification (`AvcParamSets::classify_au`, `AvcFieldInfo::pairs_with`); `sps::decode_avc_slice_sps` for the SPS fields a slice header needs. Spec transcription: `docs/codec/h264-slice-header.md`.

### Behaviour change worth knowing

Field-coded (PAFF) H.264 now yields one sample per complementary field pair (was one per field), PTS/DTS from the first field; MBAFF and progressive output is unchanged. A committed real PAFF fixture (`gulli-opengop.ts`) goes from 293 to 147 samples. Samples are whole access units as ISO/IEC 14496-15 requires, so decoders such as VideoToolbox no longer emit half-filled frames.

### Verification

Spec-built PAFF generator streams (TFF and BFF; licence MIT OR Apache-2.0, README in `tests/fixtures/paff/`) decode bit-exact in ffmpeg and demux to 12 samples for 12 frames; batch equals streaming at 75 chunk sizes; TS and fMP4 re-mux round trips; revert-verified (6 of 8 tests fail with pairing disabled). The slice-header syntax is transcribed from ITU-T H.264 in `transmux/docs/codec/h264-slice-header.md`.

---

Published from tag `transmux-v0.25.3`.
