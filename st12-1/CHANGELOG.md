# Changelog

All notable changes to `st12-1` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-12

### Added

- `LtcFrame` — parser+serializer for the SMPTE ST 12-1:2014 §9.2 80-bit LTC
  codeword: BCD hours (`00`-`23`)/minutes/seconds (`00`-`59`)/frames
  (`00`-`29`, the widest per-rate bound), the drop-frame flag (bit 10) and
  color-frame flag (bit 11, fixed positions across all frame rates), the
  eight 4-bit binary groups ("user bits", Table 4), and the fixed
  synchronization word (`0xFC 0xBF`, Table 5 — validated on parse, always
  written on serialize).
- `FrameRate` — which of Table 3's three flag-bit-position columns (30-frame
  / 25-frame / 24-frame) applies to the four remaining flag bits (positions
  27/43/58/59): 30-frame and 24-frame share one mapping (polarity
  correction/BGF0/BGF1/BGF2), while 25-frame swaps bit 27 and bit 59's
  meaning. `LtcFrame::polarity_correction`/`LtcFrame::binary_group_flags`
  resolve these bits against a caller-supplied `FrameRate`, since the 80-bit
  codeword itself carries no self-describing frame-rate field.
- `BinaryGroupUsage`/`BinaryGroupFlags` — Table 1's classification of what
  the binary groups contain, from the three binary group flag bits
  (BGF2/BGF1/BGF0).
- `docs/st12-1.md` — curated transcription of ST 12-1:2014 §8/§9 (fetched
  directly from `pub.smpte.org`), including a note on Table 3's
  frame-rate-dependent bit-position swap verified against the rendered PDF
  page image (not just a plain-text extraction, since a column-transposition
  bug there would silently produce a plausible but wrong table).
- Spec-derived vectors (`tests/spec_vectors.rs`): no real captured LTC
  bitstream exists in this workspace (LTC is normally carried as an
  out-of-scope biphase-mark-encoded analog signal, so there is nothing to
  extract a logical codeword from), so every byte vector is computed
  directly from the ST 12-1 Tables 2-5 bit diagrams by a standalone script,
  independently of this crate's own `Serialize` — documented per
  `docs/CRATE-ACCEPTANCE.md`'s fallback provenance rule.
- Two runnable examples: `build_frame` (construct + serialize from typed
  fields) and `parse_frame` (parse a spec-derived vector, decode
  `FrameRate`-dependent flags, and byte-exact round-trip).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`); builds
  standalone with `--no-default-features` and on a bare-metal
  (`thumbv7em-none-eabi`) target. Needs no heap allocation at all (every
  field is a fixed-size scalar).
- `serde` support behind the `serde` feature (`Serialize` + `Deserialize`,
  since every type here is owned with no borrowed lifetime).
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard; both public spec/field enums (`FrameRate`, `BinaryGroupUsage`)
  carry `name()` + `impl_spec_display!`.
- New fuzz target `st12_1`: exercises `LtcFrame::parse` + round trip on
  arbitrary bytes.
