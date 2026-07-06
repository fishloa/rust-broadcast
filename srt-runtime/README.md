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
- **LiveCC packet pacing** (§5.1) — `livecc::LiveCC`: live/streaming-mode
  pacing-only rate control.
- **FileCC window congestion control** (§5.2) — `filecc::FileCc`: the
  file/bulk-transfer-mode sibling of LiveCC — a two-phase hybrid AIMD
  algorithm (Slow Start, then Congestion Avoidance) that also grows/shrinks a
  congestion window, not just a pacing interval.

**Payload encryption, wired into the handshake** (§6, `crypto` feature):

- AES-CTR payload encrypt/decrypt, RFC 3394 AES key wrap/unwrap of the SEK,
  and PBKDF2 (HMAC-SHA1) KEK derivation — `crypto`.
- The Caller/Listener handshake negotiates it: `handshake_sm::CryptoConfig`
  (a pre-shared passphrase) piggybacks the Key Material exchange (§6.1.5) on
  the existing CONCLUSION extension flow, reusing `packet::KeyMaterial`
  rather than a new wire message — no separate negotiation step to wire up
  yourself.
- A sans-IO SEK-rotation driver for §6.1.6 KM Refresh —
  `km_refresh::KmRefreshDriver`.

**Optional `tokio` feature** — an async UDP socket adapter (`io::SrtSocket` /
`io::SrtListener`) that drives the handshake + ARQ + TSBPD + LiveCC/FileCC
engines end-to-end over real sockets.

The core is `no_std` + `alloc` (default `std` feature can be turned off); no
`unsafe`. The `crypto` and `tokio` features are `std`-only and off by default, so
the packet-codec + sans-IO core pulls zero crypto/async dependencies.

## What's *not* here — explicit follow-ups

- CUBIC/BBR or any other alternative file-transfer congestion-control
  algorithm (§5.2 names them as applicable alternatives to `filecc::FileCc`'s
  default algorithm but does not describe them).
- Wiring `filecc::FileCc` / `livecc::LiveCC` into a real send-queue scheduler
  (`arq::Sender` has no congestion-control hook today).
- Wiring the negotiated SEK from `handshake_sm::NegotiatedParams` (or
  `km_refresh::KmRefreshDriver`'s events) into `io`'s tokio adapter to
  actually encrypt/decrypt data-packet payloads end-to-end over a real
  socket — the handshake negotiation and the rotation state machine are both
  sans-IO and fully wired/tested; driving real payload encryption from them
  through `io.rs` (additive, `crypto`-feature-gated, without touching the
  existing ARQ/TSBPD paths) is a follow-up.

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
