# srt-runtime 0.5.0

_Released 2026-10-05._

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

---

Published from tag `srt-runtime-v0.5.0`.
