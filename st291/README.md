# st291

[![Crates.io](https://img.shields.io/crates/v/st291.svg)](https://crates.io/crates/st291)
[![docs.rs](https://img.shields.io/docsrs/st291)](https://docs.rs/st291)

SMPTE ST 291-1 ancillary (ANC) data packets, parsed and serialized as typed
Rust structs for their two transports: MPEG-2 Transport Stream carriage
(SMPTE ST 2038:2021) and RTP carriage (RFC 8331 / SMPTE ST 2110-40).

## What it does / what it does not

ST 291-1 defines the ANC data packet: the generic carrier for VANC/HANC
payloads (captions, AFD, timecode, audio metadata, ...) multiplexed into a
professional video signal. This crate is about that content, not any one
carriage mechanism. Use it when you need to read or write ANC packets that
travel as a `0xBD` PES stream in a transport stream (broadcast contribution,
SMPTE ST 2038) or as an RTP stream (SMPTE ST 2110-40 IP facilities).

- It parses and serializes the wire structures, byte-exact.
- It exposes the ST 291-1 10-bit word parity rule and the `Checksum_Word`
  computation as explicit helpers (`AncContent::with_parity`,
  `compute_checksum`, `verify_checksum`, `build`).
- It does **not** interpret the payload. `DID`/`SDID` and the user data words
  are opaque; there is no CEA-708 / AFD / ST 12-2 timecode / SCTE 104
  decoding here.
- It does **not** validate parity or checksum on parse: a bit-flipped packet
  still parses `Ok`, and the caller opts in with `verify_checksum()`.
- It does **not** implement the SDI wire (ST 292/424 HANC/VANC word packing),
  the ST 2110-40 SDP/session layer, or the RTP fixed header (that is the
  `rtp-packet` crate's job).

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|----------|---------|-----------------|--------|-------|
| SMPTE ST 2038 - Carriage of Ancillary Data Packets in an MPEG-2 Transport Stream | 2021 (vendored, verified) | Section 4.1 Table 1 `anc_data_descriptor` (tag `0xC4`), `"VANC"` registration descriptor identifier `0x56414E43`, `stream_type` `0x06`; section 4.2 Table 2 ANC data PES packet | Full | PES header parsed inline; fixed/reserved bits validated (`Error::BadFixedBits`); PTS of 2^33 or more rejected |
| IETF RFC 8331 - RTP Payload for SMPTE ST 291-1 Ancillary Data | Feb 2018 | Section 2 / 2.1 payload header and ANC packet list, `F` field, `Length`/`ANC_Count` cross-validation; section 3.1/4 media type + default 90 kHz clock rate constants | Full (wire format) | Not the SDP/session layer. `F == 0b01` parses (reported as `FieldSense::Invalid`) |
| SMPTE ST 2110-40 - SMPTE ST 291-1 Ancillary Data over RTP | 2023 (vendored) | Normatively builds on RFC 8331 | Partial | The implementation is grounded in RFC 8331; the 2023 text has not been line-audited against the code (unverified delta) |
| SMPTE ST 291-1 - Ancillary Data Packet and Space Formatting (explicit validation) | 2011 (vendored, `ST 291:2011`) | Sections 6.1/6.2/6.4/6.5 DID/SDID/DBN/DC parity (`AncContent::has_valid_parity`/`verify_parity`), 6.7 checksum, `AncContent::validate`/`AncPacket::validate` -> `ValidationReport`; see `docs/anc_parity_checksum.md` | Full (word-level rules) | Opt-in; parse stays tolerant. UDWs carry no parity rule (section 6.6) |
| SMPTE ST 291-1 - Ancillary Data Packet and Space Formatting | 2011 (vendored, `ST 291:2011`) | 10-bit parity rule and `Checksum_Word` | Partial | Derived from RFC 8331 section 2.1, which reproduces the rule; the ANC *space* formatting (SDI HANC/VANC, EDH) is out of scope |

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `ts`    | yes     | SMPTE ST 2038:2021 MPEG-2 TS transport (`AncDataDescriptor`, `AncDataPacket`, `AncPacket`). |
| `rtp`   | no      | RFC 8331 / ST 2110-40 RTP transport (`AncRtpPayload`, `RtpAncPacket`, `FieldSense`); pulls in `rtp-packet`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

`AncContent` (the shared DID/SDID/count/words/checksum core) is always compiled.

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

More runnable programs: `cargo run -p st291 --example build_anc`,
`parse_anc`, and (with `--features rtp`) `build_anc_rtp`, `parse_anc_rtp`.

## What is implemented

- **Shared content** (`AncContent`): `DID`, `SDID`, `data_count`,
  `user_data_words`, `checksum`, all stored as raw 10-bit values (parity bits
  included). The `user_data_word` loop counter uses the low 8 bits of
  `data_count`; a mismatch with `user_data_words.len()` is rejected on
  serialize (`Error::InconsistentUdwLength`). Helpers: `with_parity`,
  `compute_checksum`, `verify_checksum`, `build`.
- **`ts` transport**: `AncDataDescriptor` (PMT ES-loop descriptor, tag
  `0xC4`, with its `"VANC"` format identifier and `ANC_STREAM_TYPE` constants);
  `AncDataPacket` (PES packet, `stream_id` `0xBD`, PTS,
  `PES_header_data_length == 5`) holding a list of `AncPacket` (adds
  `c_not_y_channel_flag`, `line_number`, `horizontal_offset`) plus trailing
  `0xFF` stuffing.
- **`rtp` transport**: `AncRtpPayload` (extended sequence number, `F` as
  `FieldSense`, packet list; `Length` and `ANC_Count` are always recomputed on
  serialize and cross-checked on parse) holding `RtpAncPacket` (`C`, line
  number, horizontal offset, `S`, `StreamNum` + an `AncContent`).
  `AncRtpPayload::parse_rtp_packet` splits a whole RTP datagram via
  `rtp-packet`.

## Design notes

- Every wire structure implements the workspace `Parse` / `Serialize` pair
  with a byte-exact round-trip test, including a committed real-shape fixture
  per transport.
- The content fields are a contiguous MSB-first 10-bit stream walked with
  `broadcast_common::bits`; it is identical across both transports.
- Reserved bits are validated on parse (`BadFixedBits`, `ReservedNotZero`) so
  a corrupt packet cannot parse `Ok` and re-serialize to different bytes.
- Every length/count is written through the checked `fit_*` helpers; an
  over-range value is an error (`FieldTooWide`), never a silent wrap.
- Errors are a single `thiserror` enum, `st291::Error`; public enums are
  `#[non_exhaustive]` and carry `name()` + `Display` labels.
- `#![no_std]` + `alloc`.

## Related crates

Depends on `broadcast-common` (traits, bit reader/writer, 33-bit clock
helpers) and, with `rtp`, on `rtp-packet`. `mpeg-ts` supplies the surrounding
TS packetisation for the `ts` transport. Payload (DID/SDID) interpretation is
left to the caller.

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
