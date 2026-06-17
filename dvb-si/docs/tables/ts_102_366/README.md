# ETSI TS 102 366 v1.4.1 — AC-3 / Enhanced AC-3 (frame-header + descriptor reference)

Source: specs/etsi_ts_102_366_v01.04.01_ac3_eac3_audio.pdf §4 BSI/syncframe + code tables (pp.33-48) and Annex A DVB descriptors (pp.96-100). Audio-decode internals (exponents/mantissas/transforms) out of scope.

> Wire-structure reference, table-per-file for deep-linking. Each linked file
> carries one syntax/enum table **plus its field semantics** — enough to drive a
> spec-accurate Rust parser (symmetric Parse/Serialize; coded enums get TOML
> drift-guards when implemented). Transcribed via BlazeDocs (table oracle; not
> pdftotext), spot-checked vs the PDF render. No parser implemented yet.

## Carriage / overview

## 4.4.1.3 fscod - Sample rate code - 2 bits

This is a 2-bit code indicating sample rate according to Table 4.1. If the reserved code is indicated, the decoder should not attempt to decode audio and should mute.

## Tables

- [Table 4.1 — Sample rate codes](tables/4_1-sample-rate-codes.md)
- [Table 4.2 — Bit stream mode](tables/4_2-bit-stream-mode.md)
- [Table 4.3 — Audio coding mode](tables/4_3-audio-coding-mode.md)
- [Table 4.4 — Centre mix level](tables/4_4-centre-mix-level.md)
- [Table 4.5 — Surround mix level](tables/4_5-surround-mix-level.md)
- [Table 4.6 — Dolby® Surround mode](tables/4_6-dolby-surround-mode.md)
- [Table 4.7 — Room type](tables/4_7-room-type.md)
- [Table 4.8 — Time code exists](tables/4_8-time-code-exists.md)
- [Table 4.9 — Master coupling coordinate](tables/4_9-master-coupling-coordinate.md)
- [Table 4.10 — Number of rematrixing bands](tables/4_10-number-of-rematrixing-bands.md)
- [Table 4.11 — Delta bit allocation exist states](tables/4_11-delta-bit-allocation-exist-states.md)
- [Table 4.12 — Bit allocation deltas](tables/4_12-bit-allocation-deltas.md)
- [Table 4.13 — Frame size code table (1 word = 16 bits)](tables/4_13-frame-size-code-table-1-word-16-bits.md)
