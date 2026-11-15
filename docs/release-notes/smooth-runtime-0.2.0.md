# smooth-runtime 0.2.0

_Released 2026-11-15._

### Changed
- BREAKING: `client::FetchFailure::retry_after_us: u64` is `retry_after: core::time::Duration`, the server's `Retry-After` wait as a span rather than a raw microsecond count (RFC 9110 §10.2.3) (#2731).
- BREAKING: `error::SmoothError` no longer implements `Eq` (it keeps `PartialEq`), because its `InvalidBackoff` payload no longer does: `InvalidBackoff` carries a NaN growth factor, which is not equal to itself (broadcast-common #2725). Nothing compared `SmoothError` values for a total order.
- BREAKING: `client::FetchFailure::status`, `client::Dropped::Fragment::status` and `error::SmoothError::Fragment::status` are `Option<http::StatusCode>` (was `Option<u16>`), so a host reports a refused fetch with the status it already holds (#2692).
- Adopts `transmux::smooth_fragment::fragment_track_id`'s `TrackId` return (transmux #2647): a stream's discovered local track id is held as a `TrackId` end to end rather than a raw integer. No public API change.
- BREAKING: `SmoothError::InvalidBackoff(broadcast_common::backoff::InvalidBackoff)`: a config whose retry cap or steps a `RetrySchedule` refuses (a cap above `broadcast_common::backoff::MAX_FLOOR`) makes `SmoothClient::new` fail at construction, and its first output is that `Output::Fatal`, instead of the client panicking or repairing the schedule. Match the variant to surface the config error (#2701).
- BREAKING: `client::SYNTHETIC_MOVIE_TIMESCALE` is a `Timescale`; the client reads each stream's `Timescale` from the manifest (a stream or manifest with a zero `TimeScale` is refused by `broadcast-smooth` at parse, so the client no longer skips such streams) (#2657).
- BREAKING: every `now` the client engine takes is a `broadcast_common::stage::Timestamp` (`poll`, `on_manifest`, `on_fragment`, `on_failure`), the clock type the server engine already takes.
- BREAKING: `SmoothConfig`'s `backoff_ms: Vec<u64>` and `backoff_cap_ms: u64` are one `backoff: broadcast_common::backoff::BackoffSteps` (`with_backoff`), and `refresh_min_us`/`refresh_max_us` are `refresh_min`/`refresh_max: Duration` (`with_refresh_min`/`with_refresh_max`); `DEFAULT_BACKOFF_MS`/`DEFAULT_BACKOFF_CAP_MS` become `DEFAULT_BACKOFF_STEPS`/`DEFAULT_BACKOFF_CAP` and `DEFAULT_REFRESH_MIN_US`/`DEFAULT_REFRESH_MAX_US` become `DEFAULT_REFRESH_MIN`/`DEFAULT_REFRESH_MAX`.
- BREAKING: `SmoothConfig` is `SmoothClientConfig`, matching `RtspClientConfig`/`RtmpClientConfig`/`HlsClientConfig`. Migration: every `SmoothConfig` becomes `SmoothClientConfig` with the same fields and `with_*` setters.
- BREAKING: `SmoothClient::new(manifest_url, config)` is `SmoothClient::new(config)`: the manifest URL is `SmoothClientConfig::manifest_url`, set with the new `SmoothClientConfig::new(manifest_url: impl Into<String>)` (the same shape as `HlsClientConfig::new`/`DashClientConfig::new`) or `with_manifest_url(Url)`. Migration: `SmoothClient::new(url, SmoothClientConfig::default().with_x(..))` -> `SmoothClient::new(SmoothClientConfig::new(url.as_str()).with_x(..))`, or `.with_manifest_url(url)` for a `Url` already parsed.

---

Published from tag `smooth-runtime-v0.2.0`.
