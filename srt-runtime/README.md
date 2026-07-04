# srt-runtime

SRT (Secure Reliable Transport) packet codecs, grounded in
[`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
(the free, redistributable IETF Internet-Draft) §3, Packet Structure.

## What's here (this release)

Typed, byte-exact `parse`/`serialize` for every SRT packet type:

- The 16-byte SRT header and the `F` (Packet Type Flag) dispatch (data vs.
  control).
- **Data packets** (§3.1) — sequence number, message number, packet
  position, order/retransmit/encryption flags.
- **Every control packet type** (§3.2): Handshake (with its extension
  messages — Handshake Extension, Key Material, Stream ID, Group
  Membership), Keep-Alive, ACK (Full/Small/Light), NAK (loss-list coding,
  Appendix A), Congestion Warning, Shutdown, ACKACK, Message Drop Request,
  and Peer Error.

`no_std` + `alloc` (default `std` feature can be turned off); no `unsafe`.

## What's *not* here — explicit follow-ups

- The handshake **state machine** (caller/listener/rendezvous exchange, §4.3)
  — this crate parses/builds handshake *packets*, not a connection.
- ARQ / loss handling, TSBPD, congestion control (§4-§5).
- Actual AES key-wrap/unwrap **crypto** (§6) — `KeyMaterial` carries the
  wrapped-key bytes opaquely, performing no cryptographic operations.
- A `tokio` socket adapter (mirroring `rtsp-runtime`'s `io` module).

```toml
[dependencies]
srt-runtime = "0.1"
```

## License

MIT OR Apache-2.0.
