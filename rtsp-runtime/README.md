# rtsp-runtime

Sans-IO **RTSP 1.0** ([RFC 2326](https://www.rfc-editor.org/rfc/rfc2326)) session
engine — a driveable client **and** server, the piece the Rust ecosystem leaves
out.

Message parse/serialize is delegated to the mature [`rtsp-types`] and
[`sdp-types`] codecs, and authentication math to [`http-auth`]. What lives here
is the part nothing else provides:

- **Session state machines** — client and server, per RFC 2326 Appendix A
  (`Init → Ready → Playing/Recording`), with illegal-method-in-state rejection.
- **CSeq correlation** — requests to responses, sequence tracking.
- **`Transport` negotiation** — UDP and TCP-interleaved.
- **Interleaved RTP/RTCP framing** — the `$`-channel muxing of §10.12.
- **Auth** — Basic + Digest (RFC 7617 / 7616) wired into request signing, for
  the authenticated RTSP that IP cameras require.

All **sans-IO**: feed inbound bytes and the wall-clock, get outbound bytes and
typed events back — no sockets in the core. An optional **`tokio`** adapter (and
**`tls`** for `rtsps://`) drives real sockets over the same engine.

```toml
[dependencies]
rtsp-runtime = "0.1"                                  # sans-IO core
rtsp-runtime = { version = "0.1", features = ["tokio"] }  # + real sockets
rtsp-runtime = { version = "0.1", features = ["tls"] }    # + rtsps:// (TLS)
```

## Status

Skeleton — the engine is being built out per [issue #521](https://github.com/fishloa/rust-broadcast/issues/521).

## License

MIT OR Apache-2.0.

[`rtsp-types`]: https://crates.io/crates/rtsp-types
[`sdp-types`]: https://crates.io/crates/sdp-types
[`http-auth`]: https://crates.io/crates/http-auth
