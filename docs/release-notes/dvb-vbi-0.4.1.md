# dvb-vbi 0.4.1

_Released 2026-10-05._

### Added
- `TeletextDataField::txt_data_block_logical()` (issue #1106): a typed
  accessor returning `txt_data_block` bit-reversed into EN 300 706's own
  logical byte order (§7.1.2/§8.1/§8.2), promoting the doc-only note added
  for issue #1041 into a real, tested method — raw wire access via
  `txt_data_block` is unchanged. `VpsDataField`/`WssDataField`/
  `ClosedCaptioningDataField` gained doc notes on the same question: EN 301
  775 §4.7.2/§4.8.2 state WSS/CC bits are already in logical order (no
  reversal), and this crate does not vendor EN 300 231 so makes no bit-order
  claim for VPS.

### Fixed
- Documented (issue #1041) that `TeletextDataField::txt_data_block` bytes
  are carried in ordinary (MSB-first) byte order, which is the
  bit-reversal of the EN 300 706 byte values any Teletext decoder (e.g.
  `timed-metadata`) runs Hamming-8/4 / odd-parity against — no code change,
  this crate never decodes EN 300 706 itself.

---

Published from tag `dvb-vbi-v0.4.1`.
