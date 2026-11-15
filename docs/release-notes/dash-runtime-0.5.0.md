# dash-runtime 0.5.0

_Released 2026-11-15._

Minor-class release (0.4 -> 0.5). The client's public wall-clock and media-time surface is `jiff::Timestamp`/`Duration`/`SignedDuration` rather than microsecond integers, every `now` it takes is a `broadcast_common::stage::Timestamp`, and its URLs, byte ranges and base-URL resolution are typed so a caller cannot mix an absolute URL with a relative reference. Two deadline and re-sync bugs are fixed.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

### Wall clocks and media times are `jiff` values

The public wall-clock and media-time surface is `jiff::Timestamp`/`Duration`/`SignedDuration`, not microsecond integers. `address::DynamicContext` is `{ availability_start: jiff::Timestamp, time_shift_buffer_depth: Option<Duration>, availability_time_offset: jiff::SignedDuration, horizon_utc: Option<jiff::Timestamp> }`; `address::PlanInput` is `{ period_start: Duration, period_end: Option<Duration> }`; `address::SegmentRef`/`ResourceRef` carry `start: jiff::SignedDuration`, `duration: Duration` and `available_from`/`available_until: Option<jiff::Timestamp>`; `SegmentPlan::period_start()`/`available_from()`/`available_until()`/`index_at_or_before(jiff::SignedDuration)` replace the `*_us`/`i128` forms; `PlanOptions::horizon_utc`/`with_horizon` replace `horizon_utc_us`/`with_horizon_utc_us`; `timing::period::PeriodTiming` is `{ start: Option<Duration>, end: Option<Duration> }`; `timing::availability::ato_seconds` returns a `jiff::SignedDuration` (replacing `ato_seconds_to_us`); `timing::live_edge::{presentation_delay, join_time}` take and return `Duration`/`SignedDuration` (replacing the `_us` forms); `timing::clock::Anchor::utc` is a `jiff::Timestamp` and `uncertainty` a `Duration`; `UtcClock::utc_at`/`mono_for_utc` take and return `jiff::Timestamp`; and `timing::utc_timing::UtcSample` is `{ mono: Timestamp, utc: jiff::Timestamp, uncertainty: Duration }`. The internal `i128` microsecond arithmetic stays internal.

```rust
// before
let from = plan.available_from_us(now_us);
let index = plan.index_at_or_before(seek_us as i128);
// after
let from: Option<jiff::Timestamp> = plan.available_from();
let index = plan.index_at_or_before(jiff::SignedDuration::from_micros(seek_us));
```

### Every `now` is a `stage::Timestamp`

`DashClient`'s inputs (`poll`, `on_mpd`, `on_mpd_with`, `on_mpd_unchanged`, `on_mpd_failure`, `retry_mpd`, `on_init`, `on_index`, `on_segment`, `on_failure`, `retry`, `skip`, `on_utc_response`, `on_utc_failure`, `set_wall_clock`, `utc_at`) take a `broadcast_common::stage::Timestamp` instead of a `now_us: u64`, and `DashClient::set_wall_clock(now, unix: jiff::Timestamp)` takes the host's wall clock as a `jiff` instant. `DashClient::next_wake()` returns `Option<Timestamp>`. `client::DashConfig`'s spans are `core::time::Duration` (`utc_timeout`, `utc_wait`, `utc_max_rtt`, `host_clock_uncertainty`, `availability_margin`, `early_retry`, `early_grace`, `min_mpd_refresh`; `live_delay: Option<Duration>`), `utc_resync` is `Option<Duration>` (`None` never re-syncs on age, replacing the `u64::MAX` sentinel), and the `DEFAULT_*_US` constants are `DEFAULT_*` `Duration`s. `client::ResponseMeta::sent`/`received`, `client::UtcResponse::sent`/`received`, `FailureClass::NotYetAvailable::retry_at` and `timing::utc_timing::Exchange` are `Timestamp`s, and `Output::ClockSynced::uncertainty` a `Duration`.

```rust
// before
client.poll(now_us);
client.set_wall_clock(now_us, unix_us);
let config = DashConfig { utc_resync: u64::MAX, ..Default::default() };
// after
client.poll(Timestamp::from_micros(now_us));
client.set_wall_clock(Timestamp::from_micros(now_us), jiff::Timestamp::from_microsecond(unix_us)?);
let config = DashConfig { utc_resync: None, ..Default::default() };
```

### URLs and byte ranges are typed

`client::DashClientConfig::new(url)` takes a `url::Url` (not `impl Into<String>`), `DashClientConfig::mpd_url`/`DashClient::mpd_url()`/`DashClient::set_mpd_base` are `url::Url`, and every `client::Action` URL is a `url::Url`. A ranged fetch carries `byte_range: Option<broadcast_common::range::ByteSpan>` (offset plus non-zero length) instead of `Option<(u64, u64)>`.

```rust
// before
let config = DashClientConfig::new("https://example.com/live.mpd");
let range: Option<(u64, u64)> = action.byte_range;
// after
let config = DashClientConfig::new(Url::parse("https://example.com/live.mpd")?);
let range: Option<ByteSpan> = action.byte_range;
```

### Base-URL resolution returns a typed result

`mpd::base_url::resolve`/`resolve_chain` and `mpd::urls::BaseCandidate::resolve` return `Result<ResolvedRef, ResolveError>` (`ResolvedRef::{Absolute(Url), Relative(String)}`, `ResolveError::{ForbiddenChar, SyntheticScheme, Join}`) instead of `Option<String>`, so an absolute URL and a relative reference are distinct and a failure names its cause.

```rust
// before
let resolved: Option<String> = mpd::base_url::resolve(base, reference);
// after
match mpd::base_url::resolve(base, reference)? {
    ResolvedRef::Absolute(url) => { /* an absolute URL */ }
    ResolvedRef::Relative(reference) => { /* join it onto the base */ }
}
```

### Headers are typed and errors carry the cause

`client::ResponseMeta` and `client::UtcResponse` carry `date: Option<headers::Date>`, `etag: Option<headers::ETag>` and `last_modified: Option<headers::LastModified>` (was `Option<&str>`), and `ResponseMeta` is `Clone` but no longer `Copy`. `address::AddressError::Sidx` and `ingest::SessionError::Demux` wrap a `transmux::Error` (via `#[from]`) instead of a formatted `String`, so `AddressError`, `PlanError` and `SessionError` are `PartialEq` but no longer `Eq`.

```rust
// before
let last_modified: Option<&str> = meta.last_modified;
// after
let typed: Option<headers::LastModified> = meta.last_modified;
// from the host's own response:
let raw = headers.get(header::LAST_MODIFIED).and_then(|v| v.to_str().ok());
```

### `Timescale` and `#[non_exhaustive]`

`address::MediaSegment::timescale` and `SegmentPlan::timescale()` are `Timescale` (was `u64`), and `timing::availability::{ticks_to_us, segment_available_from_ticks, segment_available_until_ticks, complete_count_constant}` take a `Timescale`; the zero-timescale branches are gone because a zero `@timescale` is refused by `broadcast-dash` at parse. `ingest::RequestMeta` is `#[non_exhaustive]`; build it with `RequestMeta::new(method, path)` + `with_content_length`/`with_chunked`/`with_body_empty`.

## Fixes

- A live MPD refresh deadline that does not fit the clock is no deadline rather than a saturated one: the refresh interval after an update (and after a stale MPD) is added with `Timestamp::checked_add`, so a host clock near the end of its range goes idle instead of waking on every poll forever.
- The periodic UTC re-sync follows `DashConfig::utc_resync` rather than always the default constant.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-dash` 0.1 -> 0.2, `container-probe` 0.4 -> 0.5, `media-plane` 0.11 -> 0.12, `transmux` 0.31 -> 0.32.
- `quick-xml` is a dev-dependency only (its parser is reached through `broadcast-common::xml`), so it no longer appears in the built dependency graph.

---

Published from tag `dash-runtime-v0.5.0`.
