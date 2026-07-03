# rtsp-runtime 0.1.0 — 2026-07-03

First publish. A **sans-IO RTSP 1.0** (RFC 2326) session engine — the driveable
client **and** server the Rust ecosystem was missing. Message parse/serialize is
delegated to the mature `rtsp-types` + `sdp-types` codecs and authentication to
`http-auth`; this crate owns the part nothing else provides.

## What's in it

- **Session state machines** — client and server, per RFC 2326 Appendix A
  (`Init → Ready → Playing/Recording`); a method not valid in the current state
  is rejected (server → `455`).
- **CSeq correlation**, `Session` id + timeout tracking.
- **`Transport` negotiation** — typed parse/serialize (unicast/multicast,
  `interleaved=`, `client_port=`, …), UDP and TCP-interleaved.
- **Interleaved RTP/RTCP framing** — the `$`-channel muxing of §10.12, with a
  streaming demultiplexer that retains a partial tail.
- **Auth** — Basic + Digest (RFC 7617 / 7616) wired into request signing, incl.
  `401` → retry and `stale=true` nonce refresh — the authenticated RTSP that IP
  cameras require.
- **Sans-IO core** — feed inbound bytes + wall-clock, get outbound bytes + typed
  events; no sockets in the core.
- **`tokio` adapter** (optional feature) — real-socket async client + server;
  **`tls`** adds `rtsps://` over `tokio-rustls`. Both share one stream-generic
  driver; verified with real `127.0.0.1` loopback tests (full session,
  fragmented interleaved media, Digest, and end-to-end TLS).

## Compatibility

MSRV 1.81. The sans-IO core builds with `--no-default-features`; `tokio`/`tls`
pull the async stack only when enabled.
