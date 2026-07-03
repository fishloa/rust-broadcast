# rtsp-runtime

Sans-IO **RTSP 1.0** ([RFC 2326](https://www.rfc-editor.org/rfc/rfc2326)) session
engine — a driveable client **and** server, the piece the Rust ecosystem leaves
out.

Message parse/serialize is delegated to the mature [`rtsp-types`] and
[`sdp-types`] codecs, and authentication math to [`http-auth`]. What lives here
is the part nothing else provides:

- **Session state machines** — [`ClientSession`] and [`ServerSession`], per
  RFC 2326 Appendix A (`Init → Ready → Playing/Recording`), with
  illegal-method-in-state rejection (the client returns `Err`; the server
  returns `455 Method Not Valid In This State`).
- **CSeq correlation** — outbound requests carry an incrementing `CSeq`;
  inbound responses are matched back to the pending request.
- **`Transport` negotiation** — a typed, round-trippable `Transport` header
  (§12.39): UDP unicast/multicast and TCP-interleaved.
- **Interleaved RTP/RTCP framing** — `InterleavedFrame` and a streaming
  demultiplexer for the `$`-channel muxing of §10.12 (complete frames + partial
  tail).
- **Auth** — Basic + Digest (RFC 7617 / 7616) wired into request signing,
  including transparent `401` retry and `stale=true` nonce refresh, for the
  authenticated RTSP that IP cameras require.

All **sans-IO**: feed inbound bytes, get outbound bytes and typed events back —
no sockets in the core. Drive `ClientSession` with the request builders and
`handle_data`; drive `ServerSession` with `handle_request`.

- **Async socket adapter** (feature `tokio`) — `AsyncRtspClient` /
  `AsyncRtspServer` own a `tokio::net::TcpStream`, move the bytes the sans-IO
  engine produces/consumes, buffer fragmented reads, and answer Digest `401`
  challenges transparently. Both are generic over the stream type, so the same
  driver logic runs over plain TCP and TLS.
- **`rtsps://` over TLS** (feature `tls`) — the adapter wraps the stream in a
  `tokio-rustls` session before the RTSP exchange (`AsyncRtspClient::connect_tls`
  / `AsyncRtspServer::accept_tls`), for `rtsps://` on default port **322**. The
  client trusts the `webpki-roots` bundle by default, or a custom
  `rustls::ClientConfig` for a self-signed camera cert.

[`ClientSession`]: https://docs.rs/rtsp-runtime/latest/rtsp_runtime/client/struct.ClientSession.html
[`ServerSession`]: https://docs.rs/rtsp-runtime/latest/rtsp_runtime/server/struct.ServerSession.html

```toml
[dependencies]
rtsp-runtime = "0.1"                                  # sans-IO core
rtsp-runtime = { version = "0.1", features = ["tokio"] }  # + real sockets
rtsp-runtime = { version = "0.1", features = ["tls"] }    # + rtsps:// (TLS)
```

## Status

The sans-IO core — client + server state machines, `Transport` negotiation,
interleaved framing, and Basic/Digest auth — is implemented and tested against
the RFC 2326 fixtures (see [issue #521](https://github.com/fishloa/rust-broadcast/issues/521)).

The `tokio` socket adapter (and `tls` for `rtsps://`) is implemented on top of
the same core, with real-loopback integration tests covering a full
OPTIONS→DESCRIBE→SETUP→PLAY→TEARDOWN session, fragmented interleaved media,
transparent Digest auth, and an end-to-end TLS handshake with a self-signed
cert.

## License

MIT OR Apache-2.0.

[`rtsp-types`]: https://crates.io/crates/rtsp-types
[`sdp-types`]: https://crates.io/crates/sdp-types
[`http-auth`]: https://crates.io/crates/http-auth
