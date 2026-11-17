# rist-runtime 0.8.0

_Released 2026-11-17._

Minor-class release (0.7 -> 0.8). `arq::Receiver::feed_rtp` resynchronises only when the run carries the stream's own SSRC and the RTP timestamps advance by one consistent step, so injected sequence numbers no longer redirect the receiver. `feed` is now deliberately identity-free and never resynchronises.

## Upgrading

- BREAKING: `arq::Receiver::feed(seq_number, now)` never resynchronises; call `feed_rtp(&RtpPacket, now)` wherever a jump beyond `max_gap` must be able to resync. A bare sequence number carries no identity, so it can no longer tell a sender restart from forged numbers.

## Added

- `arq::Receiver::feed_rtp(&RtpPacket, now)` processes a full RTP packet: a resynchronisation run beyond `max_gap` now confirms only when every packet carries the stream's own SSRC and the RTP timestamps advance by one consistent forward step, so three injected sequence numbers no longer redirect the receiver (#2841).

## Changed

- BREAKING: `arq::Receiver::feed(seq_number, now)` never resynchronises — a bare sequence number carries no identity to tell a genuine sender restart from forged numbers. Migration: call `feed_rtp` with the parsed `rtp_packet::RtpPacket` wherever a jump beyond `max_gap` must be able to resync (#2841).

---

Published from tag `rist-runtime-v0.8.0`.
