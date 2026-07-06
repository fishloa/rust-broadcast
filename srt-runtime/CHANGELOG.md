# Changelog

All notable changes to `srt-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`filecc` — SRT File Transfer Congestion Control (FileCC), §5.2** (issue
  #620). [`filecc::FileCc`] is the file/bulk-transfer-mode sibling of
  [`livecc::LiveCC`] (§5.1): a two-phase hybrid AIMD window + pacing
  controller. Slow Start (§5.2.1.1) grows `CWND_SIZE` by the ACK
  sequence-number delta each full ACK, holding `PKT_SND_PERIOD` fixed at 1
  microsecond, until the first loss/timeout or `CWND_SIZE` exceeding
  `MAX_CWND_SIZE` ends it. Congestion Avoidance (§5.2.1.2) recomputes
  `CWND_SIZE` directly from `RECEIVING_RATE`/`RTT` each ACK, and runs the
  full NAK-driven rate-decrease state machine: the 2%-loss-ratio tolerance,
  the `LastDecSeq`-bounded congestion-period detection, the `1.03x`
  repeat-decrease backoff (bounded by `DecCount<=5`), the `AvgNAKNum` EWMA,
  and the `MAX_BW`-derived `MIN_PERIOD` clamp. Sans-IO, `no_std` (including
  `thumbv7em-none-eabi`): driven by `on_ack`/`on_loss`/`on_timeout`, no
  wall-clock reads. Curated at `specs/rules/srt-congestion.md`, every
  constant/formula cited to a source line. The draft's two flagged gaps
  (the `RECEIVING_RATE`/`EST_LINK_CAPACITY` EWMA smoothing weight; the
  packet-pairs probing mechanics that would produce those inputs) are
  resolved with a documented choice in the module doc, not silently
  invented. `DecRandom` (Step 4's repeat-decrease staggering factor) is
  rounded to the nearest whole number — found by a pre-tag audit: a raw
  fractional draw made Step 4's `NAKCount == DecCount * DecRandom` gate a
  measure-zero float comparison, since both counters are integers. Also
  documented (not "fixed", since it's a property of the draft's own literal
  Step 4 pseudocode, verified against `libsrt`'s differing reference
  formulation): once the immediate post-reset check fails (`DecRandom != 1`),
  neither counter is touched again for the rest of the congestion period, so
  Step 4 fires at most once per period unless the drawn `DecRandom` rounds
  to exactly `1`.
- **Payload encryption wired into the handshake, plus a KM Refresh driver**
  (issue #621, `crypto` feature). `draft-sharabayko-srt-01` §6.1.5 Key
  Material Exchange is now piggybacked on the existing Caller-Listener
  CONCLUSION extension flow instead of a new wire message: opt-in
  `handshake_sm::CryptoConfig` (a pre-shared passphrase plus a
  caller-supplied fresh Salt/SEK — this crate's sans-IO core still never
  reads OS randomness, matching `derive_cookie`'s existing precedent) on
  `HandshakeConfig::crypto`. When set, `CallerHandshake` derives a KEK
  (`crypto::derive_kek`), wraps its SEK (`crypto::wrap_sek`), and sends it as
  a `packet::KeyMaterial` `SRT_CMD_KMREQ` extension on its CONCLUSION;
  `ListenerHandshake` unwraps it (`crypto::unwrap_sek`) and echoes the same
  Key Material back as `SRT_CMD_KMRSP` to confirm (§6.1.5, "the responder
  echoes the same KM message back to prove it derived the same SEK"); the
  Caller verifies the echo before trusting it. The negotiated SEK/Salt are
  exposed via new `handshake_sm::NegotiatedParams::sek`/`salt` fields. A
  mismatched passphrase fails the RFC 3394 wrap-integrity check and rejects
  the connection (`RejectionReason::BadSecret`); one side configured for
  encryption and the other not rejects as `RejectionReason::Unsecure`.
  - New `km_refresh` module (`crypto` feature): a sans-IO §6.1.6 KM Refresh
    (SEK-rotation) driver, `km_refresh::KmRefreshDriver`. Driven by
    `on_packet_sent(n)`/`tick()` (no wall-clock reads), it fires
    `KmRefreshEvent::PreAnnounce` at `refresh_period - pre_announcement_period`
    packets, `Switchover` at `refresh_period`, and `Decommission` at
    `refresh_period + pre_announcement_period`, alternating `KeyParity`
    (`Even`/`Odd`) and keeping both keys valid through the transition window
    — the spec-recommended thresholds (`2^25` / `4000`) are
    `KmRefreshThresholds::RECOMMENDED`. The driver tracks state only; actual
    SEK generation/wrap/send in response to `PreAnnounce` is the caller's
    job, mirroring `CryptoConfig`'s caller-supplied-randomness design.
  - Wiring the negotiated SEK / `KmRefreshDriver` events into `io.rs`'s
    tokio adapter to actually encrypt/decrypt data-packet payloads
    end-to-end is a follow-up — this release wires the handshake negotiation
    and ships the rotation state machine, both sans-IO and fully tested.

## [0.2.0] - 2026-07-06

### Fixed

- **Tokio UDP adapter (`io.rs`) — loss recovery now genuinely works end-to-end**
  (release-audit findings S1–S4). The adapter previously could not recover lost
  packets over a real socket; the driver loop, packet pacing, dest-socket-id,
  and error mapping were all wrong.
  - **Background driver task per connection (S2 root cause).** `SrtSocket` is now
    a handle over a background task that runs a `tokio::select!` loop
    (socket RX / application-send / periodic `tokio::time::interval` tick). The
    old pull-based `send`/`recv` only advanced the protocol while the app was
    inside a call, so a fire-and-forget sender went dormant and never drained
    inbound NAKs or emitted retransmissions — loss recovery deadlocked. The
    periodic tick arm keeps retransmit/ACK/NAK progressing regardless of
    application call timing. Retransmits (drained from the NAK loss list by
    `arq::Sender::tick`) are queued ahead of new first-time data each cycle,
    preserving the spec's loss-list-before-first-transmission priority
    (`draft-sharabayko-srt-01` §4.8.2, rules 5/15/16).
  - **Single in-order delivery cursor.** Delivery is now driven solely by the
    TSBPD scheduler; the ARQ receiver drives reliability (loss detection / NAK /
    ACK point) only. Previously both cursors delivered from one staging map and
    raced, reordering retransmitted packets. TLPKTDROP is disabled in the
    adapter so a NAK-recovered gap is waited for, not skipped — `recv` delivers
    every payload in order.
  - **LiveCC pacing applied to DATA packets only (S1).** `PKT_SND_PERIOD` (§5.1)
    now paces original/retransmitted DATA packets and never throttles
    ACK/NAK/ACKACK/Keep-Alive control feedback (which loss recovery rides on).
    `LiveCC` is fed payload sizes at each data send and the send period is read
    where a data packet is actually emitted, instead of being misapplied once
    per flush to every datagram.
  - **Correct peer socket id + real SYN cookie (S3).** Outgoing packets now use
    the peer's *negotiated SRT Socket ID* (`NegotiatedParams::peer_socket_id`)
    as `dest_socket_id`, not its initial sequence number. The listener's SYN
    cookie is derived via `handshake_sm::derive_cookie` from the peer address, a
    1-minute time bucket, and a per-listener random secret (§4.3.1.1), replacing
    the hard-coded `0xC0FFEE42` constant.
  - **I/O errors preserve context (S4).** A new `Error::Io { kind, context }`
    variant carries the `std::io::ErrorKind` and the failing call site
    (`bind`/`connect`/`recv`/`send`/…), so e.g. a bind failure is
    distinguishable from a mid-connection reset — replacing the previous
    flatten-everything-to-`InvalidField{reason:"io error"}`.
  - New `tests/io_loss_recovery.rs`: a loss-injecting UDP relay drops a
    deterministic subset of first-time DATA packets between caller and listener;
    the test sends 40 payloads and asserts all arrive in order, byte-identical,
    proving NAK→retransmit recovery *through `io.rs`* (wrapped in a 15 s
    `tokio::time::timeout`).

- **`#[non_exhaustive]` on forward-evolving public types** (release-audit
  dimension F): `livecc::MaxBwConfig`, `arq::FeedOutcome`, `tsbpd::TickOutcome`,
  and `rendezvous::RendezvousRole`.

### Added

- **Tokio UDP socket adapter** (feature `tokio`, issue #611): real-socket async
  SRT connection over UDP that drives the existing sans-IO engines end-to-end:
  [`io::SrtSocket`] (caller `connect` / listener `accept`) and
  [`io::SrtListener`] (UDP bind + accept loop). Handshake (caller/listener) →
  ARQ data transfer with retransmit/ACK/NAK → TSBPD-ordered delivery with
  LiveCC pacing, all behind a single `send`/`recv` async interface. Behind a
  new non-default `tokio` feature (implies `std`); the sans-IO core stays
  `no_std` without it.
  - `io::SrtSocket::connect` — bind + HSv5 caller handshake to remote peer.
  - `io::SrtListener::bind` / `accept` — listen for incoming SRT Callers.
  - `SrtSocket::send` / `recv` — async application payload transfer.
  - `tests/io_loopback.rs` — full loopback integration test: listener binds
    ephemeral 127.0.0.1 port, caller connects, sends N≥20 distinct payloads,
    receiver gets ALL N in order (byte-identical), wrapped in
    `tokio::time::timeout` for fail-fast on deadlock.

- **Sans-IO TSBPD delivery scheduler + too-late packet drop** (§4.5/§4.6/§4.7
  of `draft-sharabayko-srt-01`; curated at `specs/rules/srt-tsbpd.md`, issue
  #607):
  - `tsbpd::TsbpdScheduler` — receiver-side delivery scheduler: `feed_data`
    accepts a packet's sequence number and 32-bit timestamp, computes its
    `PktTsbpdTime` per the rule-9 formula; `tick` releases packets in
    sequence order when their play time has arrived.
  - `PktTsbpdTime = TsbpdTimeBase + PKT_TIMESTAMP + TsbpdDelay + Drift` with
    spec-cited constants: minimum `TsbpdDelay` 120 ms (rule 10),
    `TLPKTDROP_THRESHOLD` default `1.25 × TsbpdDelay` (rule 19).
  - Too-late drop on arrival: a packet whose `PktTsbpdTime` is already past
    `(now - TLPKTDROP_THRESHOLD)` is dropped immediately.
  - Too-late drop via release loop: buffered packets past the drop threshold
    are dropped when the gap ahead of them is filled (rule 21 pseudocode).
  - 32-bit timestamp wrapping handled via lossless u64 arithmetic — no
    wrapping-period TsbpdTimeBase adjustment (rule 16 is separate, driven by
    the handshake layer, not implemented here).
  - Sans-IO (`core::time::Duration`) throughout — no wall-clock read in the
    crate.
  - `tests/tsbpd_delivery.rs` — 12 integration tests: ordered/out-of-order
    delivery, withholding until play time, too-late drop on arrival, drop
    chain after gap fill, timestamp wrap, disabled drop, gap blocking,
    custom threshold, drift inclusion, gradual tick, duplicate suppression.
  - Explicit non-goals: drift estimation (§4.7), fake-ACK on receiver skip
    (rule 22), sender-side TLPKTDROP (rule 18-20), wrapping-period
    TsbpdTimeBase adjustment (rule 16).
- **Sans-IO LiveCC packet pacing controller** (§5.1 SRT Packet Pacing and Live
  Congestion Control; issue #610, curated at `specs/rules/srt-livecc.md`):
  - `livecc::LiveCC` — sender-side pacing state: computes the inter-packet send
    period (`PKT_SND_PERIOD`) from the running EWMA average payload size
    (`AvgPayloadSize`) and the configured maximum bandwidth (`MAX_BW`), per the
    `§5.1.2` formulas.
  - `livecc::MaxBwConfig` — three bandwidth-configuration modes (`MAXBW_SET`,
    `INPUTBW_SET`, `INPUTBW_ESTIMATED`) plus `Infinite` (unbounded), per
    `§5.1.1`.
  - `on_data_packet` — updates the `AvgPayloadSize` EWMA (`7/8 * old + 1/8 *
    packet`, L3219).
  - `on_ack_received` — computes `PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW`
    (L3234), returning `Duration::ZERO` for infinite bandwidth.
  - Initial `AvgPayloadSize` capped at 1456 bytes (L3222-3223); default
    `MAXBW_SET` at 1 Gbps (L3122-3123).
  - `tests/livecc_pacing.rs` — integration tests that assert hand-computed
    `PKT_SND_PERIOD` constants for known payload and bandwidth values; EWMA
    step-by-step convergence; all three bandwidth modes; runtime mode switching.
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
  - Explicit non-goals, unchanged: TSBPD delivery, congestion control, a
    `tokio` socket adapter, and the Version-4 legacy Rendezvous path.
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
