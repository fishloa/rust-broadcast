# rmt-flute 0.10.0

_Released 2026-11-15._

Minor-class release (0.9 -> 0.10) that types the LCT identifiers and decodes the NORM command bodies once at parse: the transport session/object identifiers carry the width the flags give them (instead of raw borrowed bytes), a NAK loss list and a handshake's extension block are validated when they are parsed, and a TSI/TOI wider than eight bytes serializes instead of panicking.

Version: 0.9.0 -> 0.10.0.

## Breaking changes and migration

### `LctHeader::tsi`/`toi` are `Tsi`/`Toi`

They carry the decoded identifier and the field width the `S`/`O`/`H` flags give it, instead of raw borrowed bytes; `Tsi::new`/`Toi::new` reject a width the flags cannot describe and a value the width cannot hold.

```rust
// before
let tsi: &[u8] = &header.tsi;
let value = u64::from_be_bytes(/* pad the borrowed slice */);
// after
let tsi: Tsi = header.tsi;
let value = tsi.value();
let built = Tsi::new(1234, 4)?;
```

### `fec_payload_id` is `FecPayloadId`

It is a `FecPayloadId` in `AlcPacket`, `NormData` and `NormCmdBody::{Flush, Squelch}`, replacing the borrowed `&[u8]`: the value owns its bytes and reports its own length, so serialization no longer pairs a slice with a separately supplied length. `NormCmdBody` loses its lifetime parameter along with the borrowed field.

```rust
// before
let flush = NormCmdBody::Flush { fec_id, object_transport_id, fec_payload_id: &bytes };
// after
let flush = NormCmdBody::Flush { fec_id, object_transport_id, fec_payload_id: FecPayloadId::new(&bytes) };
```

### `SourceBlockPartition` fields are private

Read them with the `transfer_length()`/`encoding_symbol_length()`/`source_symbols()`/`num_blocks()`/`larger_block_len()`/`smaller_block_len()`/`larger_blocks()` accessors.

```rust
// before
let symbols = partition.source_symbols;
// after
let symbols = partition.source_symbols();
```

## Fixes

- A TSI/TOI wider than eight bytes (for example a fourteen-byte TOI, `O=3`, `H=1`) serializes big-endian with its leading zero bytes instead of panicking on an out-of-range offset.
- `LctHeader` rejects a version other than `LCT_VERSION` (1) on both parse and serialize with `Error::UnsupportedVersion` (RFC 5651 §5.1).
- `SourceBlockPartition::new` reports `Error::FieldOverflow` when the partition exceeds what a `FecPayloadId128` can address (more than `u32::MAX` source blocks, or more than `u16::MAX` symbols in a block) instead of producing a partition that cannot be addressed.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `rmt-flute-v0.10.0`.
