# Changelog

All notable changes to `st337` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-12

### Added

- `Burst` — parser+serializer for one complete SMPTE ST 337:2015 §7 non-PCM
  data burst: `BurstPreamble` (`Pa`/`Pb` fixed sync words, `Pc` burst-info,
  `Pd` length_code, and the six-word "extended" form's `Pe`/`Pf`) followed by
  the opaque `burst_payload` bytes.
- `BurstPreamble` — `data_type` (5 bits, `0..=31`, `31` escapes to the
  extended form), `data_mode` (`DataMode`, Table 8), `error_flag`,
  `data_type_dependent` (5 bits, opaque — meaning is per-`data_type`, defined
  in companion spec ST 338), `data_stream_number` (3 bits, `0..=7`), and
  `length_code` (the wire `Pd` value, in bits per the spec's literal text,
  including Pe/Pf's 32 bits when the extended form is used — §7.2.5).
- `DataMode` — the `data_mode` field (§7.2.4.3 Table 8): `Mode16`/`Mode20`/
  `Mode24`/`Reserved`. Only `Mode16` is supported by `Burst::parse`/
  `Burst::new` (`Error::UnsupportedDataMode` otherwise) — the 20-/24-bit
  modes imply differently-sized physical preamble words that don't fit this
  crate's uniform 2-bytes-per-word byte-stream abstraction; see
  `docs/st337.md`'s "Scope decisions" for the reasoning. Carries the
  `name()`/`Display` label pair per the workspace's #204 convention.
- `ExtendedPreamble` — `Pe` (`extended_data_type`) + `Pf` (reserved,
  preserved verbatim on round-trip rather than forced to zero). Required
  present iff `data_type == 31`, enforced on both parse and serialize.
- No `data_type` -> codec enum: the mapping is registered in SMPTE ST 338,
  which was not available to independently verify (per this project's
  "no implementation without a truthful source" discipline) — `data_type`
  is a plain validated `u8`.
- `docs/st337.md` — curated transcription of SMPTE ST 337:2015 §1-§8 +
  Annexes A-C (fetched directly from `pub.smpte.org`), the implementation/
  audit oracle for this crate, including the crate's scope decisions.
- `docs/st337-PROVENANCE.md` — a real-fixture + independent-oracle
  (`ffmpeg -f spdif`, IEC 61937) cross-check of the `Pa`/`Pb` sync-word
  constants, byte-order convention, and `Pc` bit layout against real running
  software wrapping a real E-AC-3 elementary stream — including a genuine,
  documented discrepancy between ST 337's own `length_code` semantics (bits)
  and IEC 61937's for the AC-3/E-AC-3 data type (bytes).
- Real-fixture test (`tests/fixture_eac3.rs`) wrapping a real 834-byte E-AC-3
  syncframe (extracted from this workspace's `fixtures/ts/dolby/eac3.ts`,
  committed as `tests/fixtures/eac3_frame0.bin`) in a hand-built ST 337
  burst, with byte-identical parse/serialize round-trip coverage and a
  mutation test guarding against a `self.raw`-style passthrough serializer.
- Spec-derived edge-case round-trip tests (`tests/round_trip.rs`): zero-length
  and maximum-length (`length_code` = 65535 bits) payloads, the full
  `data_type`/`data_type_dependent`/`data_stream_number` code spaces, and the
  six-word extended-preamble form (including its `length_code` Pe/Pf-offset
  quirk and verbatim-preserved reserved `Pf`).
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard.
- Two runnable examples: `build_burst` (construct + serialize from typed
  fields) and `parse_burst` (wrap the real E-AC-3 fixture, parse it back, and
  confirm the byte-identical payload).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`) + `alloc`;
  builds standalone with `--no-default-features` and on a bare-metal
  (`thumbv7em-none-eabi`) target.
- `serde` support (`Serialize`/`Deserialize` on owned types, `Serialize`-only
  on the borrowed `Burst`) behind the `serde` feature.
- New fuzz target `st337`: exercises `Burst::parse` + round trip on
  arbitrary bytes.
