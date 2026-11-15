# broadcast-dash 0.2.0

_Released 2026-11-15._

### Changed
- BREAKING: the MPD wall-clock fields are `jiff::Timestamp`, not Unix microsecond integers (#2730). `Mpd` is `{ availability_start_time: Option<jiff::Timestamp>, availability_end_time: Option<jiff::Timestamp>, publish_time: Option<jiff::Timestamp> }` (the `_us`-suffixed `i64` fields are removed), `xs_datetime::XsDateTime::unix` is a `jiff::Timestamp` (was `unix_us: i64`), `format_xs_datetime` takes a `jiff::Timestamp`, and `parse_ntp_timestamp` returns a `jiff::Timestamp`. Migration: `Some(unix_us)` -> `jiff::Timestamp::from_microsecond(unix_us)`, `t.unix_us` -> `t.unix.as_microsecond()`.
- `MpdError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports (#2646).
- BREAKING: `BaseInfo::timescale` and `EventStream::timescale` are `Option<Timescale>` (was `Option<u64>` / `Option<u32>`), and `inherit::ResolvedMulti::timescale` and `inherit::DEFAULT_TIMESCALE` are `Timescale`. A present `@timescale="0"` (or one above `u32::MAX`) is `MpdError::Invalid { what: "timescale" }` on every element that carries it, never a silent 1 Hz; an absent one stays `None` in the model, serialises as absent, and resolves to the spec default 1 through the new `BaseInfo::effective_timescale()`. Migration: `Some(90_000)` -> `Some(Timescale::from_nonzero(90_000))`, `multi.timescale` is read with `.get()` (#2657).

### Added
- `BaseInfo::effective_timescale`: `@timescale` with its absent default (1) applied.

---

Published from tag `broadcast-dash-v0.2.0`.
