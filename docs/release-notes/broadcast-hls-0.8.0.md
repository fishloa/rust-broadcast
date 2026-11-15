# broadcast-hls 0.8.0

_Released 2026-11-15._

Minor-class release (0.7 -> 0.8). A media sequence number is one type end to end, so a playlist, an LL-HLS segmenter and a client no longer disagree about its width, and an origin that renders a duration no longer needs a fallible construction it cannot fail.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

### Sequence numbers are `MediaSequence`/`PartIndex`

`MediaPlaylist::media_sequence: u64` is `MediaPlaylist::msn: MediaSequence` (the same wire tag), and `RenditionReport::last_msn`/`last_part` are a `MediaSequence` and an `Option<PartIndex>`. The new `seq` module gives one type per HLS sequence number, with checked arithmetic (`checked_add`, `checked_next`, `checked_sub`, `checked_distance_from`) so a number never wraps silently, `Display` as the bare wire number and (feature `serde`) the bare number on the wire. `From<u64>`/`From<MediaSequence>` (and the `u32` counterparts) add and remove `1`/`0`.

```rust
// before
let playlist = MediaPlaylist { media_sequence: 42, .. };
let msn: u64 = playlist.media_sequence;
let report = RenditionReport { last_msn: 42, last_part: Some(3), .. };
// after
let playlist = MediaPlaylist { msn: MediaSequence::new(42), .. };
let msn: u64 = playlist.msn.value();
let report = RenditionReport { last_msn: MediaSequence::new(42), last_part: Some(PartIndex::new(3)), .. };
```

### The remaining public types are `#[non_exhaustive]`

`LowLatencyConfig`, `PartSpec`, `SkipInfo` and `RenditionReport` are `#[non_exhaustive]`; build them with `Default::default()` + `with_*` (or `PartSpec::new`/`SkipInfo::new`/`RenditionReport::new`) instead of struct literals.

```rust
// before
let spec = PartSpec { uri: "seg0.1.m4s".into(), duration, independent: true, byte_range: None, gap: false };
// after
let spec = PartSpec::new("seg0.1.m4s".into(), duration).with_independent(true);
```

## Added

- `seq` module: `MediaSequence` (RFC 8216 §4.3.3.2, 64-bit) and `PartIndex` (RFC 8216bis §4.4.4.9, 32-bit).
- `DecimalSeconds::from_duration(Duration)` and `DecimalSeconds::from_duration_scaled(Duration, f64)`: infallible constructors for a value a `Duration` already guarantees valid (finite, non-negative), saturating at a finite maximum instead of overflowing to an infinity the renderer may never emit. An origin computing `PART-TARGET`/`PART-HOLD-BACK` from its part target no longer needs a fallible construction it cannot fail.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `broadcast-hls-v0.8.0`.
