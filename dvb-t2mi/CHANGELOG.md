# Changelog

## [Unreleased]

## [6.3.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-scte35` crate; dvb-si `TsResync` byte-stream resync helper). No changes to this crate.

## [6.2.0] — 2026-06-13

Workspace minor: the per-PLP inner-TS filter (#146), and the `t2mi_dump` example
absorbed into the new `dvb-tools` binary (#59).

### Added
- **`InnerTsRecovery::new_for_plp` + `filtered_bbframes`** (#146) — optional
  per-PLP filter on the recovery driver; only BBFrames matching the target
  PLP are decoded. `filtered_bbframes()` reports how many were skipped.

### Changed
- The `t2mi_dump` example has moved into the new `dvb-tools` binary crate as
  `dvb-tools t2mi` (#59). The example file and its `[[example]]` manifest
  entry are removed; behaviour with `--pid 0xNNN|raw` is unchanged. The same
  CLI additionally exposes `--inner` (chain-unwrap to inner MPEG-TS via
  `InnerTsRecovery`) and `--plp` (per-PLP filter that uses
  `InnerTsRecovery::new_for_plp`).

## [6.1.0] — 2026-06-12

Version-lockstep release with the workspace. No changes to this crate (the #55a
BbframePump adaptor lives in dvb-bbframe; the chain test now drives it).

## [6.0.0] — 2026-06-11

Lockstep major. Adds the one-call inner-TS recovery driver and decodes the P1
signalling that was exposed as raw bits.

### Added
- **`InnerTsRecovery`** (feature `ts`) — feed outer TS packets from the T2-MI
  PID, get the inner MPEG-TS packets out. Folds `T2miPump` + `Bbheader` +
  `CarryOverExtractor` (NM/HEM mode handling + SYNCD carry-over) into one
  `feed(&[u8]) -> &[[u8; 188]]` driver, so callers stop re-wiring the chain.
  NM and HEM-without-NPD frames are recovered; HEM+NPD is skipped.
- `S1Field::name()` (EN 302 755 Table 18: `T2_SISO`, `T2_MISO`, `Non-T2`, …) and
  an `S2Field1` decode of the P1 S2 field (FFT size + guard-interval set, plus
  the `mixed` flag), now consistent across the FEF null/IQ/composite payloads.

### Changed
- **`dvb-bbframe` is now an optional dependency** enabled by the `ts` feature
  (was a dev-dependency), required by `InnerTsRecovery`. `default = ["ts"]` is
  unchanged, so the default build is unaffected.

## [5.0.0] — 2026-06-11

Lockstep major. The individual-addressing payload is now fully typed, private
packet types are runtime-registrable, and the error model is unified with the
workspace.

### Added
- **`PayloadRegistry`** (#122, #129) — register owned types for `packet_type`
  values not in `PacketType` (or override built-ins) and dispatch through them
  via `AnyPayload::dispatch_with(packet_type, bytes, &reg)` or
  `T2miEvent::payload_with(&reg)`; the result is `AnyPayload::Other { packet_type,
  value }`, downcastable to your concrete type. `PayloadDef` is un-sealed for
  external implementation.
- **Typed individual-addressing payload** (#124): the `individual_addressing`
  packet's transmitter/function loop is fully parsed into typed entries
  (`FunctionBody` etc.) instead of a raw byte slice.

### Changed
- **Unified cross-crate error model** (#112): `Error` is now structured
  `thiserror` variants consistent with the rest of the workspace. (Breaking:
  error type / variants changed.)
- **`AnyPayload` is `#[non_exhaustive]`** with `Other`/`Unknown` fall-through
  arms; growth-prone enums across the crate gained `#[non_exhaustive]`.
  (Breaking for exhaustive matches.)
- **`FunctionBody` serde keys are PascalCase** (dropped `rename_all =
  "camelCase"`) — camelCase is reserved for the top-level dispatch enums.
  (Breaking for serde consumers of that enum.)

### Fixed
- `crc.rs` tests no longer depend on `dvb_common::crc32_mpeg2::TABLE` (now
  `pub(crate)`); they assert against known CRC vectors.
- `#[must_use]` on `PacketReassembler::new`; `FefSubPartPayload` gained the
  `yoke::Yokeable` derive (feature `yoke`).

## [4.3.0] — 2026-06-10

### Added
- **Decoded T2-MI timestamp accessors** (#47): `Bandwidth::subseconds_per_second`
  (the T_sub unit per ETSI TS 102 773 §5.2.7 Table 4),
  `T2TimestampPayload::{is_null, is_relative, emission_offset, set_emission_offset}`.
  `emission_offset` returns the time since the 2000 epoch as a `core::time::Duration`
  (no chrono dependency). Civil-UTC conversion (applying the `utco` leap-second
  offset) is intentionally deferred pending verification against a real capture.

- **Real-capture test** (`tests/real_capture.rs`): a 1.1 MB conformant T2-MI
  slice (Capital TV Colombia, via tsduck.io; PAT+PMT+T2-MI PID, stuffing
  stripped, packet-capped) validates the pump → `AnyPayload` → decoded-timestamp
  path on real broadcast bytes. Fixture excluded from the published crate.

## [4.2.0] — 2026-06-09

Version-lockstep release with the workspace (dvb-si DSM-CC `ModuleReassembler`
hardening, #42 / #43); no changes to this crate.

## [4.1.0] — 2026-06-09

### Fixed
- Crate-root quickstart doctest is gated on the `ts` feature, so
  `cargo test --no-default-features --doc` no longer fails to compile its
  `pump`-based example.

Otherwise version-lockstep with the workspace (`dvb-si` decoded accessors).

## [4.0.0] — 2026-06-08

Version-lockstep release with the workspace (the `dvb-si` 4.0 section/table
split). No functional change to this crate; the crate-root chain diagram now
names `AnyTableSection` instead of `AnyTable` to match the renamed `dvb-si`
dispatcher.

## [3.1.2] — 2026-06-07

Version-lockstep release with the workspace (dvb-si spanning-into-PUSI section fix); no changes to this crate.

## [3.1.1] — 2026-06-07

Version-lockstep release with the workspace (dvb-si `SectionReassembler`
concatenated-section fix); no changes to this crate.

All notable changes to the `dvb_t2mi` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] — 2026-06-05

Lockstep release with the `dvb-si` 3.x line.

### Breaking
- **serde is Serialize-only.** Every `Deserialize` derive is removed from the
  packet header and all payload types (and the now-dead `serde(borrow)` attrs).
  JSON is a display/export format; re-`parse` from wire bytes to reconstruct.
  `Serialize` output is unchanged.

### Added
- **`yoke` feature (off by default, additive).** `yoke::Yokeable` is derived on
  the zero-copy payload view types (`BbframePayload`, `AuxIqPayload`,
  `L1CurrentPayload`, `L1FuturePayload`, `ArbitraryCellsPayload`, `FefIqPayload`,
  `IndividualAddressingPayload` / `FunctionEntry`, and the `AnyPayload` enum) so a
  parsed T2-MI payload can be retained past the input buffer's borrow without a
  re-parse. Optional; adds no dependencies to default builds.

## [2.1.0] — 2026-06-05

### Added
- `AnyPayload::name()` — macro-generated diagnostic label from each type's
  `PayloadDef::NAME` (`"BBFRAME"`, …); `"UNKNOWN"` for the fallthrough
  variant. (#18) `t2mi_dump` now prints it.

## [2.0.0] — 2026-06-05

Version-lockstep release with the workspace (dvb-si 2.0 typed client API).

### Added

- **`payload::AnyPayload`** — macro-generated dispatch enum over all 12 T2-MI packet
  types. `AnyPayload::dispatch(packet_type, payload_bytes)` returns `Some(Result<Self>)`
  for known types, `None` for unknown (caller makes `Unknown { packet_type, body }`).
  Driven by `PayloadDef` trait + `declare_payloads!` macro; drift-tested against the
  `PACKET_TYPE` constants.
- **`pump::T2miPump`** (feature `ts`) — feed-and-iterate pump over a T2-MI TS stream.
  `T2miPump::new(pid)` + `feed_ts(&[u8])` for encapsulated streams; `feed_raw(&[u8])`
  for bare T2-MI. Per-packet CRC-32 checked; failures dropped and counted. Stats:
  `ts_packets`, `t2mi_packets`, `crc_failures`, `malformed_packets`.
- **`pump::T2miEvent`** — owning-`Bytes` event: `.header() -> Result<Header>`,
  `.payload() -> Result<AnyPayload<'_>>` (lazy, borrows event bytes).
- **`examples/t2mi_dump.rs`** — CLI pump tool: `cargo run -p dvb-t2mi --example
  t2mi_dump -- file.ts`. Prints one line per T2-MI packet with header fields and
  payload type.
- **Chain test** (`tests/chain.rs`) — T2miPump → `AnyPayload::BbFrame` →
  `bbframe::up_iter` → `SiDemux` → at least one typed `AnyTable`.

## [1.1.0] — 2026-06-04

Version-lockstep release with the workspace (dvb-si: complete EN 300 468
Table 12 descriptor coverage); doc-comment formatting/link cleanup only,
no functional changes.

## [1.0.1] — 2026-06-04

Version-lockstep release with the workspace (dvb-si README overhaul); no code changes.

## [1.0.0] — 2026-06-04

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
- `PacketReassembler::feed` drops the unused `pid` parameter (demux upstream,
  one reassembler per PID) and now treats `pointer_field` as authoritative:
  a buffered partial whose declared length over-ran (corrupt
  `payload_len_bits` / lost TS packets) is discarded at the next PUSI instead
  of swallowing the packets that follow; a pointer past the payload end drops
  sync until the next PUSI.
- Added `Header::payload_bytes(&self, packet)` — slices the declared payload
  out of the full packet buffer (errors with `PayloadLengthMismatch` on
  truncation).
- Serialize-bounds consistency: `p2_bias` (15-bit field) and
  `fef_null`/`fef_iq` `s2_field` (4-bit) now error on over-range values
  instead of silently masking, matching every other bounded field.

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
