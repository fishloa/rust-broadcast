# Story 56b — `SiMux` scheduler (dvb-si `ts` feature)

Delegated brief for the SI multiplex scheduler, the second half of GitHub issue
**#56**. **Depends on story 56a** (`SectionPacketizer` in `dvb-si/src/mux.rs`)
being merged to `main` first — build on top of it. Self-contained otherwise.
Touch only the files listed. Do **not** commit.

## Spec sources (read FIRST — ground truth; every default interval MUST cite one)

- **`dvb-si/docs/tr_101_211.md`** → "Repetition rates — satellite and cable"
  (§4.4.1) and "… terrestrial" (§4.4.2). The per-SI-table **maximum intervals**:
  NIT 10 s, BAT 10 s, SDT actual 2 s, SDT other 10 s, EIT p/f actual 2 s,
  EIT p/f other 10 s (sat/cable) / 20 s (terrestrial), EIT sched ≤8 d 10 s,
  EIT sched >8 d 30 s, TDT 30 s, TOT 30 s.
- **`dvb-si/docs/tr_101_290.md`** → "SI repetition-interval limits referenced by
  SI_repetition_error" + Table 5.0a (1.3 `PAT_error`, 1.5 `PMT_error`): PAT/PMT/CAT
  conformance ceiling **0,5 s**.
- **`dvb-si/docs/ts_101_154_av_coding.md`** → "§4.1.7 — Program Specific
  Information (PSI) repetition": PAT/PMT **shall be ≤ 100 ms** (the tightest
  authoritative mandate; this is the PAT/PMT default).
- **`dvb-si/docs/en_300_468.md`** → "§5.1.4 — Repetition rates and random access":
  the **25 ms** minimum inter-section interval floor (≤100 Mbit/s TSs).

> There is **no** PAT/PMT/CAT repetition rate in TR 101 211 (DVB SI only) or in
> ISO/IEC 13818-1 (§2.4.1 is general; only PCR 100 ms / SCR 700 ms exist). Do not
> fabricate one — use the TS 101 154 §4.1.7 mandate (100 ms) as the default and
> cite it. This was verified against the vendored PDFs by the orchestrator.

## What to build

Extend `dvb-si/src/mux.rs` (same module as `SectionPacketizer`). Caller drives
time — **no clock dependency** (mirror the rest of the crate's caller-supplied-time
design). Time is a `core::time::Duration` measured from an arbitrary epoch the
caller picks (monotonic since start).

### 1. Named default-interval constants (each with a one-line cite comment)

```rust
// dvb-si/docs/tr_101_211.md §4.4.1/§4.4.2 (maximum intervals)
pub const NIT_MAX_INTERVAL:        Duration = Duration::from_secs(10);
pub const BAT_MAX_INTERVAL:        Duration = Duration::from_secs(10);
pub const SDT_ACTUAL_MAX_INTERVAL: Duration = Duration::from_secs(2);
pub const SDT_OTHER_MAX_INTERVAL:  Duration = Duration::from_secs(10);
pub const EIT_PF_ACTUAL_MAX_INTERVAL: Duration = Duration::from_secs(2);
pub const TDT_MAX_INTERVAL:        Duration = Duration::from_secs(30);
pub const TOT_MAX_INTERVAL:        Duration = Duration::from_secs(30);
// dvb-si/docs/ts_101_154_av_coding.md §4.1.7 (PAT/PMT shall be ≤ 100 ms)
pub const PAT_MAX_INTERVAL:        Duration = Duration::from_millis(100);
pub const PMT_MAX_INTERVAL:        Duration = Duration::from_millis(100);
// dvb-si/docs/en_300_468.md §5.1.4.1 (minimum inter-section interval floor)
pub const MIN_SECTION_INTERVAL:    Duration = Duration::from_millis(25);
```
(EIT-other / EIT-schedule constants optional; add them if cheap, each cited.)
Pick the const set that covers the tables the scheduler actually defaults; the
**defaults test** below pins each to its transcription.

### 2. `SiMux`

Owns its section bytes (replaceable — SI changes over time), one entry per
"stream" the caller wants emitted on a PID at an interval:

```rust
pub struct SiMux { /* entries: pid, owned section bytes Vec<u8>, interval, last_emit Option<Duration>, packetizer */ }

impl SiMux {
    pub fn new() -> Self;
    /// Register/replace the sections emitted on `pid` at `interval`. `sections`
    /// is the concatenated complete-section bytes for one emission cycle; stored
    /// owned. Re-calling for the same `pid` updates bytes and/or interval.
    pub fn upsert(&mut self, pid: u16, sections: Vec<u8>, interval: Duration);
    /// Convenience constructors with the cited defaults (document the cite):
    pub fn upsert_pat(&mut self, sections: Vec<u8>);  // PAT_MAX_INTERVAL, PID 0x0000
    pub fn upsert_pmt(&mut self, pid: u16, sections: Vec<u8>);  // PMT_MAX_INTERVAL
    pub fn upsert_sdt_actual(&mut self, sections: Vec<u8>);     // SDT_ACTUAL_MAX_INTERVAL, PID 0x0011
    // …nit/tdt/tot similarly, each citing tr_101_211/tr_101_290/ts_101_154.

    /// Emit every entry due at `now` (i.e. `now - last_emit >= interval`, and
    /// first call always due), packetizing via SectionPacketizer, appended to
    /// `out` (cleared first). Updates each emitted entry's `last_emit = now`.
    /// Deterministic given the fed `now` sequence. Returns packet count.
    pub fn poll_into(&mut self, now: Duration, out: &mut Vec<[u8; TS_PACKET_SIZE]>) -> usize;
    pub fn poll(&mut self, now: Duration) -> Vec<[u8; TS_PACKET_SIZE]>;
}
```

- The 25 ms floor: expose `MIN_SECTION_INTERVAL` and **reject** (or document +
  clamp — pick reject via a debug_assert/doc, not silent) an `upsert` interval
  below it. Decide and document; do not silently accept sub-25 ms.
- Each entry keeps its own `SectionPacketizer` so continuity counters are correct
  and continuous per PID across poll cycles.

### Tests (in-module)

1. **Defaults pinned to spec:** assert each `*_MAX_INTERVAL` const equals the
   value in its cited transcription (e.g. `PAT_MAX_INTERVAL == Duration::from_millis(100)`,
   `NIT_MAX_INTERVAL == Duration::from_secs(10)`). This is the drift guard.
2. **Deterministic schedule:** feed a fixed `now` sequence; assert an entry emits
   exactly when due and not before; two entries with different intervals emit on
   their own cadence; `last_emit` advances correctly.
3. **First poll always emits** every registered entry.
4. **Round-trip:** the packets `poll` produces feed back through
   `SectionReassembler`/`SiDemux` to the original sections (reuses 56a's oracle).
5. **CC continuity across polls:** the per-PID continuity_counter is continuous
   across successive `poll` cycles (no reset).

## Constraints

- **No magic numbers** outside `#[cfg(test)]`; every interval is one of the named
  cited consts. **Every default cites its `docs/` transcription in a comment** —
  this is the whole point of this story; a fabricated/uncited interval fails audit.
- Caller-supplied `Duration` only — no `std::time::Instant::now()`, no clock.
- `#[cfg_attr(feature = "serde", …)]` not required (no borrowed public structs).
- Builds `--no-default-features` (ts-gated → compiles out) and MSRV 1.75.

## Gate commands (run ALL; fix until every one green before finishing)

```
cargo build  --workspace --all-features --locked
cargo test   -p dvb-si --all-features --locked
cargo build  --workspace --no-default-features --locked
cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc -p dvb-si --all-features --no-deps --locked
```

## Boundaries

- Touch only `dvb-si/src/mux.rs` (extend). No changes to `ts.rs`/`demux.rs`/tables.
- Do **not** redefine the packetizer — build on story 56a's `SectionPacketizer`.
- Do **not** commit.
