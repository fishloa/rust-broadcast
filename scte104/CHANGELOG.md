# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
