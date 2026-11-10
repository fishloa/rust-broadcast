---
status: DRAFT
---
# media-doctor 0.15.0

_Released 2026-11-10._

Breaking-class release (0.14 -> 0.15): sibling epochs move; no breaking API change of its own.

Version: 0.14.0 -> 0.15.0.

## Added

- `check_container_codec` reports `hvcc-array-nal-type` and `hvc1-array-completeness` warnings.

## Fixed

- No false `dash-static-mpd-missing-duration` when last Period carries `@duration` or MPD carries `@minimumUpdatePeriod`.
- `avcc-sps-mismatch`/`hvcc-sps-mismatch` flag record level only when below SPS level.
- `PatPmtVersionCheck` drops PMT failing CRC-32.
- Shared codec PSI discovery skips bad PMTs and ignores reserved PAT PIDs.
- Open SCTE-35 event set capped at 1024 ids.
- Byte-identical `splice_info_section` treated as retransmission.
- `CcAnomalyCheck` flags adaptation-only packet with wrong CC.
- Unparseable `#EXT-X-TARGETDURATION`/`#EXTINF` no longer read as 0.
- ISOBMFF sniff accepts compressed `!mov`/`!mof`.

## Dependencies

- `broadcast-common` 10 -> 11
- `transmux` 0.30 -> 0.31

---

Published from tag `media-doctor-v0.15.0`.
