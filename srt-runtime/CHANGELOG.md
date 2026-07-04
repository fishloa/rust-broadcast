# Changelog

All notable changes to `srt-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Sans-IO Rendezvous handshake state machine** (§4.3.2, issue #609, curated
  at `specs/rules/srt-rendezvous.md`), reusing the same shared
  `handshake_sm` types and packet codecs as the Caller-Listener flow:
  - `rendezvous::RendezvousHandshake` — a single, symmetric engine: both
    peers run the same code. `start()` sends WAVEAHAND (Version 5, this
    side's own cookie); the **cookie contest** (greater cookie wins) resolves
    each side's `rendezvous::RendezvousRole` (`Initiator`/`Responder`) at
    runtime from the first inbound message's cookie. Drives the
    `Waving -> Attention -> Initiated -> Connected` states (Parallel
    Handshake Flow, §4.3.2.2) with the full Initiator/Responder transition
    tables, including the idempotent missing-packet recovery rules
    (§4.3.2.2: a Responder stuck in `Initiated` always re-sends HSRSP on a
    repeated HSREQ; may promote to `Connected` on non-handshake traffic via
    `on_recovery_trigger()`, modeling "as if it had received AGREEMENT").
    The Serial flow (§4.3.2.1) is handled by the same engine, not a separate
    state — see the module docs' "Serial vs Parallel flow" note for why.
  - Identical cookies are surfaced as `RejectionReason::RdvCookie` (Table 7
    code `1009`, "rendezvous cookie collision") rather than an internal
    retry loop.
  - `tests/rendezvous_round_trip.rs` — two `RendezvousHandshake` peers wired
    together in memory (no sockets), both reaching `Connected` with
    cross-matching negotiated socket ids and the greater-of-both latency;
    a deterministic cookie tie-break test; and a malformed-extension-mid-flow
    test asserting a structured rejection, never a panic.
  - `tests/no_panic.rs` extended to fuzz the Rendezvous engine at Waving,
    Attention, and Initiated.
  - Explicit non-goals, unchanged: ARQ/loss, TSBPD delivery, congestion
    control, AES key-wrap/unwrap crypto, a `tokio` socket adapter, and the
    Version-4 legacy Rendezvous path.

- **Sans-IO HSv5 Caller-Listener handshake state machine** (§4.3.1, issue
  #598), driving the existing packet codecs from #565 — no raw handshake
  bytes are hand-encoded:
  - `caller::CallerHandshake` — `start()` builds the INDUCTION handshake
    (Version 4, `Extension Field` `2` per §4.3.1.1's legacy UDT socket-type
    quirk); `feed()` consumes the Listener's INDUCTION response (validating
    Version 5 + the `0x4A17` SRT magic code), builds the CONCLUSION handshake
    (captured SYN Cookie, HSREQ + optional Stream ID / Group Membership
    extensions), then consumes the Listener's CONCLUSION response to reach
    `CallerHandshakeState::Connected` with a `handshake_sm::NegotiatedParams`.
  - `listener::ListenerHandshake` — the mirror: replies to INDUCTION with a
    cookie (`handshake_sm::derive_cookie` is a ready-made, non-standardized
    derivation helper — the draft specifies only the semantic inputs, not a
    wire algorithm); validates the Caller's CONCLUSION (`Handshake Type`,
    `Version`, the echoed SYN Cookie, and every extension block), replying
    with HSRSP + optional Group on success or a Table 7 rejection packet
    (`Handshake Type` = `1000 + code`) on failure.
  - `handshake_sm::RejectionReason` — the full §4.3 Table 7 Handshake
    Rejection Reason set, with `name()` + `Display` (issue #204 convention).
  - `handshake_sm::HandshakeConfig` / `NegotiatedParams` / `HandshakeOutput` —
    the negotiation input/output and driven-engine event type. Latency is
    negotiated as the greater of both parties' TSBPD delay (§4.3.1.2); flags
    as the bitwise AND of both parties' advertised `SRT Flags`.
  - Timeouts/retransmits are modeled as caller-driven `tick()` calls — no
    wall-clock read anywhere in the crate.
  - `tests/handshake_round_trip.rs` — a full in-memory Caller<->Listener
    handshake (no sockets, no bytes touching a network) reaching `Connected`
    on both sides with cross-matching negotiated version/latency/socket
    ids/Stream ID/Group; plus a forged-cookie rejection path asserting the
    Table 7 wire encoding on the rejection packet actually sent.
  - `tests/no_panic.rs` extended to feed arbitrary parsed handshake packets
    into both engines at every state that accepts inbound packets.
  - Explicit non-goals, unchanged from `0.1.0`: ARQ/loss, TSBPD delivery,
    congestion control, AES key-wrap/unwrap crypto, and a `tokio` socket
    adapter. (The Rendezvous handshake, §4.3.2, is no longer a non-goal —
    see above.)

## [0.1.0] - 2026-07-04

Initial scaffold — SRT ([`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01))
packet codecs (issue #565).

### Added

- **Packet dispatch** (`SrtPacket`) — parses the 16-byte SRT header's `F` bit
  to route to a data or control packet (§3, Figure 2).
- **Data packet** (`DataPacket`, §3.1) — sequence number, `PacketPosition`
  (First/Middle/Last/Solo), order flag, `EncryptionKeyField`, retransmitted
  flag, message number, and the opaque payload.
- **Control packets** (`ControlPacket`, §3.2), one struct per Table 1 type:
  - `HandshakePacket` (§3.2.1) — `EncryptionField`, `HandshakeType`, and a
    lazily-walked `HandshakeExtensions` loop (mirroring `dvb-si`'s descriptor
    loop convention) with typed decoders for the Handshake Extension Message
    (`HsExtMessage`, §3.2.1.1), Key Material (§3.2.1.2), Stream ID
    (`as_stream_id`, §3.2.1.3 — including the 32-bit-little-endian-word
    storage quirk), and Group Membership (`GroupMembershipExtension`,
    §3.2.1.4).
  - `KeyMaterial` (§3.2.2) — the full KEKI/Cipher/Auth/SE/Salt/ICV/xSEK/oSEK
    layout, with the `S`/`V`/`PT`/`Sign`/reserved fixed-value fields
    validated (not stored). Carries wrapped-key bytes opaquely — no AES
    key-wrap/unwrap.
  - `KeepAlivePacket`, `CongestionWarningPacket`, `ShutdownPacket` (§3.2.3,
    §3.2.6, §3.2.7).
  - `AckPacket` with `AckCif::{Full,Small,Light}` (§3.2.4), selected by CIF
    length.
  - `NakPacket` (§3.2.5) with lazy `LossListEntry` (Single/Range) decoding
    per Appendix A's sequence-number coding.
  - `AckAckPacket`, `DropReqPacket`, `PeerErrorPacket` (§3.2.8-§3.2.10).
  - `UserDefinedPacket` for Control Type `0x7FFF` / undefined types, with
    `as_key_material()` for the Key Material-over-control-packet delivery
    form.
- Every public spec/field enum (`PacketPosition`, `EncryptionKeyField`,
  `ControlType`, `EncryptionField`, `HandshakeType`, `ExtensionType`,
  `GroupType`, `KmKeyFlag`, `Cipher`, `KmAuth`, `StreamEncapsulation`) has a
  `name()` + `Display` (issue #204 convention), enforced by
  `tests/label_coverage.rs`.
- Reserved/fixed-value fields (`Subtype`, the header `Type-specific
  Information` word where unused, the Key Material fixed fields) are
  validated on parse and not stored — see the crate root's reserved-bit
  policy.
- `tests/no_panic.rs` — a deterministic-PRNG fuzz-smoke test feeding
  truncated/random bytes to every parser and lazy-loop iterator.
- `no_std` + `alloc` core (default `std` feature togglable); no `unsafe`
  (`#![forbid(unsafe_code)]`).

### Explicit non-goals for this release

- Handshake state machine (caller/listener/rendezvous, §4.3).
- ARQ/loss handling, TSBPD, congestion control (§4-§5).
- AES key-wrap/unwrap crypto (§6).
- `tokio` socket adapter.

[Unreleased]: https://github.com/fishloa/rust-broadcast/compare/main...HEAD
