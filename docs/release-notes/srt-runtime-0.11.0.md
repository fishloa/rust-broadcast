# srt-runtime 0.11.0

_Released 2026-11-17._

### Fixed
- A listener no longer allocates handshake state for an INDUCTION: it answers statelessly from the SYN cookie, and a `PendingListener` is created only when a CONCLUSION proves the Caller received that answer and echoes a cookie this listener issued (under the current or previous key, across the current or previous one-minute bucket). A spoofed INDUCTION flood therefore holds no slots and locks no legitimate caller out, and no datagram path drives pending retransmits any more. The cookie is also bound to the Caller's own SRT Socket ID, so a captured CONCLUSION replayed with a rewritten ID does not validate (#2820).

### Fixed
- A peer re-sending a NAK can no longer grow the driver's outbound DATA queue without bound: a retransmission already queued in it is not queued again, one sequence number is retransmitted at most once per `MIN_RETRANSMIT_INTERVAL` (20 ms), and the queue is capped at the negotiated maximum flow window (a retransmission dropped at the cap stays in the sender's pending set and is offered again). Previously one 28-byte NAK naming the whole send buffer, repeated faster than the paced drain, queued tens of megabytes (#2819).

### Changed
- BREAKING: `arq::Sender::tick` returns `Vec<(u32, Vec<u8>)>` — each retransmission's sequence number alongside its wire bytes — and gains `on_retransmit_sent`/`on_retransmit_dropped`, the caller's signal that a queued retransmission left the outbound queue or was dropped. Migration: `for bytes in sender.tick(now)` -> `for (seq, bytes) in sender.tick(now)`, and call `on_retransmit_sent(seq)` once the datagram is sent (#2819).

### Tests
- Over-limit tests for the protocol caps: a DROPREQ flood stops at `arq::Receiver`'s `MAX_PENDING_SKIPS` and `TsbpdScheduler`'s `MAX_SKIPS`, `MAX_RECENT_ACCEPTS` holds under a CONCLUSION flood, and the negotiated flow window clamps to `MAX_TRACKED_WINDOW` with a packet one past it rejected (#2860).

### Fixed
- A peer re-sending Key Material messages no longer costs one 2048-iteration PBKDF2 run per datagram: the derived KEK is cached by `(Salt, KLen)`, and at most one peer KMREQ per `KMREQ_MIN_INTERVAL` (100 ms) is processed (#2840).
- `arq::Receiver` no longer rescans the whole loss list against every pending DROPREQ skip each time a packet advances the highest sequence number: the newly opened gap is filtered against at most `MAX_PENDING_SKIPS` coalesced ranges (a `BTreeMap`, not a `Vec`), and a DROPREQ ahead of the tracked data or already recorded costs no scan of the loss/out-of-order sets. A peer could previously pin a core at roughly 8 million comparisons per packet with the default window and pending-skip cap (#2821).

---

Published from tag `srt-runtime-v0.11.0`.
