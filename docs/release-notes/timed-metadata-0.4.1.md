# timed-metadata 0.4.1

**Release date:** 2026-07-27

Dev-only change: the `ssai_ad_stitch` example and its integration test moved here from `transmux` as part of the circular dev-dependency fix. The example's test helper was adapted to read `Sample::pts` directly instead of reconstructing presentation time. No public API or behaviour change to the library itself.

## Migration

No breaking changes.
