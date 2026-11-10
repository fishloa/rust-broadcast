# mpeg-ts 0.7.1

_Released 2026-11-10._

Additive release: a continuity-aware section-reassembly entry point and resync tail recovery, plus the fixes they carry. Nothing published changes signature.

Version: 0.7.0 -> 0.7.1.

## Added
- `SectionReassembler::feed_with_continuity(payload, pusi, continuity_counter, discontinuity)`: `feed` plus the packet's `continuity_counter` and adaptation-field `discontinuity_indicator`, discarding a legally retransmitted (same-counter) continuation packet instead of appending it twice (ISO/IEC 13818-1 §2.4.3.3); with `discontinuity` set an unchanged counter is legal and the packet is processed.
- `TsResync::take_tail_packets`: drains the aligned packets `finish` recovered at end of input (`finish` keeps its signature).

## Fixed
- `SectionReassembler` (via `feed_with_continuity`) no longer loses a multi-packet section to a retransmitted continuation packet; `dvb-si`'s `SiDemux` uses it.
- `TsResync::finish` keeps the aligned packets that never reached a full `LOCK_CONFIRMATIONS` window (the tail after a corrupt sync byte, or a whole stream shorter than the window) instead of dropping them.

---

Published from tag `mpeg-ts-v0.7.1`.
