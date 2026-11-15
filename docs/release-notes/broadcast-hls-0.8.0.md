# broadcast-hls 0.8.0

_Released 2026-11-15._

### Added
- `MediaSequence` (RFC 8216 §4.3.3.2, 64-bit) and `PartIndex` (RFC 8216bis §4.4.4.9 part index, 32-bit) in the new `seq` module: one type per HLS sequence number, with checked arithmetic (`checked_add`, `checked_next`, `checked_sub`, `checked_distance_from`) so a number never wraps silently, `Display` as the bare wire number and (feature `serde`) the bare number on the wire. Added/removed by `1`/`0` through the `From<u64>`/`From<MediaSequence>` (and `u32` counterparts) conversions (#2699).
- `DecimalSeconds::from_duration(Duration)` and `DecimalSeconds::from_duration_scaled(Duration, f64)`: infallible constructors for a value a `Duration` already guarantees valid (finite, non-negative), saturating at a finite maximum instead of overflowing to an infinity the renderer may never emit. An origin computing `PART-TARGET`/`PART-HOLD-BACK` from its part target no longer needs a fallible construction it cannot fail (#2654).

### Changed
- BREAKING: `MediaPlaylist::media_sequence: u64` is `MediaPlaylist::msn: MediaSequence` (same wire tag), and `RenditionReport::last_msn`/`last_part` are a `MediaSequence` and an `Option<PartIndex>`, so a playlist, an LL-HLS segmenter and a client no longer disagree about a sequence number's width (#2699). Migration: wrap a literal with `MediaSequence::new(..)`/`PartIndex::new(..)` and read a number back with `.value()`.
- BREAKING: `LowLatencyConfig`, `PartSpec`, `SkipInfo` and `RenditionReport` are now `#[non_exhaustive]`; build them with `Default::default()` + `with_*` (or `PartSpec::new`/`SkipInfo::new`/`RenditionReport::new`) instead of struct literals (#2664).

---

Published from tag `broadcast-hls-v0.8.0`.
