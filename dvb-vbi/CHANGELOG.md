# Changelog

All notable changes to `dvb-vbi` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

### Added

- `DataField` — parser+serializer for the EN 301 775 §4.4.1 (Table 1) PES data
  field: a `data_identifier` byte (Table 2) followed by a loop of data units,
  walked until the buffer is exhausted and re-emitted back-to-back.
- `DataUnit` / `DataUnitId` — each data unit's `data_unit_id` (Table 3) + 8-bit
  `data_unit_length` + typed body. `data_unit_length` is recomputed from the
  typed body on serialize (no raw passthrough); parse rejects a length that does
  not match the typed payload size. `DataUnitId` carries `name()` +
  `impl_spec_display!`. ⚠ Per Table 3 (authoritative), `0xC1` is reserved, not
  Teletext (resolving the Table 1 / Table 3 conflict; see `docs/vbi.md`).
- Typed data-unit payloads (`DataUnitPayload`):
  - `TeletextDataField` — EBU (`0x02`/`0x03`) and Inverted (`0xC0`) Teletext
    (§4.5, Table 4): shared `LineHeader` + 8-bit `framing_code`
    (`FRAMING_CODE_EBU` / `FRAMING_CODE_INVERTED`) + 42-byte opaque
    `txt_data_block` (EN 300 706 out of scope).
  - `VpsDataField` — VPS (`0xC3`, §4.6, Table 6): shared header + 13-byte block.
  - `WssDataField` — WSS (`0xC4`, §4.7, Table 8): shared header + 14-bit
    `wss_data_block` + 2-bit `reserved_future_use` `11` tail (3 bytes total).
  - `ClosedCaptioningDataField` — Closed Captioning (`0xC5`, §4.8, Table 10):
    shared header + 16-bit data block.
  - `MonochromeDataField` — monochrome 4:2:2 samples (`0xC6`, §4.9, Table 12):
    first/last segment flags + field_parity + line_offset (own first-byte
    packing, no RFU prefix), `first_pixel_position`, `n_pixels` (derived from
    `samples.len()`), and the luminance `Y_value` bytes.
  - Stuffing (`0xFF`, §4.4.1) and an `Opaque` catch-all for reserved /
    user-defined ids — both round-trip verbatim.
- `LineHeader` — the shared Teletext/VPS/WSS/CC first byte
  (reserved_future_use `11` | field_parity | 5-bit line_offset, §4.5.1 et al.).
- Length/value constants: `TXT_DATA_BLOCK_LEN`, `TELETEXT_FIELD_LEN`,
  `TELETEXT_DATA_UNIT_LENGTH` (`0x2C`), `VPS_DATA_BLOCK_LEN`, `VPS_FIELD_LEN`,
  `WSS_FIELD_LEN`, `WSS_DATA_BLOCK_MASK`, `WSS_RESERVED_TAIL`, `CC_FIELD_LEN`,
  `MONO_HEADER_LEN`, and the `ID_*` `data_unit_id` constants.
- Committed `tests/fixtures/vbi_data_field.bin` fixture (a `data_identifier`
  `0x10` data field carrying one of every typed unit) exercised by
  `tests/fixture_data_field.rs` (decoded fields + byte-exact round-trip +
  truncation rejection + serde).
- Two runnable examples: `build_data_field` (construct + serialize from typed
  fields) and `parse_data_field` (parse the committed fixture + round-trip).
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
