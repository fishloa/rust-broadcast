# transmux 0.15.3

**Release date:** 2026-07-12

Patch release adding an SSAI end-to-end example and fixing two bugs in the
DASH output and splice machinery.

## What's new

- `ssai_ad_stitch` example: a runnable SSAI walkthrough wiring
  `scte35-splice`, `timed-metadata`, and transmux's own `splice_insert` +
  HLS/DASH packaging together. Covered by an integration test asserting on
  the exact rendered manifest text and a full `emsg` round-trip.
- Fixed `splice::splice_insert` mis-scaling the cut point on non-anchor
  tracks whose media timescale differs from the anchor (video) track's.
  The video-timescale offset is now rescaled into each track's own timescale
  before searching for the split sample.
- Fixed DASH output emitting every Representation's segments as the full
  multi-track CMAF artifact instead of genuinely single-track segments.
  Each track's segments are now muxed from a filtered single-track `Media`.
- Internal RTCP codec (`transmux::rtcp`) replaced with a re-export of the
  new standalone `rtcp-packet` crate; no public API change.

## Migration

No breaking changes.
