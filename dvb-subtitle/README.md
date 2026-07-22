# dvb‑subtitle

DVB subtitling (bitmap) segment parser and serializer — **ETSI EN 300 743 V1.6.1**.

Feed it the reassembled PES data field of a DVB subtitle stream (the payload
from a PES packet with `stream_id` signalling private-data subtitling); it
returns typed, decoded segments. It depends only on
[`broadcast‑common`](https://crates.io/crates/broadcast-common) and works `#![no_std]`
(+ `alloc`).

## Features

- **Every segment type from §7.2** — display definition, page composition,
  region composition, CLUT definition, object data (incl. 2/4/8-bit pixel‑data
  sub‑blocks, character strings, and progressive zlib‑compressed pixel blocks),
  disparity signalling, alternative CLUT, end of display set, and stuffing.
- **Symmetric `Parse<'a>` / `Serialize`** with round‑trip tests — parse then
  serialize is byte‑identical, and serialize then parse compares equal.
- **Unified dispatch** via `AnySegment`: feed the raw PES payload, get back an
  enum that you pattern‑match on.
- **`#![no_std]`** with optional `std` (linking `thiserror/std`).  Serde
  support is behind the `serde` feature.
- **Re‑exports** every public segment type and its component enums, so you
  only ever need `use dvb_subtitle`.

## Examples

Two runnable examples ship with this crate
(`cargo run -p dvb-subtitle --example <name>`).

### `parse_segment`

```rust,ignore
```include_str!("../examples/parse_segment.rs")
```

### `parse_full_pes`

```rust,ignore
```include_str!("../examples/parse_full_pes.rs")
```

## Usage

```rust
use broadcast_common::Parse;
use dvb_subtitle::{PesDataField, AnySegment, DataIdentifier, EndOfPesMarker, SyncByte};

let bytes = [
    DataIdentifier,               // 0x20
    0x00,                         // subtitle_stream_id = 0x00
    SyncByte, 0x80, 0x00, 0x01,   // end_of_display_set segment
    0x00, 0x00,
    EndOfPesMarker,               // 0xFF
];
let field = PesDataField::parse(&bytes).unwrap();
for seg in &field.segments {
    println!("{}", seg.name());
}
```

## Licence

MIT OR Apache‑2.0
