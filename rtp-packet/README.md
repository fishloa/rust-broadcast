# rtp-packet

[![Crates.io](https://img.shields.io/crates/v/rtp-packet.svg)](https://crates.io/crates/rtp-packet)
[![docs.rs](https://img.shields.io/docsrs/rtp-packet)](https://docs.rs/rtp-packet)

RFC 3550 §5.1 RTP fixed header + CSRC list + §5.3.1 generic header extension —
spec-complete parse/serialize, `no_std`.

`rtp-packet` is a shared low-level wire crate (in the same spirit as
`mpeg-ts`/`mpeg-pes`/`mpeg-ps`): a single, spec-complete implementation of the
RTP fixed header meant to be consumed by every higher-level crate that speaks
RTP (`transmux`'s RTP spoke today; the upcoming SMPTE ST 2110-40 ANC-over-RTP
work next), instead of each crate re-implementing its own header codec.

- **[`RtpPacket`]** — version (validated `== 2` on parse, always written `2`
  on serialize), padding (the trailing pad-count octet correctly
  stripped/re-appended), the CSRC identifier list (0–15 entries, `CC` always
  derived from the list length), marker, payload type, sequence number,
  timestamp, SSRC, an optional [`HeaderExtension`], and the payload.
- **[`HeaderExtension`]** — the §5.3.1 generic header extension: a 16-bit
  profile-specific identifier + opaque profile-specific data (the `length`
  field, in 32-bit words, is always derived from the data length).

See `docs/rtp-header.md` for the curated RFC 3550 §5.1/§5.3.1 transcription
this crate implements field-for-field.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

let pkt = RtpPacket {
    marker: true,
    payload_type: 96,
    sequence_number: 1,
    timestamp: 3600,
    ssrc: 0x1234_5678,
    csrc: vec![],
    extension: None,
    padding: None,
    payload: &[0xDE, 0xAD, 0xBE, 0xEF],
};
let mut bytes = vec![0u8; pkt.serialized_len()];
pkt.serialize_into(&mut bytes).unwrap();
assert_eq!(RtpPacket::parse(&bytes).unwrap(), pkt);
```

## Examples

```sh
cargo run -p rtp-packet --example build_packet
cargo run -p rtp-packet --example parse_packet
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
