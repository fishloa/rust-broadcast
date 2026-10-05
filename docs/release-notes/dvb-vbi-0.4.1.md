# dvb-vbi 0.4.1

_Released 2026-10-05._

Additive patch release: one new accessor for Teletext data and clearer documentation of bit order for every VBI data unit. No existing API or behaviour changes and nobody must act. If you decode EN 300 706 Teletext from the `txt_data_block` of a `TeletextDataField`, read the bit-order note below, because applying Hamming-8/4 or odd-parity to the raw field gives wrong results.

## New API

- `TeletextDataField::txt_data_block_logical() -> [u8; TXT_DATA_BLOCK_LEN]` (#1106) returns `txt_data_block` with each byte bit-reversed into EN 300 706's own logical order (EN 300 706 sections 7.1.2, 8.1, 8.2). Apply Hamming-8/4, odd parity and character-code tables to this value, not to the raw field. The raw `txt_data_block` field is unchanged, so serialization and the round trip are unaffected. The crate's own proof of the wire order is that `FRAMING_CODE_EBU` (`0xE4`) is `0x27.reverse_bits()`, the spec's framing code constant.

## Documentation

- **Why the raw bytes look reversed (#1041).** `TeletextDataField::txt_data_block` carries bytes in ordinary MSB-first order, which is the bit-reversal of the byte values any EN 300 706 decoder checks. This crate never decodes EN 300 706 itself, so there is no behaviour change; the note records the pitfall that `txt_data_block_logical()` now removes.
- `VpsDataField`, `WssDataField` and `ClosedCaptioningDataField` gained doc notes on the same question: EN 301 775 sections 4.7.2 and 4.8.2 state WSS and Closed Captioning bits are already in logical order (no reversal needed). The crate does not vendor EN 300 231, so it makes no bit-order claim for VPS.

## Related

`timed-metadata-0.6.0.md`: its WebVTT Teletext path now calls `txt_data_block_logical()`, so it needs this release.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). No other `Cargo.toml` change.

---

Published from tag `dvb-vbi-v0.4.1`.
