# transmux 0.13.0 — 2026-07-04

Additive (minor). Completes the DASH half of the packager story.

## DASH MPD generation (#566)
`DashPackager` extended (serialize-only, hand-rolled XML, `no_std`):
- **`$Time$`/SegmentTimeline** addressing alongside `$Number$` (`<S t= d= r=>`,
  `$Time$` verified against segment `tfdt`).
- **Dynamic/live profile**: `MPD@type="dynamic"` with `availabilityStartTime`,
  `timeShiftBufferDepth`, `minimumUpdatePeriod`, `publishTime`,
  `suggestedPresentationDelay` (`static` VoD stays default).
- **AdaptationSet content**: `Role`, `@lang`, `ContentProtection` (cenc
  default_KID hook), `InbandEventStream` (emsg). Correct `codecs=` incl. the
  0.12 audio spokes. Every attribute cited to ISO/IEC 23009-1.

## Compatibility
Requires broadcast-common ≥ 8.4. MSRV 1.86. No breaking changes.
