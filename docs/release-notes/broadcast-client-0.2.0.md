# broadcast-client 0.2.0

_Released 2026-11-03._

Minor release: UDP, RTP, SRT, Smooth Streaming and RTMP sources plus client certificates (breaking: `Action` identity field).

### Added

- Client certificates (mutual TLS, #1461): `Config::client_identity` (`ClientIdentity::pem`/`der`/`handle`, key wiped on drop and redacted in `Debug`) and `Config::client_identity_scope` (`ClientIdentityScope::SessionOrigin` default, `AnyHttps`). `Action::HttpRequest` (https URLs) and `Action::TcpConnect` (`tls: true`) carry `identity: Option<Arc<ClientIdentity>>` for the host to present; both actions gained that field. Malformed material fails `Session::open` with `InvalidConfig { field: "client_identity" }`. Verified end to end against a local rustls server that requires client authentication.
- SRT caller source: `srt://host:port` URLs (`SourceKind::Srt`, picked by `Auto`) play MPEG-TS from an SRT listener on `srt-runtime`'s caller handshake, ARQ receiver and TSBPD scheduler, with passphrase encryption (`SrtEncryption`, AES-128/192/256, wrong passphrases refused by the handshake), latency, stream id and handshake/idle timeouts in `Config::srt` (`SrtConfig`). `Action::UdpBind { mode: UdpMode::Connect, .. }` opens a connected datagram socket and `Action::Send` writes one datagram. The session reads no randomness: `SrtEntropySeed` (32 bytes from the host's CSPRNG) is required and every connection attempt derives fresh identifiers and keys from it. New `ErrorKind::SrtRejected { reason }` and `ErrorKind::SrtHandshakeTimeout`; `DatagramStats::undecryptable`. Verified against `srt-runtime`'s listener and libsrt's `srt-live-transmit` (clear, encrypted, wrong passphrase, lossy link).
- UDP and RTP sources: `udp://` and `rtp://` URLs (`SourceKind::Udp`, `SourceKind::Rtp`, picked by `Auto`) play MPEG-TS from unicast or multicast datagrams. `Action::UdpBind { conn, host, port, mode, multicast, recv_buffer_bytes }` asks the host for a datagram socket (`UdpMode::Receive`, with an optional `MulticastJoin { interface, source }` for source-specific multicast); the host reports the bind with `on_connected` and feeds one datagram per `on_data`. `Config::udp` (`UdpConfig`: interface, source, `SO_RCVBUF`, RTP reorder window and wait) and `Session::datagram_stats()` (`DatagramStats`). Plain UDP datagrams must be whole TS packets; RTP is parsed with `rtp-packet`, payload type 33 only, with a bounded reorder buffer (sequence wrap, duplicates, late packets, loss, SSRC change). Silence, start timeout, stall notices and rebind use the TS-over-HTTP policy, so the events are the same as for TS-over-HTTP.
- Smooth Streaming source: `.../Manifest` URLs (or `SourceKind::Smooth`) pull MS-SSTR presentations over HTTP using `transmux`'s client manifest parser, with quality selection via `hls_runtime::client::choose_variant`, `tfxd`-based sample placement, live join/refresh, and fragment retry/give-up semantics.
- RTMP play source: `rtmp://` and `rtmps://` URLs (`SourceKind::Rtmp`, picked by `Auto`) play over a TCP connection the host opens, on `rtmp-runtime`'s sans-IO client in play mode (issue #1460), with FLV tags demuxed by `transmux::StreamingFlvDemux`; start, stall, backoff and reconnect follow the TS-over-HTTP policy.


### Changed
- The `live_loop` example is now `session_loop`: its name collided with `buffering-runtime`'s `live_loop`, and cargo warned about the clashing output file in a workspace build.

### Breaking
- `Action::HttpRequest` and `Action::TcpConnect` gained an `identity: Option<Arc<ClientIdentity>>` field for client certificates (issue #1461), so every struct literal must add the field.

## Upgrading

`Action::HttpRequest` and `Action::TcpConnect` gained an `identity` field: the client certificate the host must present on that connection, or `None`. A host that matches these variants with every field listed stops compiling; add `identity` (or `..`) to the pattern and hand the certificate to the TLS layer:

```rust
// 0.1
Action::HttpRequest { conn, method, url, headers, byte_range } => { /* ... */ }
Action::TcpConnect { conn, host, port, tls } => { /* ... */ }

// 0.2
Action::HttpRequest { conn, method, url, headers, byte_range, identity } => {
    // when `identity` is Some, present it as the TLS client certificate
}
Action::TcpConnect { conn, host, port, tls, identity } => { /* same */ }
```

The new sources also emit `Action::UdpBind` and `Action::Send`. `Action` is `#[non_exhaustive]`, so a host that ignored unknown actions keeps compiling, but it must handle these two to play `udp://`, `rtp://` and `srt://` URLs.

---

Published from tag `broadcast-client-v0.2.0`.
