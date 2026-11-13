# scte35-splice 6.0.0

_Released 2026-11-14._

Major-class release (5 -> 6) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 5.0.0 -> 6.0.0.

## Fixed
- A typed splice command or descriptor whose declared length exceeds its structure (excess bytes in `splice_null`, `time_signal`, `avail_descriptor`, ...) is now rejected with `Error::InvalidField` instead of being accepted and silently shortened on serialize, which broke the parse-serialize byte round trip.
- `SpliceInfoSection::parse` rejects a set `section_syntax_indicator` or `private_indicator` (fixed at 0 by §9.6.1, Table 5) instead of re-serializing the section with those bits cleared.

## Also in this release

### Added
- `SpliceEvent`, `splice_event`, `judge_splice_time`, `kind_label` and `SpliceSanity`: the cue-sanity engine (cue direction, effective `pts_time + pts_adjustment`, wrap-safe future-vs-past judgement) both live TS monitors now share.
- `SpliceTracker::observe_section` and `SpliceObservation::event`, so a caller that reassembles its own sections gets the same open/closed tracking and the same decoded cue as the payload-level `feed`.
- Shared `SpliceTracker` (moved from media-doctor) for SCTE-35 splice_insert open-event tracking.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.
- New sibling dependencies: `mpeg-ts` 0.8.

---

Published from tag `scte35-splice-v6.0.0`.
