# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-08-08

### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806). No public API or behaviour change.
- The 8 remaining Table 8-3 (`single_operation_message`) operations (issue
  #936): `config_request`/`config_response` (§10.4, opID `0x0009`/`0x000A`),
  `provisioning_request`/`provisioning_response` (§10.5, opID
  `0x000B`/`0x000C`), `fault_request`/`fault_response` (§10.6, opID
  `0x000F`/`0x0010`), and `as_alive_request`/`as_alive_response` (§10.7, opID
  `0x0011`/`0x0012`). These were previously falling through to
  `AnySingleOperation::Unknown` (opaque, round-tripped but untyped). All 15
  Table 8-3 opIDs are now typed, alongside the already-complete 22/22 Table
  8-4. Field layouts transcribed into new
  `docs/ansi_scte_104/pams_operations.md` (§10, pp. 69-81, verified via
  `pdf2md` against the PDF text layer). **User-visible**: new public types
  `scte104::operations::{ConfigRequest, ConfigResponse, ProvisioningResponse,
  FaultRequest, FaultResponse, AsAliveRequest, AsAliveResponse}` and
  `scte104::operations::provisioning_request::{ProvisioningRequest,
  ProvisioningService, DpiPidEntry, InjectorComponentList}`, plus new
  `AnySingleOperation` variants — additive, no breaking change.
- `tests/spec_vectors.rs` (issue #936): hand-derived byte vectors computed
  directly from the ANSI/SCTE 104 2023 syntax tables (Tables 8-1/8-2/9-5/
  10-1/10-3/10-5), independently of this crate's own serializer, asserting
  field values at documented byte offsets before round-tripping. Closes the
  gap where every prior test was self-referential (build → serialize → parse
  → compare against itself), which cannot distinguish a correct
  implementation from a self-consistent wrong one.

### Fixed
- `src/lib.rs` crate-doc coverage claim ("All operations from Tables 8-3 and
  8-4") was false — only 7 of 15 Table 8-3 opIDs were dispatched (issue #936).
  Now genuinely true (15/15 + 22/22).

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.0] — 2026-06-20

### Added

- Initial release.
- `SingleOperationMessage` framing (§8.2.2) with all basic operations:
  `general_response`, `init_request`, `init_response`, `alive_request`,
  `alive_response`, `inject_response`, `inject_complete_response`.
- `MultipleOperationMessage` framing (§8.2.3) with `timestamp()` support
  (none/UTC/VITC/GPI).
- All Normal operations: `inject_section_data`, `splice_request`,
  `splice_null_request`, `start_schedule_download`, `time_signal_request`,
  `transmit_schedule`, `proprietary_command`.
- All Supplemental operations: `component_mode_DPI`, `encrypted_DPI`,
  `insert_descriptor`, `insert_DTMF_descriptor`, `insert_avail_descriptor`,
  `insert_segmentation_descriptor`, `schedule_component_mode`,
  `schedule_definition`, `insert_tier`, `insert_time_descriptor`,
  `insert_audio_descriptor`, `insert_audio_provisioning`,
  `insert_alternate_break_duration`.
- All Control operations: `delete_ControlWord`, `update_ControlWord`,
  `insert_audio_provisioning`.
- `time()` structure (§12.4): 8-byte GPS-epoch timestamp used in alive messages.
- `timestamp()` structure (§12.5): variable-length timestamp with time_type
  discriminator.
- `AnyOperation` dispatch enum with opID drift test.
- Symmetric `Parse`/`Serialize` on every wire type (no raw passthrough).
- `#![no_std]` + alloc compatible; serde behind `serde` feature.
- Two runnable examples: `build_splice` and `multi_op_round_trip`.

[0.1.0]: https://github.com/fishloa/rust-dvb/releases/tag/v0.1.0-scte104
