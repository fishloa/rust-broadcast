# mpeg-pes 0.9.0

_Released 2026-11-15._

### Changed
- BREAKING: `StreamId`'s field is private: build one from the wire byte with `StreamId::new` and read the byte back with `StreamId::get` (the named associated constants are unchanged). The tuple field let a caller bypass the type when converting to and from the wire; wire bytes are unchanged (#2768). `Pts`/`Dts` (validated `Clock33`, `try_new`/`ticks()`) already carry their 33-bit value through `broadcast_common::clock33` and kept their `seconds()` convenience, which is a display helper, not a wire field.
- The `panic_idioms` workspace guard now covers this crate; the PTS/DTS/ESCR readers that converted a length-checked slice with `.try_into().unwrap()` / `.expect("length checked above")` build the fixed array directly instead (#2773).
### Fixed
- `PesPacket::parse` reports a buffer shorter than the declared `PES_packet_length` as `Error::BufferTooShort` (a truncated packet) instead of a silent short read (regression from the #1159 fix that stopped re-serializing past the declared length) (#2746).
### Added
- `PesPacket::parse_unit` — the lenient entry for a PES unit that may end before its declared `PES_packet_length` (a transport-stream join), returning the packet plus a `Truncation { declared, available }` (or `None` when complete). `PesPacket::parse` stays strict. `Truncation` is exported (#2746).
- `PesHeader::flag_bytes`, `PesHeader::serialize_optional_fields` and a public `PesHeader::optional_len`: the one implementation of the PES optional-header bytes (flag bytes, PTS/DTS, ESCR, ES_rate, trick mode, copy info, CRC, extension, stuffing), so a crate that frames its own PES payload (e.g. `st291`'s ANC PES) can build on this codec instead of re-implementing the timestamp coding (#2762). `PesPacket::serialize_into` now calls them; its output is unchanged (every round-trip test stays byte-identical).
- `StreamId::unbounded_length_allowed` — whether the unbounded `0` form may be written for that stream (video).
### Changed
- The test helpers that walk real captures (`fixture_pes.rs`, `dvb-subtitle`'s `support::data_fields`) skip a unit whose declared `PES_packet_length` runs past the captured bytes instead of asserting it parses: a capture that joins mid-PES (the committed `m6-single.ts`) carries such units, and the parser now reports them (#2746).
- BREAKING: `PesPacket` no longer has a `pes_packet_length` field. `serialize_into` derives `PES_packet_length` from the content, writes `0` (unbounded) only for video and only when the content passes the 16-bit ceiling, and returns `Error::FieldOverflow` for an audio or special-stream packet that cannot be bounded (#2746).
- BREAKING: `PesPacket` is `#[non_exhaustive]`; build one with the new `PesPacket::new(stream_id, payload)` and set `header` on the result (#2746).
- BREAKING: `Pts`/`Dts` now hold the validated `broadcast_common::clock33::Clock33` rather than an unchecked `u64`. Construct with `Pts::try_new(ticks)` (or `Dts::try_new`), or `From<Clock33>`; the field is private, read it with `ticks()`. An out-of-range 33-bit value is rejected instead of silently wrapping modulo 2^33 (#2747).
- BREAKING: `PesHeader::scrambling_control` is now the `ScramblingControl` enum (2-bit, with a `Reserved(u8)` arm) instead of a raw `u8`; `PesHeader::es_rate` is now `EsRate` (a checked 22-bit newtype) instead of `u32`; `Escr::to_field_bytes` returns `Result` (rejects a base wider than 33 bits, an extension wider than 9 bits, or reserved wider than 2 bits); `Escr::from_27mhz` returns `Result` and `Escr::to_clock27` reports an extension off the clock; `TrickMode::to_byte` returns `Result` (rejects an over-width sub-field); `program_packet_sequence_counter`, `ES_rate` marker bits and `additional_copy_info` are range-checked instead of masked (#2747).

---

Published from tag `mpeg-pes-v0.9.0`.
