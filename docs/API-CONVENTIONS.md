# API conventions

Binding for every public API in the workspace. New code follows them; a lane that settles a new convention adds it here in the same PR.

## Time

- Wall-clock instant: `jiff::Timestamp`. No chrono.
- Monotonic "now" in sans-IO engines: `broadcast_common::stage::Timestamp`, passed by the caller; engines never read a clock.
- Spans and timeouts: `std::time::Duration`; all timeouts through `broadcast_common::timeouts::Timeouts`. No `*_ms`/`*_us` integers in public signatures or config.
- Media time: a timestamp travels with its timescale; zero timescale is an error. 33-bit PTS/DTS and 27 MHz PCR arithmetic only via `broadcast_common::clock33`.

## Types

- No raw bytes or bare integers for structured data: `Pid`, track/stream ids, sequence numbers use their types; constructors check ranges and never silently mask.
- Byte ranges: one `ByteSpan`, never `(u64, u64)`.
- URLs as `url::Url`; HTTP method/headers as `http` types.
- Units of one quantity are the same across crates.

## Construction

- Config: `Config::default()` + `with_*` setters (or `new(required..)` when fields are mandatory); struct `#[non_exhaustive]`.
- One constructor per role (client/server/listener) taking that config.
- One shared type per concept across crates (timeouts, backoff, UDP options).

## Errors

- Typed `thiserror` enums with context fields. No `String` errors, no `Box<dyn Error>` in public APIs.
- Callers decide on variants, never on message text.
- No panics reachable from input; no panicking twin beside a fallible function; constructors validate and return `Result`.

## Surface

- Public enums/structs that can grow are `#[non_exhaustive]`; spec enums carry `name()` + `impl_spec_display!`.
- `pub` only what consumers need; no re-export aliases for removed names (delete, don't deprecate).
- One implementation per job; no lint suppressions (`unsafe_code` excepted, pinned with `SAFETY`).

## Enforcement

Mechanical rules are `workspace-guards` families; each convention's guard lands with the lane that finishes its migration. The rest is checked in review ([CRATE-ACCEPTANCE.md](CRATE-ACCEPTANCE.md), [RELEASE-AUDIT.md](RELEASE-AUDIT.md) §3).
