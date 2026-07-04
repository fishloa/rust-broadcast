# Changelog

All notable changes to `srt-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Sans-IO ARQ (Automatic Repeat reQuest) reliability engine** (§4.8
  Acknowledgement and Lost Packet Handling, §4.8.1 ACKs/ACKACKs, §4.8.2 NAKs,
  §4.10 Round-Trip Time Estimation; issue #606), driving the existing
  ACK/NAK/ACKACK/Data packet codecs — no wire format is re-encoded:
  - `arq::Sender` — buffers every sent data packet (rule 1); `on_nak`
    records a NAK's loss-list entries for prioritized retransmission (rules
    5, 15, 16, 18); `tick` drains the pending retransmit queue, setting the
    `R` flag and incrementing the resend counter; `on_ack` frees every
    packet acknowledged by the ACK's `n + 1` cumulative semantics (rule 8)
    and, for a Full ACK only, updates RTT/RTTVar from the ACK's carried
    value (rule 33) and returns the ACKACK reply (rules 3, 9).
  - `arq::Receiver` — tracks arrivals and a cumulative ack point;
    `feed_data` detects newly-opened sequence gaps and returns an immediate
    NAK (rules 4, 14) plus the sequence numbers that became
    in-order-deliverable as a result (`FeedOutcome::delivered`); `tick`
    emits a Full ACK every 10 ms (rule 11), a Light ACK once 64 packets have
    arrived since the last ACK (rule 12), and a periodic NAK once
    `NAKInterval = max((RTT + 4*RTTVar)/2, 20ms)` has elapsed and the loss
    list is non-empty (rules 21-22) — never a NAK when nothing is lost;
    `on_ackack` matches an ACKACK against its outstanding Full ACK and
    updates RTT/RTTVar from the measured round trip (rules 26-30).
  - `arq::rtt::RttEstimator` — the rule 29-31 RTT/RTTVar EWMA (`RTT = 7/8 *
    RTT + 1/8 * rtt`, `RTTVar = 3/4 * RTTVar + 1/4 * abs(RTT - rtt)`,
    microseconds, initial 100 ms / 50 ms), shared by both roles.
  - `arq::seq` — wrap-safe 31-bit sequence-number arithmetic (circular
    comparison/increment, comparable to RFC 1982 serial number arithmetic;
    the draft does not itself specify a comparison algorithm, so this is
    implementation-defined, documented as such).
  - Timing is entirely caller-driven (`now: core::time::Duration` passed to
    every `tick`/`feed_data`/`on_data`/`on_ack`/`on_ackack` call) — no
    wall-clock read anywhere in the crate.
  - `tests/arq_recovery.rs` — an in-memory `Sender`<->`Receiver` wiring (no
    sockets) that drops two packets in transit, asserts the receiver's NAK
    triggers sender retransmission, all packets are ultimately delivered in
    order, the ACK/ACKACK exchange advances the sender's acknowledged
    sequence and frees its send buffer, RTT converges toward an injected
    30 ms round trip on both sides, and a zero-loss run never emits a
    spurious NAK (from either the immediate or periodic path).
  - Explicit non-goals, unchanged from prior releases: TLPKTDROP fake-ACK
    skip handling, RTO-based/congestion-control retransmission, send-queue
    overflow sizing, TSBPD delivery timing.
- **§6 SRT payload encryption primitives** (issue #608), behind a new
  non-default `crypto` feature (zero new dependencies for the default/no_std
  packet-codec core):
  - `crypto::aes_ctr_apply` — AES-CTR payload encrypt/decrypt (self-inverse).
    The per-packet counter (`crypto::packet_counter`) is derived from the Key
    Material `Salt` and the data packet's Packet Sequence Number per the
    §6.2.2/§6.3.2 formula `IV = (MSB(112, Salt) << 2) XOR PktSeqNo` — the
    draft gives a second, textually different formula in §6.1.2 that this
    crate deliberately does *not* implement; both are transcribed and the
    conflict documented in `specs/rules/srt-crypto.md` and the `crypto`
    module doc.
  - `crypto::wrap_sek` / `crypto::unwrap_sek` — RFC 3394 AES key wrap/unwrap
    of the SEK under the KEK (§6.1.5/§6.2.1/§6.3.1), split as
    `(icv, wrapped)` to match `packet::KeyMaterial`'s `icv`/`x_sek`/`o_sek`
    fields; supports wrapping one or two concatenated SEKs (`KK` = even/odd
    vs. both).
  - `crypto::derive_kek` — KEK derivation from a pre-shared passphrase via
    PBKDF2 (HMAC-SHA1, 2048 iterations) per §6.1.4/§6.2.1/§6.3.1, salted with
    the Key Material `Salt`'s low 64 bits (`LSB(64,Salt)`).
  - `crypto::select_sek` — picks the even/odd SEK for a data packet from its
    `KK` field (`packet::EncryptionKeyField`).
  - AES-128/192/256 (`KLen` 16/24/32 bytes) supported throughout, selected by
    key length at runtime.
  - Uses the `aes`/`ctr`/`aes-kw`/`pbkdf2`/`hmac`/`sha1` RustCrypto crates
    (all `no_std`) — no hand-rolled crypto.
  - `tests/crypto_vectors.rs` — external ground truth, not spec-vector-free
    self-checks: the RFC 3394 §4.1 worked key-wrap vector (byte-exact wrap
    *and* unwrap), the NIST SP 800-38A Appendix F.5.1 CTR-AES128 vector
    (byte-exact both directions), and an SRT-specific SEK+Salt+PktSeqNo
    payload round-trip including a wrong-SEK-does-not-recover negative case.
    `draft-sharabayko-srt-01` §6 has no test vectors of its own
    (`specs/rules/srt-crypto.md`).
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
  - Explicit non-goals, unchanged from `0.1.0`: the Rendezvous handshake
    (§4.3.2), ARQ/loss, TSBPD delivery, congestion control, AES key-wrap/
    unwrap crypto, and a `tokio` socket adapter.

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
