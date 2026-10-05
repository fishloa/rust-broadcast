# dvb-stream 0.6.0

_Released 2026-10-05._

**Minor-epoch breaking release (0.x).** UDP multicast reading is rebuilt on a shared `tokio_util` codec, which removes `section_stream::UdpReader`, adds a configurable multicast bind, and fixes three data-loss bugs: UDP datagrams larger than 7 TS packets were truncated, a partial packet at the end of one datagram was stitched onto the next unrelated one, and `T2miEventStream` dropped any partial packet at a read boundary. If you name `UdpReader`, or construct streams over `SectionStream<UdpReader>`, you must change the type; otherwise the upgrade is a version bump plus the dependency epoch moves below. Read with [dvb-si 11.0.0](dvb-si-11.0.0.md), [dvb-t2mi 11.0.0](dvb-t2mi-11.0.0.md) and [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md).

## Dependency and feature changes

```toml
-dvb-si    = { version = "10",  default-features = false, features = ["ts", "std"] }
-dvb-t2mi  = { version = "10",  default-features = false, features = ["ts", "std"] }
-mpeg-ts   = { version = "0.4", default-features = false, features = ["std"] }
+dvb-si    = { version = "11.0", default-features = false, features = ["ts", "std"] }
+dvb-t2mi  = { version = "11.0", default-features = false, features = ["ts", "std"] }
+mpeg-ts   = { version = "0.5", default-features = false, features = ["std"] }
-tokio     = { version = "1", default-features = false, features = ["io-util", "net"] }
+tokio     = { version = "1", default-features = false, features = ["io-util", "net", "time"] }
+tokio-util = { version = "0.7", default-features = false, features = ["codec"] }
+bytes     = "1"
+socket2   = { version = "0.6", optional = true, features = ["all"] }
-udp = []
+udp = ["dep:socket2", "tokio-util/net"]
-broadcast-common = "9.3"   # dev-dependency
+broadcast-common = "9.4"   # dev-dependency
```

The `udp` feature now pulls in `socket2` and `tokio-util/net`.

## Breaking changes

1. `TsFramer` (crate-private) is replaced by the public `TsDecoder`, a `tokio_util::codec::Decoder` over 188-byte TS packets (`Item = bytes::Bytes`; `TsDecoder::datagram()` for UDP framing). `SectionStream<R>` and `T2miEventStream<R>` keep their reader-generic API and wrap `FramedRead<R, TsDecoder>`. Framing behaviour is unchanged, pinned by a golden over a real capture, including the bound on loss at a corrupt sync byte (at most one old 7 x 188-byte read dropped per desync, then in-buffer resync).
2. `section_stream::UdpReader` is removed. The multicast constructors move to the concrete stream types with unchanged signatures:

```rust
// before
let s = SectionStream::<UdpReader>::bind_multicast(bind_addr, group).await?;
let t = T2miEventStream::<UdpReader>::bind_multicast(bind_addr, group, pid).await?;
// after (feature "udp")
let s = UdpSectionStream::bind_multicast(bind_addr, group).await?;
let t = UdpT2miStream::bind_multicast(bind_addr, group, pid).await?;
```

## New

- `udp::MulticastConfig` (`#[non_exhaustive]`; `MulticastConfig::new(bind_addr, group)` plus `with_interface`, `with_recv_buffer_size`, and a `reuse_address` option): `socket2`-based bind and join with `SO_RCVBUF`, an optional multicast interface, and opt-in `SO_REUSEADDR` (plus `SO_REUSEPORT` on unix). Reuse defaults to off, as with the plain bind it replaces. Use it with the new `UdpSectionStream::bind(&config)` and `UdpT2miStream::bind(&config, pid)`; `from_socket` constructors accept an already-bound socket.
- `SectionStream::take_io_error` and `T2miEventStream::take_io_error` (also on the UDP wrappers): when `poll_next` ends the stream because the reader errored rather than reached a clean EOF, the error is now retrievable, so a supervisor can tell "source finished" from "source failed" and reconnect (#1099, W-DS-1).

## Fixes

- `bind_multicast` (both streams) read each datagram into the 1,316-byte (7 x 188) stream buffer, silently truncating any larger datagram (routine for RTP-encapsulated delivery), and stitched a trailing partial packet from one datagram onto the next, unrelated one. Datagrams are now read into a 65,535-byte buffer and partial packets are never carried across datagrams (#1099, W-DS-2).
- `T2miEventStream::feed_buf` discarded any trailing partial TS packet at the end of every read, unlike `SectionStream`, so any read boundary off a 188-byte packet boundary lost data. Partial bytes are now carried to the next read (#1036).
- `SectionStream` and `T2miEventStream` now share one framer instead of two diverging copies of `feed_buf` and the read loop (#1141).

---

Published from tag `dvb-stream-v0.6.0`.
