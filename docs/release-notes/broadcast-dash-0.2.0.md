# broadcast-dash 0.2.0

_Released 2026-11-15._

Minor-class release (0.1 -> 0.2). An MPD's wall-clock fields are `jiff::Timestamp` instead of Unix microsecond integers, and a timescale is the shared `Timescale` on every element that carries `@timescale`, so a consumer never has to decide what a zero means.

Version: 0.1.0 -> 0.2.0.

## Breaking changes and migration

### Wall-clock fields are `jiff::Timestamp`

`Mpd` carries `availability_start_time`, `availability_end_time` and `publish_time` as `Option<jiff::Timestamp>` (the `_us`-suffixed `i64` fields are removed), `xs_datetime::XsDateTime::unix` is a `jiff::Timestamp` (was `unix_us: i64`), `format_xs_datetime` takes a `jiff::Timestamp`, and `parse_ntp_timestamp` returns one.

```rust
// before
let start_us: Option<i64> = mpd.availability_start_time_us;
let unix_us = xs.unix_us;
// after
let start: Option<jiff::Timestamp> = mpd.availability_start_time;
let unix_us = xs.unix.as_microsecond();
let built = jiff::Timestamp::from_microsecond(unix_us)?;
```

### A timescale is `Timescale`

`BaseInfo::timescale` and `EventStream::timescale` are `Option<Timescale>`, and `inherit::ResolvedMulti::timescale` and `inherit::DEFAULT_TIMESCALE` are `Timescale`. A present `@timescale="0"` (or one above `u32::MAX`) is `MpdError::Invalid { what: "timescale" }` on every element that carries it, never a silent 1 Hz; an absent one stays `None` in the model, serializes as absent, and resolves to the spec default 1 through the new `BaseInfo::effective_timescale()`.

```rust
// before
let timescale: Option<u64> = base.timescale;
let rate = multi.timescale;
// after
let timescale: Option<Timescale> = base.timescale;
let rate: Timescale = base.effective_timescale();   // with the absent default applied
let hz: u32 = multi.timescale.get();
```

### The XML failure is typed

`MpdError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports.

## Added

- `BaseInfo::effective_timescale`: `@timescale` with its absent default (1) applied.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `broadcast-dash-v0.2.0`.
