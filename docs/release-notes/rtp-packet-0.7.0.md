# rtp-packet 0.7.0

_Released 2026-11-14._

Major-class release (0.6 -> 0.7) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.6.0 -> 0.7.0.

## Added
- `seq` module: RFC 3550 §A.1 `SeqValidity`/`SeqVerdict` and constants, the single implementation used by transmux and broadcast-client.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `rtp-packet-v0.7.0`.
