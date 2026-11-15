# mpeg-pes 0.9.0

_Released 2026-11-15._

Minor-class release (0.8 -> 0.9) that makes the PES header honest about what it carries: `PES_packet_length` is derived from the content instead of stored, timestamps and scrambling are validated values, and `PesPacket::parse` now reports a truncated packet rather than silently returning a short one. The lenient entry a transport-stream joiner needs is `PesPacket::parse_unit`.

Version: 0.8.0 -> 0.9.0.

## Breaking changes and migration

### `PesPacket::parse` is strict: use `parse_unit` for a wire-assembled unit

`PesPacket::parse` reports a buffer shorter than the declared `PES_packet_length` as `Error::BufferTooShort` instead of a silent short read. A unit assembled from a transport stream ends where the next `payload_unit_start_indicator` says, which a broadcast encoder may declare longer than the bytes that arrived, so a join carries such units. `PesPacket::parse_unit` accepts them and returns the packet plus a `Truncation { declared, available }` (`None` when complete); the payload is exactly the bytes present, never padded.

```rust
// before
let packet = PesPacket::parse(&unit)?;                    // now errors on a short unit
// after (strict, for a complete packet)
let packet = PesPacket::parse(&complete)?;
// after (a unit that may be shorter than declared)
let (packet, truncation) = PesPacket::parse_unit(&unit)?;
if truncation.is_some() { /* the join cut it short */ }
```

### `PesPacket` derives `PES_packet_length` and is built with `new`

The `pes_packet_length` field is gone. `serialize_into` derives it from the content, writes `0` (unbounded) only for video and only when the content passes the 16-bit ceiling, and returns `Error::FieldOverflow` for an audio or special-stream packet that cannot be bounded. `PesPacket` is `#[non_exhaustive]`, built with `PesPacket::new(stream_id, payload)` and with `header` set on the result.

```rust
// before
let packet = PesPacket { header, stream_id, payload, pes_packet_length: 0 };
// after
let mut packet = PesPacket::new(stream_id, payload);
packet.header = header;
```

### `Pts`/`Dts` hold a validated `Clock33`

They carry `broadcast_common::clock33::Clock33` instead of an unchecked `u64`: construct with `Pts::try_new(ticks)`/`Dts::try_new(ticks)` (or `From<Clock33>`), and read with `ticks()`. An out-of-range 33-bit value is rejected instead of silently wrapping modulo 2^33. `seconds()` stays a display helper.

```rust
// before
let pts = Pts(90_000);
let ticks = pts.0;
// after
let pts = Pts::try_new(90_000)?;
let ticks = pts.ticks();
```

### `StreamId`'s field is private

Build one from the wire byte with `StreamId::new` and read the byte back with `StreamId::get`; the named associated constants are unchanged. The tuple field let a caller bypass the type when converting to and from the wire (wire bytes are unchanged).

```rust
// before
let id = StreamId(0xE0);
let byte = id.0;
// after
let id = StreamId::new(0xE0);
let byte = id.get();
```

### The optional header's fields are typed

`PesHeader::scrambling_control` is the `ScramblingControl` enum (2-bit, with a `Reserved(u8)` arm) instead of a raw `u8`, and `PesHeader::es_rate` is `EsRate` (a checked 22-bit newtype) instead of `u32`. `Escr::to_field_bytes`, `Escr::from_27mhz` and `TrickMode::to_byte` return `Result` (an over-wide base, extension, reserved run or sub-field is rejected rather than masked), `Escr::to_clock27` reports an extension off the clock, and `program_packet_sequence_counter`, the `ES_rate` marker bits and `additional_copy_info` are range-checked instead of masked.

```rust
// before
let header = PesHeader { scrambling_control: 0, es_rate: Some(25_000), .. };
// after
let header = PesHeader { scrambling_control: ScramblingControl::NotScrambled, es_rate: Some(EsRate::try_new(25_000)?), .. };
```

## Fixes

- `PesPacket::parse` no longer silently returns a short packet for a buffer shorter than its declared `PES_packet_length`: it is reported as `Error::BufferTooShort` (a regression from the change that stopped re-serializing past the declared length).
- A serializer can no longer write a `PES_packet_length` that disagrees with the bytes it writes, because the field is derived.
- The PTS/DTS/ESCR readers build their fixed arrays directly instead of `.try_into().unwrap()`/`.expect("length checked above")`; the `panic_idioms` guard now covers the crate.

## Added

- `PesPacket::parse_unit` and `Truncation`, for a PES unit that may end before its declared `PES_packet_length`.
- `PesHeader::flag_bytes`, `PesHeader::serialize_optional_fields` and a public `PesHeader::optional_len`: the one implementation of the PES optional-header bytes, so a crate that frames its own PES payload can build on this codec instead of re-implementing the timestamp coding. `PesPacket::serialize_into` calls them; its output is unchanged.
- `StreamId::unbounded_length_allowed`: whether the unbounded `0` form may be written for that stream (video).

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `mpeg-pes-v0.9.0`.
