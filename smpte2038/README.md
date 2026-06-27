# smpte2038

[![Crates.io](https://img.shields.io/crates/v/smpte2038.svg)](https://crates.io/crates/smpte2038)
[![docs.rs](https://img.shields.io/docsrs/smpte2038)](https://docs.rs/smpte2038)

SMPTE ST 2038:2021 — carriage of ANC (SMPTE ST 291-1 ancillary) data packets in
an MPEG-2 Transport Stream.

Implements the two wire structures from §4:

- **`AncDataDescriptor`** — the `anc_data_descriptor` (tag `0xC4`) in the PMT ES
  loop, plus the `"VANC"` `registration_descriptor` `format_identifier`
  `0x56414E43` and `stream_type` `0x06` (§4.1, Table 1).
- **`AncDataPacket`** — the ANC data PES packet (`stream_id == 0xBD`, PTS,
  `PES_header_data_length == 0x05`) carrying a list of bit-packed `AncPacket`
  records followed by `0xFF` stuffing (§4.2, Table 2).

The per-ANC-packet `DID`/`SDID`/`data_count`/`user_data_word`/`checksum_word`
fields are a contiguous **MSB-first 10-bit bit stream**. Per §4.2.1 the
`user_data_word` loop counter uses only the **low 8 bits** of `data_count`; the
full 10-bit values are stored verbatim. ST 291-1 parity/checksum is **not**
validated here (ST 2038 defers it to ST 291-1, which is not vendored).

`#![no_std]` + `alloc`; depends only on `dvb-common`.

## Quick start

```rust
use smpte2038::{AncDataPacket, AncPacket};

let pkt = AncDataPacket {
    pes_priority: false,
    copyright: false,
    original_or_copy: false,
    pts: 90_000,
    anc_packets: vec![AncPacket {
        c_not_y_channel_flag: false,
        line_number: 9,
        horizontal_offset: 0,
        did: 0x161,
        sdid: 0x101,
        data_count: 0x002,
        user_data_words: vec![0x2CF, 0x101],
        checksum: 0x233,
    }],
    stuffing_bytes: 0,
};
let mut bytes = vec![0u8; pkt.serialized_len()];
pkt.serialize_into(&mut bytes).unwrap();
assert_eq!(AncDataPacket::parse(&bytes).unwrap(), pkt);
```

## Examples

```sh
cargo run -p smpte2038 --example build_anc
cargo run -p smpte2038 --example parse_anc
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.81

## License

MIT OR Apache-2.0
