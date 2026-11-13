# mpeg-ts 0.8.0

_Released 2026-11-14._

Major-class release (0.7 -> 0.8) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.7.0 -> 0.8.0.

## Added
- `SectionReassembler::feed_with_continuity(payload, pusi, continuity_counter, discontinuity)`: `feed` plus the packet's `continuity_counter` and adaptation-field `discontinuity_indicator`, discarding a legally retransmitted (same-counter) continuation packet instead of appending it twice (ISO/IEC 13818-1 §2.4.3.3); with `discontinuity` set an unchanged counter is legal and the packet is processed.
- `TsResync::take_tail_packets`: drains the aligned packets `finish` recovered at end of input (`finish` keeps its signature).

## Fixed
- `SectionReassembler` (via `feed_with_continuity`) no longer loses a multi-packet section to a retransmitted continuation packet; `dvb-si`'s `SiDemux` uses it.
- `TsResync::finish` keeps the aligned packets that never reached a full `LOCK_CONFIRMATIONS` window (the tail after a corrupt sync byte, or a whole stream shorter than the window) instead of dropping them.

## Also in this release

### Added
- Stateless datagram resync helpers `resync`, `resync_confirmed`, `unconfirmed_candidate` moved from `dvb-stream` (lone sync byte accepted when the buffer cannot hold a confirming packet) (#2479).
- `ts_dup` module moved from `broadcast-common` (legal duplicate TS packet detection, H.222.0 §2.4.3.3) (#2466).
- `TS_PACKETS_PER_DATAGRAM` (7) and `TS_BYTES_PER_DATAGRAM` (1316) — the whole-packet TS payload of one IP datagram, the size a TS-over-UDP/SRT/RTP sender writes — and `MAX_DATAGRAM_PAYLOAD` (1472), the Ethernet-MTU datagram they fit. The SRT push, the media-doctor watch test and multimux's UDP ingest test now share them instead of a local `7 * 188` (issue #2369).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `mpeg-ts-v0.8.0`.
