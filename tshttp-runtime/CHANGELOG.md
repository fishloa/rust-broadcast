# Changelog

## [Unreleased]

## [0.1.0] - 2026-10-31
### Added

- `TsHttpClient::set_retry_after` / `TsHttpPolicy::set_retry_after`: a server-requested delay (milliseconds) that raises the next reconnect wait to at least that long; consumed by that failure, cleared by a successful response.
- `TsHttpPolicy` (`policy` module): the demux-free policy core of the session (every transition of `docs/tshttp-runtime/DESIGN.md`), driven by `on_response`/`on_bytes(len, now, sampled)`/`on_sample`/`on_eof`/`on_error`/`reconnect`/`poll_timeout[_with]`, plus `accepts_bytes`, `is_ended`, `is_finite`, `open_count` and `push_event`. `TsHttpClient` is now this core plus a `StreamingTsDemux`; its public API and behaviour are unchanged. Covered by `tests/policy_core.rs`, which drives both over a real fixture and requires identical decisions.
- `tshttp-runtime` 0.1.0: sans-IO TS-over-HTTP pull session client.
  - `TsHttpClient` with `poll`/`on_response`/`feed`/`on_eof`/`on_error`/
    `reconnect`/`poll_timeout`/`next_deadline`/`poll_event`.
  - `Config`, `Action`, `Event`, `StarveKind`.
  - `no_std` + `alloc`; caller-supplied millisecond time.
- `Config::validate()` / `ConfigError`, `Config::clamped()`, `Config::min_reconnect_interval_ms`; `TsHttpClient::new` uses the clamped config.

### Fixed

- A zero `backoff_ms` entry made the session reconnect in the same instant: `Config::validate` now rejects it (`ConfigError::ZeroBackoffEntry`) and `Config::clamped` (used by `TsHttpClient::new`) raises it to 1 ms.

- `next_deadline` no longer returns a time in the past after the first stall notice (hosts busy-looped until `stall_reconnect_ms`); bytes arriving after a notice restart the notice schedule.
- The start timer now runs from `Open`: a server that accepts but never sends response headers is aborted and retried after `start_timeout_ms` (T20, `next_deadline` is `Some` while Opening).
- A server that sends a sample and closes immediately can no longer cause a reconnect spin: `Config::min_reconnect_interval_ms` (default 1000) spaces opens after a short played-then-ended connection; long-lived streams still reconnect with 0 ms.
- Stall catch-up emits at most one `StallNotice` per `poll_timeout` call (carrying the current stalled seconds) and uses saturating arithmetic.
- A 206 whose `Content-Range` last byte is not below the total, and any finite response with a `Content-Encoding` other than identity, are treated as live, not finite.

### Changed

- `Event` no longer implements `PartialEq`/`Eq` (the former Debug-string comparison of `Event::Demux` was not structural) and, with `serde`, serializes only (`Deserialize` removed: `Event::Demux` could never round-trip).
- `Config` has a new public field `min_reconnect_interval_ms`.
