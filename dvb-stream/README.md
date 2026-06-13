# dvb-stream

Async/tokio stream adapters over [`dvb-si`](https://crates.io/crates/dvb-si) `SiDemux` and
[`dvb-t2mi`](https://crates.io/crates/dvb-t2mi) `T2miPump`.

[![CI](https://github.com/fishloa/rust-dvb/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-dvb/actions)
[![crates.io](https://img.shields.io/crates/v/dvb-stream.svg)](https://crates.io/crates/dvb-stream)
[![docs.rs](https://img.shields.io/docsrs/dvb-stream)](https://docs.rs/dvb-stream)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## What it does

`dvb-stream` wraps the synchronous `dvb-si`/`dvb-t2mi` pumps as
[`futures_core::Stream`] implementations, quarantining `tokio` and
`futures-core` away from the pure-Rust parser crates.

- **`SectionStream`** — feed any `tokio::io::AsyncRead` (file, TCP, UDP), receive
  one `dvb_si::demux::SectionEvent` per changed complete SI section. Events are
  owned (`bytes::Bytes`), `'static`, and `Send + Sync`.
- **`T2miEventStream`** — same treatment for T2-MI: feed any `AsyncRead`, receive
  one `dvb_t2mi::pump::T2miEvent` per CRC-valid T2-MI packet.

Both streams perform 188-byte TS packet alignment and resync on the byte stream.
With the `udp` feature (default), `bind_multicast` convenience constructors are
provided for the dominant real-world DVB transport (UDP multicast, e.g.
`239.0.0.1:5004`).

## MSRV and versioning

**MSRV: 1.75** (matches the workspace). `dvb-stream` is versioned and released
**independently** from the `dvb-si`/`dvb-t2mi` lockstep (`0.x` series), because
tokio's own MSRV moves faster than the workspace pin.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `udp`   | on      | `SectionStream::bind_multicast` / `T2miEventStream::bind_multicast` via `tokio::net::UdpSocket`. |

## License

MIT OR Apache-2.0.
