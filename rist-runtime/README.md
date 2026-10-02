# rist-runtime

RIST Simple Profile (VSF TR-06-1:2020) RTCP message types.

Wire-level codecs for the RTCP messages defined (or profiled) by the RIST
Simple Profile specification, built on top of the generic
[`rtcp-packet`](https://crates.io/crates/rtcp-packet) crate
(RFC 3550 §6 SR/RR/SDES/BYE/APP):

- **GenericNack** — RFC 4585 §6.2.1, RTCP Transport-Layer Feedback (PT 205,
  FMT 1). Bitmask-based retransmission request.
- **RangeNack** — RIST-specific RTCP APP (PT 204, subtype 0, name `"RIST"`).
  Range-based retransmission request (TR-06-1 §5.3.2.2).
- **RttEcho** — RTCP APP (PT 204, name `"RIST"`, subtype 2/3). Round-trip
  time measurement (TR-06-1 §5.2.6).
- **RistSenderCompound** / **RistReceiverCompound** — compound RTCP packet
  builders enforcing the RIST §5.2.1 structure. Each carries its leading
**ReportPart** (SR or RR) and any sub-packet it does not model as an
**UnknownPacket** (kept at its wire position), so a compound round-trips
byte-identical.
- **RTCP padding (`P` bit)** — RFC 3550 §6.4.1 padding on the report and on
each sub-packet is validated on parse (a zero or over-running count is an
error) and preserved verbatim on serialize.

All wire types implement the workspace-standard `Parse`/`Serialize` trait
pair with byte-exact round-trip fidelity.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Links the standard library. Without it the crate is `#![no_std]` + `alloc`. |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
