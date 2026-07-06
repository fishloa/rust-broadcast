# srt-runtime

SRT (Secure Reliable Transport), grounded in
[`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
(the free, redistributable IETF Internet-Draft). A **sans-IO** core (feed bytes,
tick a clock — no sockets in the core) plus an optional `tokio` socket adapter.

## What's here

**Packet codecs (§3)** — typed, byte-exact `parse`/`serialize` for every SRT
packet type:

- The 16-byte SRT header and the `F` (Packet Type Flag) dispatch (data vs.
  control).
- **Data packets** (§3.1) — sequence number, message number, packet position,
  order/retransmit/encryption flags.
- **Every control packet type** (§3.2): Handshake (with its extension messages —
  Handshake Extension, Key Material, Stream ID, Group Membership), Keep-Alive,
  ACK (Full/Small/Light), NAK (loss-list coding, Appendix A), Congestion
  Warning, Shutdown, ACKACK, Message Drop Request, and Peer Error.

**Sans-IO connection engines** (feed typed packets + `tick()`, get bytes/events
out — no wall-clock read inside the crate):

- **HSv5 Caller/Listener handshake** state machines (§4.3.1) — `caller`,
  `listener`.
- **Rendezvous handshake** state machine (§4.3.2) — `rendezvous`.
- **ARQ reliability engine** (§4.8/§4.10) — `arq::Sender` / `arq::Receiver`:
  wrap-safe sequence arithmetic, retransmit on NAK, receiver loss list,
  ACK/ACKACK, RTT/RTTVar estimation.
- **TSBPD delivery scheduler** (§4.5/§4.6) — `tsbpd::TsbpdScheduler`:
  timestamp-based ordered delivery + too-late packet drop.
- **LiveCC packet pacing** (§5.1) — `livecc::LiveCC`.

**Optional features:**

- `crypto` — AES-CTR payload encrypt/decrypt, RFC 3394 key wrap/unwrap, and
  PBKDF2/HMAC-SHA1 KEK derivation (§6). The key-wrap and KEK primitives are
  verified against RFC 3394 and NIST SP 800-38A test vectors.
- `tokio` — an async UDP socket adapter (`io::SrtSocket` / `io::SrtListener`)
  that drives the handshake + ARQ + TSBPD engines end-to-end over real sockets.

The core is `no_std` + `alloc` (default `std` feature can be turned off); no
`unsafe`. The `crypto` and `tokio` features are `std`-only and off by default, so
the packet-codec + sans-IO core pulls zero crypto/async dependencies.

## What's *not* here — explicit follow-ups

- Window-based congestion control beyond LiveCC packet pacing (the rest of §5).
- Wiring `crypto` into the handshake state machines / a per-connection
  SEK-rotation driver (§6.1.6 KM Refresh) — this release ships the crypto
  *primitives* only.

## Permanently out of scope

- **The Version-4 legacy Rendezvous path (§4.3.2).** Only the current HSv5
  Rendezvous flow is implemented. V4 is a legacy interop path for pre-HSv5
  peers; not planned.

```toml
[dependencies]
srt-runtime = "0.2"
# optional: srt-runtime = { version = "0.2", features = ["tokio", "crypto"] }
```

## License

MIT OR Apache-2.0.
