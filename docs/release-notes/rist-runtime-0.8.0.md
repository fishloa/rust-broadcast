# rist-runtime 0.8.0

_Released 2026-11-17._

### Added
- `arq::Receiver::feed_rtp(&RtpPacket, now)` processes a full RTP packet: a resynchronisation run beyond `max_gap` now confirms only when every packet carries the stream's own SSRC and the RTP timestamps advance by one consistent forward step, so three injected sequence numbers no longer redirect the receiver (#2841).

### Changed
- BREAKING: `arq::Receiver::feed(seq_number, now)` never resynchronises — a bare sequence number carries no identity to tell a genuine sender restart from forged numbers. Migration: call `feed_rtp` with the parsed `rtp_packet::RtpPacket` wherever a jump beyond `max_gap` must be able to resync (#2841).

---

Published from tag `rist-runtime-v0.8.0`.
