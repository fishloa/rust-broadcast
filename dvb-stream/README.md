# dvb-stream

[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)
[![crates.io](https://img.shields.io/crates/v/dvb-stream.svg)](https://crates.io/crates/dvb-stream)
[![docs.rs](https://img.shields.io/docsrs/dvb-stream)](https://docs.rs/dvb-stream)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#related-crates-and-license)

**Async/tokio `Stream` adapters over `dvb-si`'s `SiDemux` and `dvb-t2mi`'s `T2miPump`: feed a file, TCP socket or UDP/multicast source, receive owned SI section or T2-MI events.**

## What it does / what it does not

`dvb-stream` wraps the synchronous demuxers as `futures_core::Stream`s so an
async program (a probe, a collector, a service) can consume DVB data without
blocking, while keeping `tokio` and `futures-core` out of the pure parser
crates.

- `SectionStream` takes any `tokio::io::AsyncRead` and yields one
  `dvb_si::demux::SectionEvent` per *changed* complete SI section. Events are
  owned (`bytes::Bytes`), `'static`, `Clone` and `Send + Sync`.
- `T2miEventStream` does the same for T2-MI: one `dvb_t2mi::pump::T2miEvent` per
  CRC-valid T2-MI packet.
- With the `udp` feature (default), `UdpSectionStream` / `UdpT2miStream` read
  UDP unicast or multicast (the dominant real-world DVB transport), one
  datagram per framing unit, including RTP-encapsulated MPEG-2 TS.

Both stream families frame bytes with a `tokio_util::codec::FramedRead` over the
public `TsDecoder` (188-byte alignment and resync). No tasks are spawned;
cancelling is dropping the stream. It does not parse anything itself (that is
`dvb-si` / `dvb-t2mi`), is std-only (tokio needs std), and offers no TCP/RTSP/SRT
connection management.

## Standards supported

The crate adds framing and transport only; table and packet decoding is
delegated.

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1 §2.4.3.2 | H.222.0 v9 (08/2023) | 188-byte TS packet alignment and sync-byte resync (`TsDecoder`, `resync`) via `mpeg-ts` | Full for framing | 204-byte RS-coded packets are not handled here (see `mpeg-ts::TsResync`). |
| RFC 3550 §5.1 (RTP) and RFC 2250 §2 (RTP payload for MPEG-2 TS) | RFC 3550 vendored (`specs/rfc3550_rtp.txt`); RFC 2250 not vendored (unverified) | `TsDecoder::datagram` unwraps an RTP header (CSRCs, extension, padding) when the payload is whole 188-byte packets starting on `0x47` | Partial | Payload type is not inspected; the detection is structural. Uses `rtp-packet`. |
| ETSI EN 300 468 (SI) and TS 102 773 (T2-MI) | via `dvb-si` and `dvb-t2mi` | Section and T2-MI event semantics | Delegated | |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `udp` | yes | `UdpSectionStream`, `UdpT2miStream`, `udp::MulticastConfig` (`socket2` bind and join: `SO_RCVBUF`, opt-in `SO_REUSEADDR`/`SO_REUSEPORT`, multicast interface), `tokio_util::udp::UdpFramed` framing |

`std` only. Versioned and released independently of the `dvb-si`/`dvb-t2mi`
lockstep (0.x), because tokio's MSRV moves faster than the workspace pin.

```toml
[dependencies]
dvb-stream = "0.6"
```

## Quick start

```rust
use dvb_stream::SectionStream;
use dvb_si::tables::AnyTableSection;
use futures_util::StreamExt;

let data = std::fs::read("../fixtures/ts/m6-single.ts").unwrap(); // committed capture
let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
let pats = rt.block_on(async {
    let mut s = SectionStream::new(&data[..]); // &[u8] implements AsyncRead
    let mut pats = 0u32;
    while let Some(event) = s.next().await {
        if let Ok(AnyTableSection::PatSection(pat)) = event.table_section() {
            let _ = pat.transport_stream_id;
            pats += 1;
        }
    }
    pats
});
assert!(pats >= 1);
```

Multicast (not run here, needs a network):

```rust,no_run
# #[cfg(feature = "udp")] {
use std::net::{Ipv4Addr, SocketAddrV4};
use dvb_stream::{UdpSectionStream, udp::MulticastConfig};

# async fn run() -> std::io::Result<()> {
let cfg = MulticastConfig::new(
    SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 5004),
    Ipv4Addr::new(239, 0, 0, 1),
)
.with_recv_buffer_size(4 << 20);
let _stream = UdpSectionStream::bind(&cfg).await?;
# Ok(()) }
# }
```

Examples: `cargo run -p dvb-stream --example count_sections` and
`--example stream_stats` (table-type tally plus demux and resync statistics).

## What is implemented

| Item | Purpose |
|---|---|
| `SectionStream<R>` (`new`, `with_builder`, `with_demux`, `stats`, `resync_stats`, `take_io_error`) | `AsyncRead` to `SectionEvent` stream |
| `T2miEventStream<R>` (`new(reader, pid: Pid)`, `with_pump`, `stats`, `resync_stats`, `take_io_error`) | `AsyncRead` to `T2miEvent` stream |
| `UdpSectionStream`, `UdpT2miStream` (`from_socket`, `bind_multicast`, `bind(&MulticastConfig)`, `stats`, `resync_stats`, `take_io_error`) | UDP/multicast sources (`udp` feature) |
| `udp::MulticastConfig` (`new`, `with_interface`, `with_recv_buffer_size`, `with_reuse_address`, `bind`) | `socket2` socket options; `#[non_exhaustive]` |
| `TsDecoder` (`new`, `datagram`, `resync_stats`) | `tokio_util::codec::Decoder` for aligned TS packets; `datagram()` resyncs each datagram independently and unwraps RTP |
| `resync::{resync, aligned_packets, TS_PACKET_SIZE, TS_SYNC_BYTE}`, `ResyncStats { resyncs, bytes_discarded, desyncs }` | The one resync implementation shared by every stream |

`take_io_error` surfaces the I/O error that ended a stream (a `Stream` item has
no error channel).

## Design notes

- **Owned, `'static` events** so they cross task boundaries and outlive the read
  buffer.
- **Sans-spawn.** The adapter owns the buffer and feeds the synchronous pump
  from `poll_next`; events queue per packet and drain before the next read.
- **Observability.** `resync_stats()` and the inner pump's `stats()` make every
  discarded byte and CRC failure visible.
- **No hand-rolled socket or RTP code.** `tests/no_handroll_guard.rs` carries
  the workspace tripwire; UDP binds go through `socket2`, RTP through
  `rtp-packet`.
- The crate defines no public enum, so the label-coverage and
  `#[non_exhaustive]` drift guards do not apply (recorded in
  `broadcast-common`'s guard-coverage test).

## Related crates and license

Depends on `dvb-si`, `dvb-t2mi`, `mpeg-ts`, `rtp-packet`, `broadcast-common`.
Pairs with `dvb-tools` (the synchronous file CLI). MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
