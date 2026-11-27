# API conventions

Binding for every public API in the workspace. New code follows them; a lane that settles a new convention adds it here in the same PR.

## Time

- Wall-clock instant: `jiff::Timestamp`. No chrono.
- Monotonic "now" in sans-IO engines: `broadcast_common::stage::Timestamp`, passed by the caller; engines never read a clock.
- Spans and timeouts: `std::time::Duration`; all timeouts through `broadcast_common::timeouts::Timeouts`. No `*_ms`/`*_us` integers in public signatures or config.
- Deadlines are `Timestamp::checked_add(Duration)`; `None` means the timer never fires; never `saturating_add` a deadline (`saturating_*` only clamps a "now" reading).
- Media time: a timestamp travels with its timescale; zero timescale is an error. 33-bit PTS/DTS and 27 MHz PCR arithmetic only via `broadcast_common::clock33`.

## Types

- No raw bytes or bare integers for structured data: `Pid`, track/stream ids, sequence numbers use their types; constructors check ranges and never silently mask.
- Byte ranges: one `ByteSpan`, never `(u64, u64)`.
- URLs as `url::Url`; HTTP method/headers as `http` types.
- Units of one quantity are the same across crates.

## Wire formats

- The symmetric `Parse<'a>` / `Serialize` contract, the shared error kinds, the checked field-width helpers, the bit I/O, the media `timescale`, the typed reserved-bit fields and the CENC scheme identity live in `broadcast_core` (the frozen floor, versioned apart from the DVB lockstep). A wire crate implements them from there; it depends on `broadcast-common` only for the stage/clock/codec/XML pieces, which sit above the contract.

## Construction

- Config: `Config::default()` + `with_*` setters (or `new(required..)` when fields are mandatory); struct `#[non_exhaustive]`.
- One constructor per role (client/server/listener) taking that config.
- One shared type per concept across crates (timeouts, backoff, UDP options).
- A caller-owned stage inserted into an existing assembly is a **per-call parameter**, not a builder field (`Repackage::run_transformed(fmp4, transform)`, `cli::run_bytes_with(input, opts, transform)`), so the configured driver keeps `Clone` and one instance serves several inputs. The bound on what the stage may hold is the caller's (`*_limit`, unlimited by default), never a library-chosen number.

## Best-effort reading

- A reader parses as well as it can and continues. It skips broken metadata and broken frames, and
  refuses only when what is missing makes playback impossible. **A silent skip is a defect**: whenever
  the reader leaves content out, alters it, resynchronises past it, truncates, repairs a timeline or
  drops a track, it publishes a `broadcast_core::damage::Damage`.
- The report is plain data: `DamageKind` (what happened to the content, not which box), `Severity`
  (`Informational`/`Degraded`; a fatal failure is an error, never a report), `DamageLocation` (the input
  byte range, the track, the media time range in a stated timescale and the unit ordinal — each only when
  the reader observed it), `what` (the structure in the format's own terms, plus a four-character code for
  box formats), `cause` (a shared error kind from `broadcast_core::error` where one applies) and
  `recoverable_bytes` (whether the skipped bytes are still available verbatim).
- The channel is the reader's existing output enum, which gains one variant carrying the `Damage`
  (`DemuxEvent::Damage`, `SessionEvent::Damage`, `Event::Damage`). No callback is stored in a reader.
- The caller decides what a report means: `DamagePolicy` maps each `Severity` (and optionally each
  `DamageKind`) to `Disposition::Report` (the default for everything) or `Disposition::Fail`. A reader
  calls `DamagePolicy::admit(damage)`: `Ok(damage)` publishes it and continues, `Err(DamageError)` stops
  with the same report inside the error. There is no silent setting.
- A crate that keeps its own report enum must convert it into `broadcast_core::damage::Damage` (an
  `impl From` into `Damage` or `DamageKind`); the `damage_report` workspace guard fails otherwise.

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
