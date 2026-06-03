# Changelog

All notable changes to the `dvb_t2mi` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `serde` feature flag — optional `Serialize`/`Deserialize` derives on the T2-MI
  header and all 12 payload types (off by default).

### Fixed

- `IndividualAddressingPayload` (§5.2.8) now matches Fig 11: top level is
  `rfu(8) · individual_addressing_length(8) · individual_addressing_data(var)`;
  `tx_identifier` lives inside each per-transmitter entry of the data loop, not
  at the top. The redundant length field was dropped (derived on serialize).
- `FefSubPartPayload` invalid `subpart_variety` now reports `ReservedBitsViolation`
  instead of truncating the 16-bit value into an `InvalidPacketType` u8.

## [0.1.0] — 2026-04-20

Initial release. Complete implementation of ETSI TS 102 773 v1.4.1 DVB-T2 Modulator Interface.

### Added

- **Header parsing** — 6-byte T2-MI header with `PacketType` enum covering all 12 defined types (0x00, 0x01, 0x02, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33)
- **Payload parsers** — every payload type with `Parse` + `Serialize` round-trip tests:
  - `BbframePayload` (§5.2.1) — Baseband Frame with `frame_idx`, `plp_id`, `intl_frame_start`
  - `AuxIqPayload` (§5.2.2) — Auxiliary I/Q data with 12-bit two's complement samples
  - `ArbitraryCellsPayload` (§5.2.3) — Arbitrary cell insertion with 22-bit cell address
  - `L1CurrentPayload` (§5.2.4) — L1-current signalling with `FrequencySource` enum
  - `L1FuturePayload` (§5.2.5) — L1-future signalling
  - `P2BiasPayload` (§5.2.6) — P2 bias balancing cells
  - `T2TimestampPayload` (§5.2.7) — DVB-T2 timestamps with `Bandwidth` enum, `null_timestamp` detection
  - `IndividualAddressingPayload` (§5.2.8) — Per-transmitter addressing functions (ACE-PAPR, MISO, TR-PAPR, L1-ACE-PAPR, TX-SIG, frequency)
  - `FefNullPayload` (§5.2.9) — FEF Null with `S1Field` enum (V0–V7)
  - `FefIqPayload` (§5.2.10) — FEF I/Q data
  - `FefCompositePayload` (§5.2.11) — FEF composite with `num_subparts`
  - `FefSubPartPayload` (§5.2.12) — FEF sub-parts with `SubpartVariety` (Null/IQ/PRBS/TX-SIG), `PrbsType`
- **CRC-32** — MPEG-2 Annex A polynomial (0x04C1_1DB7) with precomputed 256-entry table, `crc32()` and `validate_crc()` functions
- **TS reassembler** — MPEG-2 TS decapsulation per §6.1.1 with `PacketReassembler`, handling PUSI + `pointer_field`, packet spanning, and multi-packet extraction
- **Error handling** — `Error` enum with `thiserror` for all failure modes (buffer too short, reserved bits, CRC mismatch, invalid packet type, output buffer too small, payload length mismatch)
- **Trait contracts** — `Parse<'a>` and `Serialize` traits with `Result<T>` alias

### Fixed

- Corrected `t2mi_stream_id` bit position (byte 2 [2:0], not byte 3)
- Corrected byte 3 as 8-bit RFU (not 4-bit with `t2mi_stream_id`)
- Fixed `fef_composite` layout: s1_field at byte 1 [6:4], s2_field at [3:0], total 8 bytes per Figure 15
- Fixed `fef_subpart` layout: rfu2 is 10 bits (byte 11 + byte 12 [7:6]), subpart_length is 22 bits (byte 12 [5:0] + byte 13 + byte 14), total 15-byte header
- Fixed `l1_current` RFU check: bottom 6 bits of byte 1 (not top 2)

### Known Limitations

- **BBHeader parsing** — the 10-byte DVB-T2 BBHEADER (MATYPE, UPL, DFL, SYNCD, ISSYI, CRC-8) is defined in EN 302 755, not in this crate. The `BbframePayload` returns raw bytes; consumers parse the BBHEADER themselves.
- **HEM detection** — High Efficiency Mode detection (CRC-8 init XOR) and user packet extraction (187 vs 188 byte stride) are adaptor-layer concerns, not part of the T2-MI protocol.
- **L1 signalling interpretation** — L1-current and L1-future payloads are passed through as raw bytes. Full L1 parsing requires EN 302 755 clause 7.2 implementation.
- **No `no_std` support** — the `ts` module requires `bytes::BytesMut` (std heap allocation). Core types (`packet`, `payload`, `crc`) could be `no_std` with minor changes.

### Spec References

All implementations follow ETSI TS 102 773 v1.4.1 (2016-03). The authoritative spec is available from [etsi.org](https://www.etsi.org/deliver/etsi_ts/102700_102799/102773/).
