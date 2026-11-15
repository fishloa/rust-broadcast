# hls-runtime 0.14.0

_Released 2026-11-15._

Minor-class release (0.13 -> 0.14). The origin is configured with one required-argument `HlsOriginConfig` instead of a builder chain, its closed segments carry the `Trunk`'s own start and `Duration`, and the sequence numbers, URLs, byte ranges and spans across the client and server are the workspace's shared types. Rendering a playlist no longer has a panicking path.

Version: 0.13.0 -> 0.14.0.

## Breaking changes and migration

### `HlsOrigin` is built from `HlsOriginConfig`

`HlsOriginBuilder` and `HlsOrigin::builder` are replaced by `HlsOriginConfig` and `HlsOrigin::new` (the `SmoothOrigin` shape): the required target duration and window size are `HlsOriginConfig::new(trunk, target_duration, window_segments)` arguments, the optional settings stay `with_*`, and `HlsOrigin::new(config)` builds it. `HlsOriginBuildError::MissingTargetDurationSecs`/`MissingWindowSegments` are gone (the required values cannot be missing). The target duration is a `Duration` (it was `target_duration_secs: f64`), so a non-finite or negative value is unrepresentable; a zero target is still `HlsOriginBuildError::InvalidTargetDuration`, and the rendered `EXT-X-TARGETDURATION` is unchanged (rounded up to whole seconds at the encode edge).

```rust
// before
let origin = HlsOrigin::builder(trunk)
    .with_target_duration_secs(4.0)
    .with_window_segments(6)
    .build()?;
// after
use core::num::NonZeroUsize;
let origin = HlsOrigin::new(
    HlsOriginConfig::new(trunk, Duration::from_secs(4), NonZeroUsize::new(6).unwrap()),
)?;
```

### `ClosedSegment` carries the segment `Duration` and a `MediaTime` start

`ClosedSegment::start` is a `timed_metadata::MediaTime` (was `start_ns: u64`), `ClosedSegment::duration` is a `core::time::Duration` (was `duration_secs: f64`) — the `Trunk`'s own segment duration, carried with no float round-trip — and `ClosedSegment::new` takes both.

```rust
// before
let segment = ClosedSegment::new(msn, start_ns, 4.0, false);
let seconds = segment.duration_secs;
// after
let segment = ClosedSegment::new(msn, media_time, Duration::from_secs(4), false);
let seconds = segment.duration.as_secs_f64();
```

### Part targets and waits are `Duration`

`HlsOriginConfig::with_low_latency` takes the part target as a `Duration` instead of milliseconds, `HlsOrigin::set_part_target_ms(u32)` is `HlsOrigin::set_part_target(Duration)`, and the client's non-blocking-reload hint is `Action::Wait(Duration)` instead of `Action::WaitMs(u64)`. A zero target is still `HlsOriginBuildError::ZeroPartTarget`.

```rust
// before
let config = HlsOriginConfig::new(trunk, 4.0, w).with_low_latency(500);
origin.set_part_target_ms(334);
// after
let config = HlsOriginConfig::new(trunk, Duration::from_secs(4), w).with_low_latency(Duration::from_millis(500));
origin.set_part_target(Duration::from_millis(334));
```

### Sequence numbers are `broadcast_hls::MediaSequence`/`PartIndex`

`client::ResourceId::Part { msn, part }` and `ResourceId::Segment { msn }` carry a `MediaSequence` and a `PartIndex` (was `u64`), `client::BlockingReload` too, and `Error::MediaSequenceOverflow`/`Error::DeltaNotMergeable` carry a `MediaSequence`; `HlsOrigin::current_live_edge` returns `(MediaSequence, Option<PartIndex>)` and `HlsOrigin::next_media_sequence` a `MediaSequence`. The server's wire query (`BlockingQuery::hls_msn: Option<u64>`, `hls_part: Option<u32>`) stays numeric, because parsing the query is the adapter's job. `Debug` still renders the bare numbers, so a log line and the client golden are unchanged.

```rust
// before
let id = ResourceId::Segment { msn: 42 };
let msn: u64 = origin.next_media_sequence();
// after
let id = ResourceId::Segment { msn: MediaSequence::new(42) };
let msn: MediaSequence = origin.next_media_sequence();
let number: u64 = msn.value();
```

### URLs and byte ranges are typed

`HlsClientConfig::new(url)` takes a `url::Url` (not `impl Into<String>`), `HlsClientConfig::playlist_url`/`playlist_base` are `url::Url`, and every `Action` URL is a `url::Url`. `Action::FetchPlaylist` carries a `PlaylistRequest { url, blocking, skip }` (`PlaylistRequest::request_url()` returns the query-augmented `Url`). `HlsClient::set_playlist_base` is removed (use `HlsClientConfig::with_playlist_base`). `Action::FetchResource::byte_range` is `Option<broadcast_common::range::ByteSpan>` (offset plus non-zero length) instead of `Option<(u64, u64)>`.

```rust
// before
let config = HlsClientConfig::new("https://example.com/live.m3u8");
let range: Option<(u64, u64)> = action.byte_range;   // (offset, length)
// after
let config = HlsClientConfig::new(Url::parse("https://example.com/live.m3u8")?);
let range: Option<ByteSpan> = action.byte_range;
if let Some(span) = range { let header = span.range_header(); }
```

### Unknown-number fields are `Option<Url>` and `u64`

`client::VariantInfo::url: String` is `uri: Option<url::Url>`: the absolute URL the manifest declares (`Some` for an HLS master's media playlist or a DASH Representation's Initialization Segment) and `None` for a protocol whose variant has none of its own (Smooth). No empty string stands in for "no URI". `VariantInfo::new` takes the `Option<Url>`, and `bandwidth: u32` is `bandwidth: u64`, so a DASH `@bandwidth` above the `u32` range is carried as parsed rather than saturated.

```rust
// before
let text = &variant.url;
let bw: u32 = variant.bandwidth;
// after
let text = variant.uri.as_ref().map(Url::as_str);
let bw: u64 = variant.bandwidth;
```

### The client is driven by a `stage::Timestamp`

`HlsClient::poll_timeout` takes and returns `broadcast_common::stage::Timestamp` instead of `std::time::Instant`, so the client runs on the same caller-supplied clock as `broadcast-client`'s sources (and from a recorded clock in a test); it is no longer `std`-only.

```rust
// before
let next: Option<Instant> = client.poll_timeout();
// after
let next: Option<Timestamp> = client.poll_timeout(Timestamp::from_micros(now_us));
```

### The remaining public types are `#[non_exhaustive]`

`client::VariantPolicy`, `client::VariantInfo` and `client::RenditionInfo` are `#[non_exhaustive]`; build them with `Default::default()`/`VariantInfo::new`/`RenditionInfo::new` + `with_*` instead of struct literals. `LowLatencyConfig`, `PartSpec`, `SkipInfo` and `RenditionReport` (in `broadcast-hls`) follow the same rule.

## Fixes

- Rendering a playlist no longer has a panicking path: `PART-TARGET`, `PART-HOLD-BACK`, every `EXTINF` and part `DURATION`, and the closed-segment duration come from `DecimalSeconds::from_duration`/`from_duration_scaled` instead of a fallible construction whose invariant the caller had to prove with an `expect`. The window keeps each segment's `Duration` and halves it once, for rendering, rather than storing rounded seconds.

## Dependencies
- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-hls` 0.7 -> 0.8, `container-probe` 0.4 -> 0.5, `media-plane` 0.11 -> 0.12, `mpeg-ts` 0.8 -> 0.9, `timed-metadata` 0.10 -> 0.11, `transmux` 0.31 -> 0.32.

---

Published from tag `hls-runtime-v0.14.0`.
