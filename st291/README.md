# st291

[![Crates.io](https://img.shields.io/crates/v/st291.svg)](https://crates.io/crates/st291)
[![docs.rs](https://img.shields.io/docsrs/st291)](https://docs.rs/st291)

SMPTE ST 291-1 — ancillary (ANC) data content. ST 291-1 defines the ANC data
packet: the generic carrier for VANC/HANC payloads (captions, AFD, timecode,
audio metadata, …) multiplexed into a professional video signal. This crate
is about that content, not any one carriage mechanism — ST 291-1 packets can
be conveyed over more than one transport, and this crate grows to cover each
as it is added.

## Transports

- **`ts`** (default) — SMPTE ST 2038:2021 carriage of ANC data packets in an
  MPEG-2 Transport Stream. Implements the two wire structures from §4:
  - **`AncDataDescriptor`** — the `anc_data_descriptor` (tag `0xC4`) in the PMT
    ES loop, plus the `"VANC"` `registration_descriptor` `format_identifier`
    `0x56414E43` and `stream_type` `0x06` (§4.1, Table 1).
  - **`AncDataPacket`** — the ANC data PES packet (`stream_id == 0xBD`, PTS,
    `PES_header_data_length == 0x05`) carrying a list of bit-packed
    `AncPacket` records followed by `0xFF` stuffing (§4.2, Table 2).
- **`rtp`** — RFC 8331 / ST 2110-40 carriage of ANC data packets over RTP
  (issue #648). `AncRtpPayload` is the §2.1 payload (`Extended Sequence
  Number`/`Length`/`ANC_Count`/`F`/`reserved` + a list of `RtpAncPacket`s),
  riding on an `rtp_packet::RtpPacket`'s payload (the RTP fixed header, RFC
  3550, is the `rtp-packet` crate's responsibility). `Length`/`ANC_Count` are
  always recomputed on serialize and cross-validated on parse; the `F` field
  is a `#[non_exhaustive]` `FieldSense` enum whose `Invalid` (`0b01`) variant
  parses successfully rather than being rejected.

The per-ANC-packet `DID`/`SDID`/`data_count`/`user_data_word`/`checksum_word`
content fields are a contiguous **MSB-first 10-bit bit stream**, identical
across both transports (the always-compiled `AncContent` type). Per §4.2.1/
§2.1 the `user_data_word` loop counter uses only the **low 8 bits** of
`data_count`; the full 10-bit values are stored verbatim. ST 291-1
parity/checksum is **not** validated here (deferred to ST 291-1, which is
not vendored).

`#![no_std]` + `alloc`; depends only on `broadcast-common` (plus
`rtp-packet`, optionally, for the `rtp` feature).

## Quick start

```rust
use st291::{AncDataPacket, AncPacket};

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
cargo run -p st291 --example build_anc
cargo run -p st291 --example parse_anc
cargo run -p st291 --example build_anc_rtp --features rtp
cargo run -p st291 --example parse_anc_rtp --features rtp
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `ts`    | yes     | SMPTE ST 2038:2021 MPEG-2 TS transport (`AncDataDescriptor` + `AncDataPacket`). |
| `rtp`   | no      | RFC 8331 / ST 2110-40 RTP transport (`AncRtpPayload` + `RtpAncPacket`). |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
