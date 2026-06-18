# dvb-conformance

[![crates.io](https://img.shields.io/crates/v/dvb-conformance.svg)](https://crates.io/crates/dvb-conformance)
[![docs.rs](https://img.shields.io/docsrs/dvb-conformance)](https://docs.rs/dvb-conformance)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

ETSI TR 101 290 v1.4.1 transport-stream conformance monitor (DVB measurement
guidelines). Feed TS packets with a caller-supplied monotonic clock, drain
structured conformance events per indicator.

## Quick start

```rust
use dvb_conformance::{ConformanceMonitor, ConformanceEvent, Indicator};
use core::time::Duration;

let mut monitor = ConformanceMonitor::new();  // default config

let mut t = Duration::ZERO;
for packet in ts_packets {
    t += Duration::from_micros(188 * 8 * 1_000_000 / 6_000_000); // ≈188 bytes @ 6 Mbit/s
    for event in monitor.feed(packet, t) {
        eprintln!("[{:?}] {}: {}", event.priority, event.indicator.name(), event.detail);
    }
}
```

Use `ConformanceMonitor::with_config(Config { .. })` to tune thresholds (PAT/PMT
max intervals, PCR repetition and discontinuity limits, SI repetition intervals, etc.).

## What's implemented

### Indicators — 13 implemented, per the `Indicator` enum in `src/lib.rs`

| Priority | Clause | Indicator | Notes |
|----------|--------|-----------|-------|
| 1 | 1.1 | `TsSyncLoss` | Hysteresis: 5 consecutive good → in-sync; 2 consecutive bad → sync lost |
| 1 | 1.2 | `SyncByteError` | `sync_byte != 0x47` on any packet |
| 1 | 1.3.a | `PatError2` | PAT absent > 500 ms (default); wrong table_id or scrambled on PID 0x0000 |
| 1 | 1.4 | `ContinuityCountError` | CC wrap + duplicate-packet allowance (one dup per sequence) |
| 1 | 1.5.a | `PmtError2` | PMT absent > 500 ms per `program_map_PID`; scrambled on PMT PID |
| 1 | 1.6 | `PidError` | Referenced ES PID absent > 5 s (default) |
| 2 | 2.1 | `TransportError` | `transport_error_indicator` set |
| 2 | 2.2 | `CrcError` | CRC-32 mismatch on completed long-form SI/PSI section |
| 2 | 2.3a | `PcrRepetitionError` | PCR interval > 100 ms (default) on any PCR-carrying PID |
| 2 | 2.3b | `PcrDiscontinuityError` | PCR delta > 100 ms (default) without `discontinuity_indicator` |
| 2 | 2.5 | `PtsError` | PTS interval > 700 ms (default) on ES PIDs |
| 2 | 2.6 | `CatError` | Wrong table_id on PID 0x0001; scrambled packet with no CAT seen |
| 3 | 3.2 | `SiRepetitionError` | Max-interval dimension: NIT_actual (10 s), SDT_actual (2 s), EIT P/F actual (2 s), TDT (30 s) |

Intentionally excluded:

| Clause | Indicator | Reason |
|--------|-----------|--------|
| 2.4 | `PCR_accuracy_error` | Requires hardware arrival timestamps — not computable under the caller-supplied-time model |
| 3.2 | SI_repetition_error (25 ms min gap) | Deferred — needs per-`(table_id, section_number)` tracking to avoid false positives on dense multi-section tables |
| 3.1 / 3.3–3.10 | — | Out of scope |

### Caller-supplied time model

`ConformanceMonitor::feed(packet, t)` takes a `core::time::Duration` alongside
each TS packet. The monitor uses this clock for all presence/absence timeout
checks (1.3.a, 1.5.a, 1.6, 2.3a, 2.3b, 2.5, 3.2). The caller must supply
monotonic non-decreasing timestamps; the monitor does not enforce this but
non-monotonic timestamps will produce spurious events. Because there is no
independent hardware clock, PCR accuracy (2.4) and buffer-model indicators are
not computable.

### SI_repetition_error (3.2) — implementation notes

- Maximum-interval checks are implemented for NIT_actual (10 s), SDT_actual
  (2 s), EIT P/F actual (2 s), and TDT (30 s). Each table's timer is lazily
  armed — checking starts only after the first section of that table is seen;
  an entirely absent table is not flagged by this indicator (that is the role
  of the out-of-scope per-table presence indicators).
- EIT P/F is tracked at the table level (any section with table_id `0x4E`
  resets the timer), not per section_number, to avoid false positives on dense
  EIT schedules.

### PAT-following PMT discovery

The monitor parses each completed PAT section and automatically starts tracking
the `program_map_PID` entries it finds, enabling indicator 1.5.a (PMT absence)
and ES PID extraction (indicator 1.6).

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | **on** | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | **on** | Serialize-only (`serde::Serialize`) on `ConformanceEvent`, `Indicator`, `Priority`, `Stats`. |

## MSRV

Rust **1.81**.

## References

- ETSI TR 101 290 v1.4.1 (2023-05) — DVB Measurement Guidelines (§5.2.1 Table 5.0a, §5.2.2 Table 5.0b, §5.2.3 Table 5.0c)
- ISO/IEC 13818-1 — MPEG-2 Systems

## Examples

Run with `cargo run -p dvb-conformance --example <name>`:

- **`monitor_stream`** — run the TR 101 290 monitor over a capture and print the headline stats.
- **`priority_breakdown`** — tally findings by measurement priority (1/2/3) and indicator.

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
