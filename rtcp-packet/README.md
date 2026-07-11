# rtcp-packet

[![Crates.io](https://img.shields.io/crates/v/rtcp-packet.svg)](https://crates.io/crates/rtcp-packet)
[![docs.rs](https://img.shields.io/docsrs/rtcp-packet)](https://docs.rs/rtcp-packet)

RFC 3550 §6 RTCP control packets — SR, RR, SDES, BYE, APP, and the §6.1
compound packet — spec-complete parse/serialize, `no_std`.

`rtcp-packet` is a shared low-level wire crate (in the same spirit as
`rtp-packet`/`mpeg-ts`/`mpeg-pes`/`mpeg-ps`): a single, spec-complete
implementation of the RTCP control-packet codec meant to be consumed by
every higher-level crate that speaks RTP/RTCP (`transmux`'s RTCP module
today), instead of each crate re-implementing its own codec. It was
extracted unchanged from `transmux::rtcp`.

- **[`SenderReport`]** (SR, §6.4.1, PT 200) / **[`ReceiverReport`]** (RR,
  §6.4.2, PT 201) / **[`ReportBlock`]** — the shared 24-byte reception
  report block, including the §6.4.1 24-bit **signed** `cumulative_lost`.
- **[`SourceDescription`]** / **[`SdesChunk`]** / **[`SdesItem`]** /
  **[`SdesItemType`]** (SDES, §6.5, PT 202) — CNAME/NAME/EMAIL/PHONE/LOC/
  TOOL/NOTE/PRIV item types, 32-bit chunk padding.
- **[`Bye`]** (§6.6, PT 203) — SSRC/CSRC list + optional reason text.
- **[`App`]** (§6.7, PT 204) — subtype, SSRC, 4-byte ASCII name,
  application-dependent data.
- **[`RtcpPacket`]** / **[`RtcpPacketType`]** — the PT-byte dispatch enum.
- **[`CompoundPacket`]** (§6.1) — a sequence of RTCP packets that must begin
  with SR or RR, with byte-exact round-trip across the whole compound.

See `docs/rtcp.md` for the curated RFC 3550 §6 transcription this crate
implements field-for-field, including two documented decode-completeness
gaps (SR/RR profile-specific extensions; the SDES PRIV item's internal
`prefix`/`value` sub-structure) that are not separately typed.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use rtcp_packet::{ReportBlock, SenderReport};

let sr = SenderReport {
    ssrc: 0x1122_3344,
    ntp_msw: 0xE0E1_E2E3,
    ntp_lsw: 0x1020_3040,
    rtp_timestamp: 0x0009_0000,
    packet_count: 4321,
    octet_count: 999_999,
    report_blocks: vec![ReportBlock {
        ssrc: 0xAAAA_AAAA,
        fraction_lost: 12,
        cumulative_lost: -3,
        ext_highest_seq: 0x0001_2345,
        jitter: 500,
        lsr: 0xAABB_CCDD,
        dlsr: 0x0000_1000,
    }],
};
let mut bytes = vec![0u8; sr.serialized_len()];
sr.serialize_into(&mut bytes).unwrap();
assert_eq!(SenderReport::parse(&bytes).unwrap(), sr);
```

## Examples

```sh
cargo run -p rtcp-packet --example build_sender_report
cargo run -p rtcp-packet --example parse_compound_packet
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
