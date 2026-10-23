# Changelog

## [Unreleased]

### Added

- `tshttp-runtime` 0.1.0: sans-IO TS-over-HTTP pull session client.
  - `TsHttpClient` with `poll`/`on_response`/`feed`/`on_eof`/`on_error`/
    `reconnect`/`poll_timeout`/`next_deadline`/`poll_event`.
  - `Config`, `Action`, `Event`, `StarveKind`.
  - `no_std` + `alloc`; caller-supplied millisecond time.
