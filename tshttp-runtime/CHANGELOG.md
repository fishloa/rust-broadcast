# Changelog

## [Unreleased]

## [0.1.0] - 2026-11-01
### Added

- `TsHttpClient::set_retry_after` / `TsHttpPolicy::set_retry_after`: a server-requested delay (milliseconds) that raises the next reconnect wait to at least that long; consumed by that failure, cleared by a successful response.
- `TsHttpPolicy` (`policy` module): the demux-free policy core of the session (every transition of `docs/tshttp-runtime/DESIGN.md`), driven by `on_response`/`on_bytes(len, now, sampled)`/`on_sample`/`on_eof`/`on_error`/`reconnect`/`poll_timeout[_with]`, plus `accepts_bytes`, `is_ended`, `is_finite`, `open_count` and `push_event`. `TsHttpClient` is now this core plus a `StreamingTsDemux`; its public API and behaviour are unchanged. Covered by `tests/policy_core.rs`, which drives both over a real fixture and requires identical decisions.
- `tshttp-runtime` 0.1.0: sans-IO TS-over-HTTP pull session client.
  - `TsHttpClient` with `poll`/`on_response`/`feed`/`on_eof`/`on_error`/
    `reconnect`/`poll_timeout`/`next_deadline`/`poll_event`.
  - `Config`, `Action`, `Event`, `StarveKind`.
  - `no_std` + `alloc`; caller-supplied millisecond time.
- `Config::validate()` / `ConfigError`, `Config::clamped()`, `Config::min_reconnect_interval_ms`; `TsHttpClient::new` uses the clamped config.
- Behaviour of the session, all covered by `tests/`:
  - A zero `backoff_ms` entry is rejected by `Config::validate` (`ConfigError::ZeroBackoffEntry`) and raised to 1 ms by `Config::clamped`, so the session can never reconnect in the same instant.
  - `next_deadline` never returns a time in the past after the first stall notice, and bytes arriving after a notice restart the notice schedule; catch-up emits at most one `StallNotice` per `poll_timeout` call, with saturating arithmetic.
  - The start timer runs from `Open` (T20): a server that accepts but never sends headers is aborted and retried after `start_timeout_ms`.
  - A server that sends a sample and closes immediately cannot cause a reconnect spin: `Config::min_reconnect_interval_ms` (default 1000) spaces opens after a short played-then-ended connection; long-lived streams still reconnect with 0 ms.
  - A `206` whose `Content-Range` last byte is not below the total, and any finite response with a `Content-Encoding` other than identity, are treated as live, not finite.
- `Event` implements neither `PartialEq` nor `Eq` (the `Event::Demux` payload has no structural equality) and, with `serde`, serializes only.
