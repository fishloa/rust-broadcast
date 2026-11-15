# smooth-runtime 0.2.0

_Released 2026-11-15._

Minor-class release (0.1 -> 0.2). The client config is `SmoothClientConfig` with the manifest URL as one of its fields, every span it takes or reports is a `Duration`, its clock is the caller's `stage::Timestamp`, a refused fetch reports the `http::StatusCode` the host already holds, and a retry schedule no schedule can run with fails at construction instead of panicking.

Version: 0.1.0 -> 0.2.0.

## Breaking changes and migration

### `SmoothConfig` is `SmoothClientConfig`, and `SmoothClient::new` takes only the config

`SmoothConfig` is `SmoothClientConfig`, matching `RtspClientConfig`/`RtmpClientConfig`/`HlsClientConfig`, and `SmoothClient::new(manifest_url, config)` is `SmoothClient::new(config)`: the manifest URL is `SmoothClientConfig::manifest_url`, set with `SmoothClientConfig::new(manifest_url)` (the same shape as `HlsClientConfig::new`/`DashClientConfig::new`) or `with_manifest_url(Url)`.

```rust
// before
let client = SmoothClient::new(url.as_str(), SmoothConfig { max_fragment_failures: 3, ..Default::default() });
// after
let client = SmoothClient::new(SmoothClientConfig::new(url.as_str()).with_max_fragment_failures(3));
// or, from a Url already parsed:
let client = SmoothClient::new(SmoothClientConfig::default().with_manifest_url(url));
```

### Spans are `Duration`, waits are `BackoffSteps`

`SmoothConfig`'s `backoff_ms: Vec<u64>` and `backoff_cap_ms: u64` are one `backoff: BackoffSteps` (`with_backoff`), and `refresh_min_us`/`refresh_max_us` are `refresh_min`/`refresh_max: Duration` (`with_refresh_min`/`with_refresh_max`). `DEFAULT_BACKOFF_MS`/`DEFAULT_BACKOFF_CAP_MS` are `DEFAULT_BACKOFF_STEPS`/`DEFAULT_BACKOFF_CAP` and `DEFAULT_REFRESH_MIN_US`/`DEFAULT_REFRESH_MAX_US` are `DEFAULT_REFRESH_MIN`/`DEFAULT_REFRESH_MAX`. `client::FetchFailure::retry_after_us: u64` is `retry_after: Duration`, the server's `Retry-After` wait as a span (RFC 9110 §10.2.3).

```rust
// before
let config = SmoothConfig::default().with_backoff_ms(vec![500, 2_000]).with_refresh_min_us(1_000_000);
let wait: u64 = failure.retry_after_us;
// after
let config = SmoothClientConfig::default()
    .with_backoff(BackoffSteps { steps: vec![Duration::from_millis(500), Duration::from_millis(2_000)], cap: Duration::from_secs(30) })
    .with_refresh_min(Duration::from_secs(1));
let wait: Duration = failure.retry_after;
```

### Every `now` is a `stage::Timestamp`

`poll`, `on_manifest`, `on_fragment` and `on_failure` take a `broadcast_common::stage::Timestamp` (the clock type the server engine already takes) instead of a raw microsecond count.

```rust
// before
client.poll(now_us);
// after
client.poll(Timestamp::from_micros(now_us));
```

### A refused fetch reports a typed status

`client::FetchFailure::status`, `client::Dropped::Fragment::status` and `error::SmoothError::Fragment::status` are `Option<http::StatusCode>` (was `Option<u16>`), so a host reports a refused fetch with the status it already holds, and `client::SYNTHETIC_MOVIE_TIMESCALE` is a `Timescale` (each stream's `Timescale` comes from the manifest, and a zero `TimeScale` is refused by `broadcast-smooth` at parse).

```rust
// before
if failure.status == Some(404) { .. }
// after
if failure.status == Some(http::StatusCode::NOT_FOUND) { .. }
```

### A refused retry schedule fails at construction

`SmoothError::InvalidBackoff(broadcast_common::backoff::InvalidBackoff)`: a config whose retry cap or steps a `RetrySchedule` refuses (a cap above `broadcast_common::backoff::MAX_FLOOR`) makes `SmoothClient::new` fail, and its first output is that `Output::Fatal`, instead of the client panicking or repairing the schedule. `SmoothError` no longer implements `Eq` (it keeps `PartialEq`), because `InvalidBackoff` cannot (a NaN growth factor is not equal to itself).

```rust
// before
if let Err(SmoothError::InvalidBackoff(err)) = result { eprintln!("{err:?}"); }
// after
if let Err(SmoothError::InvalidBackoff(err)) = result { eprintln!("{err}"); }   // Display is the message
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-smooth` 0.1 -> 0.2, `media-plane` 0.11 -> 0.12, `transmux` 0.31 -> 0.32.

---

Published from tag `smooth-runtime-v0.2.0`.
