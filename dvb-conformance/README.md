# dvb-conformance

[![crates.io](https://img.shields.io/crates/v/dvb-conformance.svg)](https://crates.io/crates/dvb-conformance)
[![docs.rs](https://img.shields.io/docsrs/dvb-conformance)](https://docs.rs/dvb-conformance)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#related-crates-and-license)

**ETSI TR 101 290 transport-stream conformance monitor: feed TS packets with a caller-supplied clock, get structured per-indicator events for the first, second and third measurement priorities.**

## What it does / what it does not

`ConformanceMonitor` is a sans-IO state machine that checks a transport stream
against the measurement guidelines of TR 101 290: sync and continuity
(priority 1), transport/CRC/PCR/PTS/CAT errors (priority 2), and SI
repetition, table-id and unreferenced-PID checks plus a partial T-STD buffer
model (priority 3). It follows the PAT to discover PMTs and elementary-stream
PIDs by itself. It is for monitoring probes, QC tools and test harnesses
(`compliance-probe` drives it over a live trunk; `media-doctor`, `ts-fix`,
`multimux` and `transmux` also depend on it).

It does not own a clock: you pass a monotonic `Duration` with every packet, so
anything that needs arrival-time resolution beyond that is not computable
(notably indicator 2.4, `PCR_accuracy_error`, which needs +/-500 ns hardware
timestamps; the `PcrAccuracyError` variant exists for completeness and is never
emitted). It does not do I/O, does not repair streams (see `ts-fix`), and
models only part of the T-STD (see below).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI TR 101 290, *Measurement guidelines for DVB systems* | V1.4.1 (2020-06) (vendored; verified) | §5.2.1 Table 5.0a (1.1 to 1.6), §5.2.2 Table 5.0b (2.1 to 2.3b, 2.5, 2.6), §5.2.3 Table 5.0c (3.1 to 3.10) | Partial: 27 of 28 indicator variants are emitted | 2.4 `PCR_accuracy_error` is not computable; 3.3/3.9/3.10 are partial (see "T-STD buffer model"). Transcription: `docs/tr_101_290.md`. |
| ITU-T H.222.0 / ISO/IEC 13818-1 | H.222.0 v9 (08/2023) (vendored; verified) | §2.4.3.3 continuity_counter and legal duplicate packets (shared with `ts-fix` and `media-doctor` via `broadcast_common::ts_dup`), §2.4.2.2 clocks, §2.4.2.4 T-STD transport buffer TBn/TBsys | Partial | Transcription of the buffer clauses: `docs/h222_0-tstd-buffer-model.md`. |
| ETSI TR 101 211 | V1.9.1, vendored (`etsi_tr_101_211_v01.09.01_dvb_si_guidelines`; §4.4 "Minimum repetition rates") | §4.4 section interval per sub-table, the clause Table 5.0c cites for the 25 ms minimum gap | Used for `SiMinGapError` keying | |
| ETSI EN 300 468 | via `dvb-si` (V1.19.1 / V1.20.1 vendored) | Table-id sets on the well-known SI PIDs; PAT/PMT/EIT section parsing | Used, not owned | Section parsing is delegated to `dvb-si`. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Forwards `std` to `broadcast-common`, `dvb-si`, `mpeg-ts`; off gives `no_std` + `alloc` |
| `serde` | yes | `serde::Serialize` on `ConformanceEvent`, `Indicator`, `Priority`, `Stats` |

```toml
[dependencies]
dvb-conformance = "11"
```

## Quick start

```rust
use core::time::Duration;
use dvb_conformance::ConformanceMonitor;

// A committed capture; the path is relative to the crate root.
let data = std::fs::read("../fixtures/ts/m6-single.ts").unwrap();
let mut monitor = ConformanceMonitor::new(); // default Config
for (i, pkt) in data.chunks_exact(188).enumerate() {
    let t = Duration::from_micros(i as u64 * 40); // caller-supplied monotonic clock
    for event in monitor.feed(pkt, t) {
        eprintln!("[{}] {} (clause {}): {}", event.priority.name(),
                  event.indicator.name(), event.indicator.clause(), event.detail);
    }
}
let stats = monitor.stats();
assert!(stats.packets > 0 && stats.in_sync);
```

`ConformanceMonitor::with_config(Config { .. })` tunes every threshold: PAT/PMT
intervals, `pid_error_period`, sync acquire/loss counts, PCR repetition and
discontinuity limits, PTS repetition, SI repetition intervals, the 3.4
unreferenced-PID period, the 25 ms minimum gap and the `_other` intervals.
Examples: `cargo run -p dvb-conformance --example monitor_stream` and
`--example priority_breakdown`.

## What is implemented

| Item | Purpose |
|---|---|
| `ConformanceMonitor` (`new`, `with_config`, `feed(packet, t) -> &[ConformanceEvent]`, `stats`) | The monitor |
| `Config` | Hysteresis/timeout parameters (defaults are the TR 101 290 values) |
| `ConformanceEvent { indicator, priority, pid: Option<Pid>, at, detail }` | One raised error |
| `Indicator` (28 variants; `priority()`, `name()`, `clause()`), `Priority` (`First`, `Second`, `Third`) | What was raised |
| `Stats { packets, events, in_sync }` | Diagnostic counters |

### Priority 1 (Table 5.0a)

| Clause | Indicator | Notes |
|---|---|---|
| 1.1 | `TsSyncLoss` | Hysteresis: 5 consecutive good sync bytes to acquire, 2 bad to lose |
| 1.2 | `SyncByteError` | `sync_byte != 0x47` |
| 1.3.a | `PatError2` | PAT absent > 500 ms; wrong table_id or scrambled on PID 0 |
| 1.4 | `ContinuityCountError` | CC wrap, with the H.222.0 one-legal-duplicate allowance |
| 1.5.a | `PmtError2` | PMT absent > 500 ms per `program_map_PID`; scrambled PMT PID |
| 1.6 | `PidError` | Referenced ES PID absent > 5 s |

### Priority 2 (Table 5.0b)

| Clause | Indicator | Notes |
|---|---|---|
| 2.1 | `TransportError` | `transport_error_indicator` set |
| 2.2 | `CrcError` | CRC-32 mismatch on a completed long-form section |
| 2.3a / 2.3b | `PcrRepetitionError`, `PcrDiscontinuityError` | PCR interval > 100 ms; PCR delta > 100 ms without `discontinuity_indicator` (modular PCR arithmetic from `broadcast_common::clock33`) |
| 2.4 | `PcrAccuracyError` | **Never emitted** (needs +/-500 ns hardware timing) |
| 2.5 | `PtsError` | PTS interval > 700 ms |
| 2.6 | `CatError` | Wrong table_id on PID 1; scrambled packets with no CAT seen |

### Priority 3 (Table 5.0c)

| Clause | Indicator | Notes |
|---|---|---|
| 3.1 / 3.1.b | `NitError`, `NitOtherError` | Disallowed table_id on PID 0x10; NIT_actual absent > 10 s; NIT_other sections > 10 s apart |
| 3.2 | `SiRepetitionError` | Max interval: NIT_actual 10 s, SDT_actual 2 s, EIT P/F actual 2 s, TDT 30 s; timers lazily armed |
| 3.3 | `BufferError` | TBsys overflow only (512 B, 1 Mbit/s drain, fed per TS packet) |
| 3.4 | `UnreferencedPid` | A PID not referenced by PAT/CAT/PMT/well-known SI persists > 500 ms |
| 3.5 / 3.5.b | `SdtError`, `SdtOtherError` | PID 0x11 table_id set; SDT_actual absent > 2 s; SDT_other spacing |
| 3.6 / 3.6.b / 3.6.c | `EitError`, `EitOtherError`, `EitPfError` | PID 0x12 table_id set; EIT P/F actual absent > 2 s; other spacing; missing P/F section |
| 3.7 | `RstError` | Disallowed table_id on PID 0x13 (no absence threshold in the spec) |
| 3.8 | `TdtError` | Disallowed table_id on PID 0x14; TDT absent > 30 s |
| 3.9 | `EmptyBufferError` | TBn/TBsys empty at least once per second |
| 3.10 | `DataDelayError` | Data delay through TBn/TBsys > 1 s |
| shared | `SiMinGapError` | Two sections of the same sub-table `(pid, table_id, table_id_extension)` < 25 ms apart (interval per sub-table, TR 101 211 §4.4) |

### T-STD buffer model (3.3, 3.9, 3.10)

`src/tstd.rs` models TBn (per PID, 512 B) and TBsys (512 B, 1 Mbit/s drain,
fed per packet by PSI only). TBn overflow, MBn/EBn/Bn/Bsys and the 60 s
still-picture data-delay threshold are **deferred**: they need the coded
bitrate and buffer sizes from `multiplex_buffer_descriptor` /
`smoothing_buffer_descriptor`, which the monitor does not parse. Constants
cite H.222.0 §2.4.2.4 (TB size 512 bytes, TBsys `Rxn` 1x10^6 bit/s, Bsys 1536
bytes, deferred).

### Known limitations

- 3.4 does not exempt CAT-referenced EMM PIDs (CA descriptors are not decoded)
  or PIDs the operator defines as private data, so those streams can give a
  false positive.
- Clock monotonicity is the caller's responsibility; non-monotonic timestamps
  produce spurious events.
- EIT P/F is tracked at table level (any table_id `0x4E` section resets the
  timer) to avoid false positives on dense schedules.

## Design notes

- **Sans-IO, caller clock.** All presence/absence checks run on the
  `Duration` you pass; no `Instant`, no threads.
- **Single reading of duplicates.** The "two, and only two" legal-duplicate
  rule is `broadcast_common::ts_dup`, shared with `ts-fix` and `media-doctor`
  so the three tools cannot disagree (issue #956).
- **No 33-bit arithmetic of its own.** `tests/clock33_guard.rs` bans a local
  2^33 / `abs_diff`; PCR/PTS wrap handling is `broadcast_common::clock33`.
- **Honest gaps over false positives.** An indicator that cannot be computed
  honestly is documented and not emitted rather than approximated.
- Public enums carry `name()` and `Display`; `no_std` + `alloc`.

## Related crates and license

Depends on `broadcast-common`, `mpeg-ts` and `dvb-si`. Used by
`compliance-probe`, `media-doctor`, `ts-fix`, `multimux` and `transmux`. Lockstep
version with `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-tools`.

MSRV 1.95.0. Licensed under [MIT](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-MIT) or
[Apache-2.0](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-APACHE), at your option.
