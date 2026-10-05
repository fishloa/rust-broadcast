# mpeg-ts 0.5.0

_Released 2026-10-05._

**Minor-epoch breaking release (0.x).** Three API changes require action: `OwnedTsPacket`'s header fields are now private (use the getters), `OwnedTsPacket::serialize_with_payload` returns a `Result`, and the crate requires `broadcast-common` 9.4. Several framing and parsing bugs are fixed, the most visible being `mux::SiMux` silently dropping one of two tables that share a PID. Because the caret epoch moves from 0.4 to 0.5, every crate that depends on `mpeg-ts` must move together: see [dvb-si 11.0.0](dvb-si-11.0.0.md), [dvb-conformance 11.0.0](dvb-conformance-11.0.0.md), [dvb-tools 11.0.0](dvb-tools-11.0.0.md), [mpeg-ps 0.5.0](mpeg-ps-0.5.0.md) (its `mpeg-pes` dependency) and [broadcast-common 9.4.0](broadcast-common-9.4.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
```

No feature or MSRV change. Diagnose a mixed graph with `cargo tree -i mpeg-ts`; two versions in the output means a stale pin.

## Breaking changes

### 1. `OwnedTsPacket` fields are private (audit r01-W8, #1074)

`raw`, `pid`, `pusi`, `has_adaptation`, `has_payload`, `tei`, `scrambling` and `continuity_counter` are no longer public. A public copy of a header field that a caller could assign drifted silently from `raw`, which is what actually serialises. Read access is through getters of the same names; `discontinuity` stays public because it is caller metadata, not a copy of wire bytes. To change wire bits, use the associated functions that operate on the raw packet (for example `OwnedTsPacket::set_pcr`) rather than assigning a field.

```rust
// before
let pid = pkt.pid;
let bytes: [u8; 188] = pkt.raw;
// after
let pid = pkt.pid();
let bytes: &[u8; 188] = pkt.raw();     // borrowed; use pkt.into_raw() for the owned array
```

### 2. `OwnedTsPacket::serialize_with_payload` returns `Result` (#1129)

It used to truncate silently with `payload.len().min(184)`. It now returns `Result<[u8; TS_PACKET_SIZE], Error>` and rejects a payload over the new `OwnedTsPacket::MAX_PAYLOAD_LEN` (184) with the new `Error::PayloadTooLarge`.

```rust
// before
let raw = pkt.serialize_with_payload(&payload);
// after
let raw = pkt.serialize_with_payload(&payload)?;
```

The other new `Error` variants are `FieldOverflow` (wrapping `broadcast_common::len::FieldOverflow`) and `SectionPayloadLengthMismatch`; `Error` is `#[non_exhaustive]`, so a wildcard arm already covers them.

## Behaviour changes that are not API breaks

- `TsPacket::parse` now rejects an adaptation field whose declared `adaptation_field_length` does not fit the packet. It used to clamp it with `.min()`, while `OwnedTsPacket::adaptation_field` already returned `None` for the same bytes (#1074). Input that parsed before may now be an error.
- `OwnedTsPacket::set_pcr` now requires `adaptation_field_length >= 7` (flags byte plus the 6-byte PCR). A malformed field that had the PCR flag set but a shorter declared length let it write past the adaptation field into the payload (#1074).
- `Section::serialize_into` validates that `payload` length and the settable `section_length` agree, and returns `Error::SectionPayloadLengthMismatch` instead of panicking (oversized payload) or emitting a short, wrong-CRC section (undersized payload) (#1074).
- `PusiReassembler` is now a PID filter over `broadcast_common::pusi::PusiAccumulator`, the accumulator `mpeg-pes::PesAssembler` also uses. One observable difference: a PUSI that closes an empty in-progress unit no longer returns `Some(vec![])` (audit r01-W13, #1074). It also re-exports `DEFAULT_MAX_UNIT_SIZE` from `broadcast_common::pusi`.
- `SectionPacketiser::packetise_into` finds the next section start with a forward-only cursor; output is unchanged (differential round-trip test over 400 boundary-straddling sections) (audit r01-O1).

## Fixes

- `mux::SiMux` keyed scheduler entries by PID alone, so two tables sharing a PID by spec (TDT/TOT on `0x0014`, SDT/BAT on `0x0011`, EIT present/following vs schedule on `0x0012`) collided: `upsert_tot` after `upsert_tdt` silently replaced the TDT, which never appeared in the output. Entries are now keyed by `(pid, table_id)`, and all entries on one PID share one `SectionPacketiser`, so one continuity counter stays continuous across them. Verified against TSDuck's `tsp -P tables` and `tsanalyze` (`mpeg-ts/tests/tsduck_simux_oracle.rs`) (#1000).
- `SectionPacketiser::packetise_into` could emit PUSI=1 with `pointer_field == 183` when a section tail exactly filled a packet's PUSI payload capacity, a pointer past the packet's own payload (H.222.0 §2.4.4). A related missing bound let a non-PUSI continuation packet at the same boundary copy one byte past the next section's start, which the reassembler then dropped, corrupting that next section (#1074).
- `AdaptationField::serialize_into` wrote `transport_private_data_length` with an unchecked `as u8`, wrapping for more than 255 bytes while the whole data was still copied (#1129).

## Internal

Named the remaining flag-bit and adaptation-field-extension masks (`CURRENT_NEXT_MASK`, `PCR_RESERVED_BITS`), replaced hand-masked low-byte splits with `to_be_bytes`, and de-duplicated the 12-bit `section_length` decode (one `ts::section_total_len` helper) and the adaptation-field skip walk (one `adaptation_field_skip` helper, with `extract_ts_payload` reusing `TsHeader::parse`) (#1074). Audit items r01-O2 and r01-O4 were deliberately not done (no measured hot path; O4 would add public API speculatively).

---

Published from tag `mpeg-ts-v0.5.0`.
