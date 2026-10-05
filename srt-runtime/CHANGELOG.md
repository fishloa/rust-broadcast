# Changelog

All notable changes to `srt-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-10-05
### Added
- `io::IoConfig` (`#[non_exhaustive]`, `with_*` builders): `max_datagram` (default 1500, clamped to `io::MIN_MAX_DATAGRAM` = 64 ..= `io::MAX_MAX_DATAGRAM` = 65535), `connect` (10 s: resolve + bind), `handshake` (5 s), `read_idle` (5 s), `write` (5 s: every `send_to`). New entry points `SrtSocket::connect_with`/`connect_from_with` and `SrtListener::bind_with`; the old ones use `IoConfig::default()`. `SrtSocket::send_bytes(Bytes)` hands a payload to the driver without a copy.
- `SocketStats::rx_oversize` and `SrtListener::accept_overflow_dropped()`; `arq::Receiver::next_timeout()` and `tsbpd::TsbpdScheduler::next_release_after()` (the `no_std` building blocks of the driver's deadline).
- `SrtSocket::stats()` (returning the new `SocketStats`) and `SrtListener::unrouted_dropped()`
  report datagrams a bounded internal channel dropped because it was full — this adapter's own
  backpressure, not wire-level loss ARQ/TLPKTDROP already account for.
- Typed handshake errors (#1084, r08-SRT-W12): `Error::Rejected(RejectionReason)` carries the
  peer's own reason (a wrong secret is distinguishable from a refused backlog),
  `Error::HandshakeTimedOut { stage }`, `Error::EncryptionUnsupported`, and
  `Error::Handshake { stage, source }`, which keeps the underlying parse/engine error as its
  source instead of flattening it.
- `arq::FeedOutcome::out_of_window` and `arq::Receiver::with_mtu` (r08-SRT-W2/W4); the receiver's
  periodic NAK is sized against the negotiated MTU.
- `tsbpd::TsbpdScheduler::time_base_us()` / `drift_us()` and the constants
  `tsbpd::DRIFT_SAMPLE_COUNT` / `tsbpd::DRIFT_MAX_US` (r08-SRT-W9).
- `handshake_sm::SecretBytes` (`crypto` feature): the type of a negotiated SEK, and the constants
  `handshake_sm::REJECTION_CODE_LIMIT` and `handshake_sm::DEFAULT_MAX_RETRIES`.
- `arq::seq::seq_in_closed_range(seq, first, last)`: circular inclusive-range membership (a
  `last` preceding `first` is an empty range and never matches).
- `io::SocketStats::late_dropped` and `tsbpd::TsbpdScheduler::tlpktdrop_enabled()`.
- `CryptoConfig` and `SecretBytes` implement `zeroize::Zeroize` / `ZeroizeOnDrop`. The `crypto`
  feature now depends on `zeroize` and `subtle`, both already in the workspace lock (#1142).

### Changed (breaking)
- `SrtSocket::recv` now returns `Option<bytes::Bytes>` (was `Option<Vec<u8>>`): each received datagram is one exactly-sized `Bytes` (deliberately not a view into a shared receive chunk, which would pin the whole chunk while any one packet is held) and the application's payload is a slice of it, so a packet is not copied again on the way up (SP6.5). `bytes` and `tokio-util` are new dependencies of the `tokio` feature only; the `no_std` core is unchanged.
- The `tokio` adapter's connection driver no longer ticks on a fixed 2 ms interval: it sleeps until its own next deadline (`Driver::poll_timeout`: next Full ACK/NAK, TSBPD release or too-late skip, keep-alive, peer-idle expiry, pacing slot) or until an event. An idle connection wakes 100 times a second (the 10 ms Full ACK is protocol, rule 11) instead of 500. A late wake now sends the paced packets whose slots passed (at most 16 per wake), so an idle backlog drains at ~16x the old one-packet-per-wake ceiling while pacing still holds.
- `SrtListener` handshakes now progress in a tracked background task: `accept()` only waits for a finished handshake, so a caller connects (and a lost CONCLUSION response is answered again) whether or not anyone is polling `accept()`. Send errors met while answering a peer are counted (`SrtListener::send_errors()`), never queued or returned from `accept()`, so a flood of them cannot displace finished connections. Once the handle is gone the task stops serving new handshakes but keeps the routing pump alive for connections already accepted. At most 64 finished handshakes wait for `accept()`; more are dropped and counted (`SrtListener::accept_overflow_dropped`).
- The `HANDSHAKE_TIMEOUT` (5 s) and `PEER_IDLE_TIMEOUT` (5 s) constants are now `IoConfig::handshake` / `IoConfig::read_idle` (same defaults).
- `tsbpd::TsbpdScheduler::new` takes the time base and the initial drift as signed `i64`
  microseconds (was `u64`): `TsbpdTimeBase = T_NOW - HSREQ_TIMESTAMP` is negative whenever the
  peer's clock is ahead (r08-SRT-W9).
- `NegotiatedParams::sek` is an `Option<SecretBytes>` (was `Option<Vec<u8>>`; it derefs to
  `&[u8]`), is never serialized by the `serde` feature, and `CryptoConfig`, `NegotiatedParams` and
  `KeyMaterial` no longer print passphrases, keys, wrapped keys or ICVs in `Debug` — only lengths
  (#1142). `CryptoConfig` now wipes itself on drop, so its fields can no longer be moved out of
  it, and compares secrets in constant time.
- `KeyMaterial::parse` and `serialize_into` both reject `KK = 00b` ("no SEK provided", which
  §3.2.2 calls invalid): the old parse accepted a message `serialize_into` could not reproduce
  (r08-SRT-W13).
- `RejectionReason::from_handshake_type` only recognises `1000..2^31` as a rejection; libsrt's
  negative `URQ_*` range (for instance `0xFFFF_FFFC`) is no longer reported as
  `Rejected(Reserved(..))` (r08-SRT-W14).
- `HandshakeConfig::default().max_retries` is 12 (was 5): twelve 250 ms retransmits are libsrt's
  3 s connect timeout.
- `handshake_sm::derive_cookie` is SipHash-2-4 keyed by a 128-bit `secret: u128` (was a `u64`
  fed to an unkeyed-strength splitmix mix), so every cookie value changes (r08-SRT-O5). The
  tokio listener draws that key from the OS and replaces it every minute; a handshake begun
  under the previous key still completes, because its pending entry keeps the cookie it was
  issued.
- The tokio adapter's connect/bind/accept errors are the new typed variants instead of
  `Error::InvalidField` (refused crypto is `EncryptionUnsupported`, a peer refusal is `Rejected`,
  an unanswered handshake is `HandshakeTimedOut`, an engine failure is `Handshake`).
- `SrtSocket::send` waits (bounded hand-off channel) while the sender already holds a full flow
  window of unacknowledged packets or the hand-off queue is full, instead of queueing without
  limit; it fails only once the connection has ended (r08-SRT-W6).
- A received Keep-Alive is no longer echoed back (each side now sends its own, §3.2.3).
- `arq::Sender::on_data` returns `Result<Vec<u8>>`: `Error::FieldTooWide` for a `seq` wider than 31
  bits or a `message_number` wider than 26 (nothing is buffered), and it expects `seq` to increase
  (checked by a `debug_assert!`). `on_nak` resolves ranges against the send buffer instead of
  expanding them, ignores a range whose end precedes its start, and examines at most 65 536
  sequence numbers per NAK datagram.
- `arq::Receiver::new` takes a third argument, `max_flow_window: u32` (the Available Buffer Size
  its Full ACK advertises, §3.2.1): `Receiver::new(dest_socket_id, initial_seq, max_flow_window)`.
- `arq::Receiver::feed_data` ignores (`FeedOutcome::out_of_window`) a sequence number wider than
  31 bits or further ahead of the ack point than the flow window (clamped to 262 144), and a
  first packet beyond the peer's ISN now reveals the gap before it.
- `NegotiatedParams` gains `mtu` and `max_flow_window_size`: the smaller of the two sides'
  advertised values (§3.2.1), computed by the Caller, Listener and Rendezvous engines.
- The sans-IO engines refuse a peer ISN wider than 31 bits (`Rejected(Rogue)`): it would seed the
  receive side out of range and stall the connection. The Rendezvous engine returns
  `HandshakeOutOfSequence` instead of panicking on a broken internal invariant.
- `crypto::derive_kek` and `crypto::unwrap_sek` return `Zeroizing<Vec<u8>>` (wiped on drop)
  instead of `Vec<u8>`; the `serde` feature no longer serializes a `KeyMaterial`'s salt, ICV or
  wrapped keys, and `Debug` redacts them (#1142).
- The Caller's Key Material request now carries `SE` = 2 (MPEG-TS/SRT) on the wire (was 0).
- The wire `Timestamp` of every packet the adapter and engines build wraps modulo 2^32 µs
  (~71.6 min) instead of saturating at `u32::MAX` (#1063, listed under Fixed below).
- `KeepAlivePacket`, `CongestionWarningPacket`, `ShutdownPacket`, `AckAckPacket`, and `PeerErrorPacket`
  now carry a `libsrt_pad: bool` field, so both the pure-spec 16-byte (empty CIF) and libsrt's 20-byte
  (4-byte zero-pad CIF) wire shapes round-trip byte-identically.
- `KeyMaterial::serialize_into` now returns `Error::FieldTooWide` instead of silently
  truncating, when a length does not fit its wire field (#1129).
- The tokio adapter's internal per-connection ingress channel, the listener's
  candidate-new-connection queue, and the delivered-payload channel to the application are now
  bounded (dropped and counted on full, via `try_send`) instead of unbounded — a fast peer, or a
  slow/stalled application, could previously grow this process's memory without bound purely from
  network input.

### Fixed
- Defect 3: dropping an `SrtListener` left its UDP port bound for as long as no further datagram arrived (the routing pump only noticed the dropped listener on its next `recv_from`). The pump and the handshake task are now owned by a `TaskTracker` and end on a `CancellationToken` when the last of {listener handle, every accepted connection} is dropped: an idle listener frees its port at once, and connections it already accepted keep being routed until they are gone.
- A datagram larger than `IoConfig::max_datagram` used to be silently truncated by UDP and could then parse as a shorter, valid DATA packet with a corrupted payload; it is now read with one extra byte of room, dropped and counted in `SocketStats::rx_oversize`.
- Every `send_to` is bounded by `IoConfig::write`; resolving and binding by `IoConfig::connect`.
- A DATA packet behind the delivery cursor (a duplicate, a retransmission of what was delivered
  or given up on, or an attacker-chosen stale sequence number stamped with a future timestamp)
  was staged, ignored by TSBPD and never removed, so `staged` grew without bound; it is now
  dropped on arrival and counted in `SocketStats::late_dropped`.
- The adapter honours the negotiated TLPKTDROP flag (§3.2.1.1.1) instead of always skipping
  gaps: without it a missing packet is waited for and the ARQ ack point never moves past it —
  acknowledging a packet that never arrived told the sender it was received. Checked against a
  libsrt caller with `tlpktdrop=0`.
- The adapter runs with the *negotiated* MTU and Maximum Flow Window Size (the smaller of the two
  sides', §3.2.1) for NAK sizing, the receive window and the send window, instead of its own
  configured values.
- The listener's connection epoch (and so its TSBPD time base) is the instant the CONCLUSION
  arrived, not the later instant `accept` hands the connection over, and `SrtListener` no longer
  prints its cookie key in `Debug` (#1142).
- r08-SRT-W1/O3: a NAK no longer costs `O(buffer)` per sequence number (up to 65 537 numbers per
  range entry, ~10^11 comparisons for one NAK against a 10 000-packet buffer): entries are found
  by binary search and a range is intersected with the buffered window. The per-ACK rebuild of a
  `BTreeSet` of the whole buffer is gone.
- r08-SRT-W2: a crafted sequence jump no longer floods the loss list and out-of-order set (~2.6 MB
  of nodes per datagram); the Receiver tracks at most one flow window ahead of its ack point, and
  the adapter advances that ack point past packets TSBPD gives up on (the "fake ACK" of Too-Late
  Packet Drop), so it stops NAKing them and the window follows the delivery cursor.
- r08-SRT-W3: Full ACKs whose ACKACK never arrives are forgotten after `max(4 x RTT, 1 s)`
  (and capped at 1 024) instead of accumulating at 100 per second.
- r08-SRT-W4: a periodic NAK longer than one datagram is split across several NAKs sized to the
  MTU instead of one the socket cannot send (which ended the driver loop).
- r08-SRT-W5: a repeated CONCLUSION from an already accepted Caller (our response was lost) is
  answered with the same response, by `ListenerHandshake` and by `SrtListener`; a repeated
  INDUCTION while awaiting the CONCLUSION is re-answered instead of rejecting the Caller as
  Rogue.
- r08-SRT-W6: dropping an `SrtSocket` sends the peer a SHUTDOWN; an idle connection sends a
  Keep-Alive after one second without sending anything (the 10 ms Full ACK normally keeps it
  from ever being needed).
- #1134: a poisoned `Mutex` can no longer panic the adapter's route-table `Drop` or its routing
  pump (the state behind it is always consistent), so one panicking connection task cannot take
  the listener down.
- r08-SRT-W7: the Caller's Key Material request declares `SE` = MPEG-TS/SRT (2) as libsrt requires
  (a KMREQ with `SE` 0 is refused); verified against a real `srt-live-transmit`, which now
  accepts our key exchange and decrypts our data (and we decrypt libsrt's).
- r08-SRT-W9: the TSBPD time base is seeded from the peer's handshake timestamp (rule 12), not a
  hard-coded 0, and the scheduler estimates drift (§4.7) from fresh in-sequence packets in
  1 000-packet windows, folding what exceeds 5 ms into the time base.
- Both roles of the tokio adapter run TSBPD with the *negotiated* latency — the greater of the two
  parties' requests (§4.3.1.2) — instead of only their own configured one, so a peer that asked
  for a longer latency is no longer played out early.
- A `retransmit_after_ticks` of `u32::MAX` made the adapter's handshake timer period zero and
  panicked `SrtSocket::connect`; the period now has a 1 ms floor.
- r08-SRT-W10: an unanswered handshake request is re-sent every 250 ms (was after a whole 5 s
  receive timeout, three times over: 15 s for one lost INDUCTION), and a duplicate INDUCTION
  response no longer aborts the connect.
- The two TSBPD wrap tests that pretended to cover r08-SRT-W8 (already fixed by #1063's
  `unwrap_timestamp`) could not fail: they ticked at a time at which naive and correct play times
  agree. They now pin the order of release, and a TLPKTDROP-enabled stream crossing the wrap is
  tested end to end.
- A received datagram is staged in its own allocation and handed to the application with its
  header drained, instead of being copied a second time (r08-SRT-O4); a retransmit's payload
  length for LiveCC is read from the packet length instead of re-parsing it (r08-SRT-O2).
- The async adapter's `SrtListener` now demultiplexes its shared socket by Destination Socket ID
  (matching libsrt), not source address: `0` always routes to the accept/handshake path, and any
  other ID must match an accepted connection's own Socket ID *and* its recorded source address, or
  the datagram is dropped. Previously an accepted connection's route-table entry was removed only
  when its driver task exited normally — `SrtSocket::Drop`'s task abort skipped that cleanup
  entirely, leaking the entry (and its channel) for the life of the listener (BLOCKER).
- A `SrtSocket::connect`ed caller's dedicated-socket forwarder task is now aborted when the socket
  is dropped. It previously held its own `Arc<UdpSocket>` clone and looped on `recv_from`
  indefinitely — dropping the `SrtSocket` released only the driver's own reference, so the local
  UDP port stayed bound and a later attempt to bind it again failed with `AddrInUse`.
- A connection with no traffic at all from its peer (not even a Keep-Alive) for 5 seconds — libsrt's
  `SRTO_PEERIDLETIMEO` default — is now torn down instead of the driver task polling a dead
  connection forever.
- The async adapter's receiver-side Full ACK reported `avail_buf_size: 0` (a deliberately
  unfabricated placeholder) unconditionally, which a real libsrt Caller reads as "no receive buffer
  space at all": the handshake completed and Keep-Alives kept flowing, but the Caller silently
  withheld every DATA packet forever. Now reports the negotiated Maximum Flow Window Size, the most
  this receiver will ever have staged ahead of its delivery cursor (the flow-window overflow guard
  enforces that cap independently, so this is never an overstatement).
- The caller-side handshake's CONCLUSION now sends Destination Socket ID `0` (matching a real
  libsrt Caller) instead of the Listener's own captured Socket ID, and takes the negotiated peer
  Socket ID from the CONCLUSION response itself rather than the earlier INDUCTION response —
  sending the captured ID made a real libsrt Listener silently drop the CONCLUSION, stalling the
  handshake forever (libsrt interop).
- `SrtSocket::connect`'s handshake loop no longer fails the whole connect attempt on a stray
  packet from an unrelated source, or one that doesn't parse as a well-formed handshake reply right
  now — only a genuine rejection or retransmit-budget timeout does that.
- ACK/NAK/ACKACK/Keep-Alive control feedback could get queued behind a not-yet-due, LiveCC-paced
  DATA packet in the adapter's single outbound queue and wait out that packet's pacing delay before
  being sent — defeating the point of never pacing control traffic. Outbound DATA and control now
  use separate queues, so control is always flushed in full before DATA pacing is even considered.
- The Message Number counter now wraps to `1`, not `0`: libsrt's public header reserves `0`
  (`SRT_MSGNO_CONTROL`) for its own packet-filter control messages, so a data packet wrapping to
  `0` would have been misread as a filter-control message by a real libsrt peer once every 2^26
  messages.
- A Key Material message's `Salt` over 1020 bytes (`SLen/4` past the 8-bit field's 255 max) no
  longer shifts into the reserved `Resv3` bits — `serialize_into` now rejects it instead of
  emitting a misframed message with `Ok` (#1129).
- Keep-Alive/Congestion Warning/Shutdown/ACKACK/Peer Error control packets now accept the 4-byte
  zero pad a real libsrt peer always appends to these types, instead of rejecting them as
  `UnexpectedTrailingBytes`; we now emit that same pad ourselves for wire compatibility (#1060).
- The async adapter's `flush_outbound` no longer `tokio::time::sleep`s per DATA packet for LiveCC
  pacing — a real sleep, even for a sub-millisecond computed period, blocked for tokio's real
  timer resolution and capped throughput at roughly 1000 pkt/s regardless of the configured
  `MAX_BW`, while also stalling RX for the same window. Replaced with a token-bucket schedule
  serviced by its own non-blocking `select!` arm (#1061).
- The 31-bit Packet Sequence Number and 26-bit Message Number counters now wrap at their own wire
  field width instead of at `u32::MAX` — the old `wrapping_add(1)` let both counters walk past
  their field width (after ~20 h of continuous sending, or immediately with a high initial
  sequence number; after ~67 million messages), after which every packet's
  `DataPacket::serialize_into` returned `Error::FieldTooWide` and panicked the
  `.expect("buffer sized from serialized_len")` call sites in `arq::sender` that assumed only a
  too-small buffer could fail (#1062).
- The wire `Timestamp` field (and its receiver-side `TsbpdScheduler` handling) now wraps modulo
  `2^32` microseconds (~71.58 min) instead of clamping at `u32::MAX` on the sender side, and now
  correctly *un*wraps a real wraparound into an always-increasing value on the receiver side
  instead of naively widening the raw `u32` to `u64` — the old behavior made every packet sent (or
  received from a peer) after that point look impossibly far in the past, so Too-Late-Packet-Drop
  discarded it forever (#1063).
- `SrtListener` no longer shares its bound socket's `recv_from` across `accept()` and every
  accepted connection's driver task, which could hand one connection's datagram to a completely
  different task (silently lost for its rightful recipient) whenever more than one connection (or
  a pending handshake) was active at once. A single routing pump now demultiplexes every inbound
  datagram by source address to the right connection (#1029).
- The Rendezvous cookie contest (`RendezvousHandshake::resolve_role`) now matches libsrt's actual
  `CUDT::backwardCompatibleCookieContest` semantics (a signed 32-bit difference, with a documented
  tie-break at the exact halfway point) instead of a plain unsigned `own_cookie > peer_cookie`
  compare, which disagreed with a real libsrt peer for roughly a quarter of all cookie pairs —
  whenever exactly one of the two cookies had its top bit set (#1064).

### Changed

- Dependency bumps, non-breaking (no public API change): `aes` 0.9, `ctr` 0.10, `aes-kw` 0.3, `pbkdf2` 0.13, `hmac` 0.13, `sha1` 0.11 (RustCrypto 0.13 generation). Key-wrap, PBKDF2 and AES-CTR output is unchanged (known-answer vectors and the libsrt interop tests pass).

- `arq::seq` arithmetic now delegates to `broadcast_common::seq::SeqSpace` (one algorithm, two moduli; public functions and results unchanged) (#1141).


## [0.4.1] - 2026-09-25

### Security
Fixes GHSA-28hg-fc5v-m865, GHSA-gjm5-23jf-293p, GHSA-r6hf-93jv-c3wc and GHSA-7346-x8wq-2rgr.
Upgrade if you use the tokio adapter (`io::SrtSocket` / `io::SrtListener`).

### Changed
- The tokio adapter (`io::SrtSocket`, `io::SrtListener`) now refuses encrypted connections:
  a `HandshakeConfig` with `crypto` set is rejected before any I/O, an incoming handshake that
  requests encryption is rejected, and encrypted data packets are never delivered. The sans-IO
  engines still negotiate keys; the adapter will support encryption once its data path does.
- Too-late packet drop is now enabled by default in the tokio adapter (live mode).

### Fixed
- The tokio adapter handles DROPREQ and caps its receive staging buffer at the flow window, so a
  loss the sender will not retransmit no longer stalls delivery or grows memory.
- `SrtListener` bounds the number of in-progress handshakes and expires them on a timer, even
  under continuous traffic.
- `tsbpd::TsbpdScheduler` with too-late drop enabled no longer waits forever on a missing
  packet: per the receiver-buffer read rule (§4.6), once the next buffered packet's play time
  arrives the gap is skipped and that packet delivered, and `TickOutcome::dropped` now reports
  the skipped sequence numbers from both `feed_data` and `tick`. With too-late drop disabled the
  scheduler still waits for the gap (reliable in-order delivery).
- The tokio adapter (`io::SrtSocket::connect`/`connect_from`, `io::SrtListener`) now generates a
  fresh, random Initial Sequence Number and SRT Socket ID for every connection instead of reusing
  a fixed/default `HandshakeConfig::initial_seq_number` for both, or handing out sequential Socket
  IDs. The sans-IO handshake engines were already caller-driven on both values (unchanged, still
  no `pub` API change); only the tokio adapter's own choice of what to hand them was fixed. The
  adapter's randomness (this and the existing cookie-secret derivation) now comes from the OS
  source via an optional `getrandom` dependency, enabled only by the `tokio` feature, rather than
  `std::collections::hash_map::RandomState` — the `no_std` sans-IO core stays dependency-free.
- The tokio adapter's generated SRT Socket IDs could land anywhere in the full 32-bit range,
  including values a real libsrt peer never allocates (its group-id marker bit set, or its top bit
  set, which reads back negative in libsrt's signed `int`). Generated Socket IDs are now folded
  into the same `1..=0x3FFF_FFFF` range libsrt itself allocates from.

## [0.4.0] - 2026-08-11

### Fixed
- `io::SrtSocket::connect_from` (the tokio Caller adapter) no longer defaults
  the peer's `initial_seq_number` to `0` when it can't be extracted from the
  handshake bytes that just drove the connection to `Connected`. That
  fallback made a genuine peer ISN of 0 indistinguishable from an extraction
  failure, silently mis-seeding ARQ/TSBPD sequence tracking for the whole
  connection — a defect that would surface much later as inexplicable
  loss/reordering, far from its cause. The extraction is now
  `require_peer_isn`, which returns `Error::InvalidField` instead of `0` on
  any parse failure; `SrtSocket::connect`/`connect_from` propagate that error
  instead of silently continuing. No public API previously guaranteed the old
  default, so this is not expected to be observable as a behaviour change in
  practice — see the module's `isn_tests` for why the path is effectively
  unreachable today, and why the guard still matters for future refactors.

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Changed (Breaking)
- `KeyParity` (`km_refresh`), `LossListEntry` (`packet::nak`) now carry
  `#[non_exhaustive]` (issue #806's non_exhaustive drift-guard audit). A
  downstream `match` on either of these now needs a wildcard arm.

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806).

### Fixed
- Doc accuracy (#941 row 6): README install snippet corrected from `"0.2"`
  to `"0.3"` (the crate is 0.3.0).

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.2.0] - 2026-07-06

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
    end-to-end is the one remaining tracked follow-up (see README) — this
    release wires the handshake negotiation and ships the rotation state
    machine, both sans-IO and fully tested.
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
