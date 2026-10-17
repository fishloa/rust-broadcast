# transmux 0.27.4

_Released 2026-10-18._

**Patch release.** On a multi-program transport stream, `StreamingTsDemux` emitted a `DemuxEvent::ClockReference` for the PCR on *any* PID and attached no program, so a consumer saw stray PCR PIDs and could not tell which program a clock belonged to. 0.27.4 emits a clock reference only from a PID that a PMT names as its `PCR_PID`, and says which program it belongs to. It also adds an opt-in cap on queued `InputDegraded` events and documents the draining requirement. Reported by zenith (issues #1354 and #1352).

## Why this version

- **Proposed version:** 0.27.3 -> **0.27.4** (patch). The only new public items are `EventProvenance::program_number` (a field on a `#[non_exhaustive]` struct, so downstream code can neither construct it with a literal nor match it exhaustively), `StreamingTsDemux::with_max_degraded_events` and `StreamingTsDemux::degraded_events_dropped`. The default of every existing call is unchanged except the `ClockReference` filtering below, which is a correctness fix.
- **Caret epoch:** unchanged (0.27); dependents on `transmux = "0.27"` need no edits. Every workspace dependent (`multimux`, `hls-runtime`, `media-plane`, `compliance-probe`, `media-doctor`, `differential-sweep`, ...) builds unchanged; the two that name `EventProvenance` use `EventProvenance::default()`, and the ones that match `ClockReference` use `{ .. }`.
- **No dependency changes.** The `mpeg-ts` 0.6.2 release that ships with it is documentation only; this crate does not need it.

## Migration

- **`ClockReference` consumers now get `provenance.program_number` and fewer stray PCR PIDs.** Measured on the real captures before the fix: France TNT UHF32 produced 5 PCR PIDs (all genuine PCR_PIDs) and Hot Bird MHP produced 20 PCR PIDs for 19 PMT-named PCR_PIDs, the extra one (PID 985, a single PCR at packet 68185) being named by no PMT. After the fix a PCR on a PID no program names is dropped. If several programs share a PCR_PID you get one event per program; dedupe on `provenance.pid` for a PID-level view. Anything that relied on seeing a PCR from an unannounced PID will stop seeing it.
- **`program_number: None` means "not yet attributable".** Until every PMT the PAT announces has been applied, a PCR on a not-yet-claimed PID is still emitted, with `program_number: None`, rather than held (holding would lose the first PMT interval of clock samples). It never means "no program".
- **`Media::pcr`** (batch `TsDemux`) is filtered the same way.
- **Long-running consumers must drain `poll_event`.** The event queue only empties through `poll_event` (about 2 `InputDegraded` per loss on lossy input), so an undrained caller grows memory linearly. If you cannot drain promptly, opt into the new cap:

```rust
let mut demux = StreamingTsDemux::new().with_max_degraded_events(1024);
// ... feed / poll_event as before ...
let lost = demux.degraded_events_dropped(); // counted overflow, never silent
```

The default is unchanged (unbounded), and only `InputDegraded` events are ever dropped by the cap.

## What changed

### Fixed
- **`DemuxEvent::ClockReference` on a multi-program TS (#1354).** `StreamingTsDemux` now emits only from a PID some program's PMT names as `PCR_PID` (ISO/IEC 13818-1 §2.4.3.5, §2.4.4.8-9), one event per program naming it, with the new `EventProvenance::program_number`. Before every announced PMT is known a PCR on an as-yet-unclaimed PID is emitted with `program_number: None` instead of held. `Media::pcr` (batch `TsDemux`) is filtered the same way. A byte offset is not added: `packet_index * 188` is the offset only when no resync loss preceded it, and `InputDegraded::SyncLoss` already reports true stream offsets.

### Added
- `EventProvenance::program_number: Option<u16>` (additive on a `#[non_exhaustive]` type; today set only by `ClockReference`).
- `StreamingTsDemux::with_max_degraded_events(cap)` + `degraded_events_dropped()` (#1352): opt-in cap on queued `InputDegraded` events with a counted overflow. Default unchanged (unbounded); other event kinds are never dropped.

### Documentation
- `StreamingTsDemux::feed` / `poll_event` / `finish` state the draining requirement (#1352). README and crate docs updated.

---

Published from tag `transmux-v0.27.4`.
