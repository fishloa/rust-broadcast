# srt-runtime 0.2.0 — 2026-07-06

From packet codecs to a working SRT connection: sans-IO connection engines for
every layer between the wire and delivered payload, plus a real `tokio` socket
adapter that drives them end-to-end.

## Added — the sans-IO connection stack
- **HSv5 Caller/Listener handshake** (§4.3.1, #598/#605) — `caller`, `listener`.
- **Rendezvous handshake** (§4.3.2, #609) — symmetric cookie-contest peer↔peer flow.
- **ARQ reliability engine** (§4.8/§4.10, #606) — wrap-safe sequence arithmetic,
  sender retransmit on NAK, receiver loss list, ACK/ACKACK, RTT/RTTVar EWMA.
- **TSBPD delivery scheduler** (§4.5/§4.6, #607) — timestamp-ordered release +
  too-late packet drop.
- **LiveCC packet pacing** (§5.1, #610).
- **`crypto` feature** (§6, #608) — AES-CTR payload encrypt/decrypt, RFC 3394
  key wrap/unwrap, PBKDF2/HMAC-SHA1 KEK derivation. Verified against RFC 3394
  and NIST SP 800-38A vectors; the AES-CTR counter matches the reference
  implementation (`libsrt` `haicrypt/hcrypt.h`), not the draft's ambiguous
  `§6.2.2` formula (see Fixed).
- **`tokio` feature** (#611) — `io::SrtSocket` / `io::SrtListener`: an async UDP
  socket adapter driving the handshake + ARQ + TSBPD + LiveCC engines over real
  sockets. Background per-connection driver task; loopback + loss-injection
  integration tests.
- **FileCC window congestion control** (§5.2, #620) — `filecc::FileCc`, the
  file/bulk-transfer-mode sibling of `LiveCC`: hybrid AIMD Slow Start +
  Congestion Avoidance, the full NAK-driven rate-decrease state machine, and
  the `MAX_BW`-derived `MIN_PERIOD` clamp. A pre-tag audit found and fixed a
  measure-zero float-comparison bug in `DecRandom` (now rounded to the
  nearest whole number); a documented (not "fixed") one-shot quirk in the
  draft's own Step 4 pseudocode is called out in the module doc.
- **Crypto wired into the handshake, plus KM Refresh** (§6.1.5/§6.1.6, #621,
  `crypto` feature) — Key Material now piggybacks on the existing
  Caller-Listener CONCLUSION extension flow (`handshake_sm::CryptoConfig`);
  the Listener echoes it back to prove key derivation, and a mismatched
  passphrase or one-sided encryption config is rejected. New
  `km_refresh::KmRefreshDriver` — a sans-IO §6.1.6 SEK-rotation state machine
  (PreAnnounce/Switchover/Decommission), state-tracking only.

Every constant/formula is grep-verified against a curated, spec-cited markdown
transcription (`specs/rules/srt-{arq,tsbpd,livecc,crypto,rendezvous}.md`) before
implementation. The sans-IO core stays `no_std` + `alloc`; `crypto` and `tokio`
are `std`-only, off by default, and pull zero extra dependencies when disabled.

## Fixed — release-audit findings (pre-tag)
- **Crypto counter construction.** The draft gives two unreconciled AES-CTR IV
  formulas; this crate had implemented the `§6.2.2` form's `<< 2` shift
  literally, which is a known draft-text artifact — no real SRT peer could have
  decrypted the result. `packet_counter` now matches the reference
  implementation's `hcrypt_SetCtrIV` exactly (no shift). Added a known-answer
  test that catches this regression.
- **`tokio` adapter loss recovery.** The socket adapter was pull-based (only
  advanced the protocol during an active `send`/`recv` call), so NAK-triggered
  retransmission never progressed once the application went idle — loss
  recovery deadlocked. Rewrote the driver as a background task with a periodic
  tick, made TSBPD the single delivery authority (fixing a dual-cursor reorder
  race), and added a loss-injecting-relay integration test proving end-to-end
  recovery (20/20 deterministic runs).
- Packet pacing now applies to data packets only, never control feedback;
  correct peer socket ID (was the peer's initial sequence number); IO errors
  preserve `ErrorKind` + call-site context instead of collapsing to one variant.
- `#[non_exhaustive]` added to `MaxBwConfig`, `FeedOutcome`, `TickOutcome`,
  `RendezvousRole`.

## Not yet (tracked follow-ups)
Wiring the negotiated SEK / `KmRefreshDriver` events into `io.rs`'s tokio
adapter to actually encrypt/decrypt data-packet payloads end-to-end over a
real socket (the handshake negotiation and rotation state machine are both
sans-IO and fully tested). The Version-4 legacy Rendezvous path is
permanently out of scope (see README), not a follow-up.

## Compatibility
`no_std` core unchanged; MSRV 1.86. Additive minor release — no breaking
changes to the 0.1.0 packet-codec API.
