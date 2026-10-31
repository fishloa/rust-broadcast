# compliance-probe

[![Crates.io](https://img.shields.io/crates/v/compliance-probe.svg)](https://crates.io/crates/compliance-probe)
[![docs.rs](https://img.shields.io/docsrs/compliance-probe)](https://docs.rs/compliance-probe)

**A live TR 101 290 + PCR drift/jitter + SCTE-35 cue-sanity probe that records what it finds through the `metrics` facade, for a host process to render as Prometheus.**

## What it does / what it does not

Point it at a continuous feed and it checks, packet by packet, what an
operator's dashboard needs: ETSI TR 101 290 priority 1/2/3 indicators (through
`dvb-conformance`), a per-PID PCR interval-error ("drift") and jitter estimate,
SCTE-35 `splice_insert` cue sanity (future vs past `pts_time`), and the
structural checks of `media-doctor`. Results go out as counters and gauges via
the [`metrics`](https://docs.rs/metrics) facade; a host that already installs a
recorder (for example `multimux`, which installs a process-wide
`metrics_exporter_prometheus` recorder) exposes them with no further work. A
ready-made Grafana dashboard ships in `dashboards/compliance-probe.json`
(a panel per metric, a text panel listing what is not monitored, and a
priority-1 alert rule).

**This crate does not open a socket, run a thread or serve `/metrics`.** It is a
library: you feed it TS packets, SCTE-35 sections, or `media_plane::ByteTap` /
`EventCursor` items, and it records through whichever recorder is installed.

### What is deliberately NOT measured

A green dashboard must mean "checked and passing", never "not checked":

- **TR 101 290 indicator 2.4 `PCR_accuracy_error`**: `dvb-conformance` never
  emits it (it needs +/-500 ns hardware arrival timing). The PCR drift/jitter
  gauges here are a different, coarser software-clock signal, not 2.4 renamed.
- **T-STD buffer-model sub-checks** that `dvb-conformance` documents as partial
  (`Buffer_error` TBn overflow, `Empty_buffer_error` MBn, `Data_delay_error`'s
  60 s still-picture threshold): this crate adds no buffer-model logic.
- **SCTE-35 malformed-section detection on the Trunk-cursor path**: an event in
  a `Trunk`'s log was already parsed successfully upstream; only the wire path
  (`Probe::feed_scte35_section`) can observe malformedness.
- **A fabricated "now" for an unresolved SCTE-35 anchor**: a
  `media_plane::EventAnchor::Segment`/`Utc` entry stays unresolved, never
  guessed into a media time.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI TR 101 290, *Measurement guidelines for DVB systems* | V1.4.1 (2020-06) (vendored; verified) | Priority 1/2/3 indicators as implemented by `dvb-conformance` | Partial (inherits `dvb-conformance`: 27 of 28 indicators emitted) | Continuity-count errors are one indicator (`indicator="Continuity_count_error"`), not a separate metric. |
| ANSI/SCTE 35 | 2023r1 (vendored; verified) | §9.7.3.1 `splice_insert` `time_specified_flag` / `pts_time`: future-vs-past judgement, immediate splices, malformed section (wire path) | Partial: `splice_insert` only | Other command types (`splice_null`, `time_signal`, `splice_schedule`) report `NotSpliceInsert` and are not judged. |
| ITU-T H.222.0 / ISO/IEC 13818-1 | H.222.0 v9 (08/2023) | §2.4.2.2 PCR clock, §2.4.3.4/§2.4.3.5 `discontinuity_indicator` resets the drift baseline | Used | Modular 27 MHz arithmetic from `broadcast_common::clock33`. |
| Container/stream structural checks | via `media-doctor` | `SyncByteCheck`, `PatPmtVersionCheck`, `CcAnomalyCheck`, `PcrCheck`, `PtsCheck`, `CodecSignallingCheck`, `Scte35Check` | Delegated | `run_structural_checks` runs them over a whole buffer. |

## The clock you feed is part of the measurement

`Probe::feed_ts_packet` takes an arrival timestamp, and most TR 101 290
indicators are **timeout-based** (PAT > 500 ms, PID > 5 s, SI 2/10/30 s, and
the T-STD TBsys 1 Mbit/s drain). The clock is an *input to the result*.
`tests/wasm_analyzer_equivalence.rs` pins a measured instance: the `demo/` WASM
analyzer and this crate use the identical default `dvb_conformance::Config`,
yet report 877 vs 876 events on `fixtures/ts/m6-single.ts`, because that
analyzer anchors its clock on PCR with a `+1 ns` per-packet fallback and the
fixture has no PCR: it models 1264 packets as 1.264 us (~1.5 Tbit/s), so TBsys
cannot drain and 3.3 `Buffer_error` fires once (it fires zero times at a
realistic bitrate).

| Indicator | Degenerate clock (analyzer) | Realistic clock (100 us/pkt) |
|---|---|---|
| `Continuity_count_error` | 876 | 876 |
| `Buffer_error` (T-STD 3.3) | 1 | 0 |
| **Total** | **877** | **876** |

So feed real arrival time (`Probe::drain_byte_tap` does, using the `ByteTap`'s
recorded `Timestamp`), and treat `Buffer_error` / `Empty_buffer_error` /
`Data_delay_error` as only as trustworthy as that clock. `Continuity_count_error`
is clock-independent: the test asserts 876 at every rate from frozen to
2 ms/packet.

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std`; enables metric recording through the `metrics` facade and the `media-plane` bridge (`trunk_bridge`: `drain_byte_tap`, `drain_event_cursor`, `note_media_time`). Without it the crate is `no_std` + `alloc`: every check still runs, it is simply not reported. |

```toml
[dependencies]
compliance-probe = "0.5"
```

## Quick start

```rust
use std::time::Duration;
use compliance_probe::Probe;

let data = std::fs::read("../fixtures/ts/m6-single.ts").unwrap(); // committed capture
let mut probe = Probe::new();
for (i, chunk) in data.chunks_exact(188).enumerate() {
    let packet: &[u8; 188] = chunk.try_into().unwrap();
    // A file has no arrival time: assume 40 us/packet (~37.6 Mbit/s). See above.
    probe.feed_ts_packet(packet, Duration::from_micros(i as u64 * 40));
}
let stats = probe.conformance_stats();
assert_eq!(stats.packets, 1264);
assert!(stats.in_sync);
```

Runnable examples: `cargo run -p compliance-probe --example fixture_report`
(plain byte-buffer feed, cross-checked against `dvb-conformance`) and
`--example byte_tap_live` (the `media_plane::ByteTap` attachment point).

## What is implemented

| Item | Purpose |
|---|---|
| `Probe` (`new`, `with_config(dvb_conformance::Config)`, `feed_ts_packet(&[u8;188], t)`, `feed_scte35_section(section, now_pts) -> Scte35Sanity`, `conformance_stats`) | Per-packet and per-section checks |
| `Probe::drain_byte_tap`, `drain_event_cursor`, `note_media_time`; `trunk_bridge::TrunkBridge` (`std`) | One bounded, non-blocking cursor on a `media_plane::ByteTap` (TR 101 290 needs raw wire bytes: a demuxed sample has already dropped the continuity counter) and on a `Trunk`'s `EventCursor` (SCTE-35), never a second copy of the stream |
| `conformance::PcrSample { pid, drift_ppm, jitter_ppm }` | Per-PID PCR estimate; baseline resets on `discontinuity_indicator` |
| `scte35::{Scte35Sanity, check_section}` | `Malformed`, `NotSpliceInsert`, `Immediate`, `InFuture`, `InPast` |
| `structural::run_structural_checks(ts) -> media_doctor::Report` | `media-doctor`'s diagnostics, findings counted |
| `metric_names::*` | `compliance_probe_ts_packets_total`, `_tr101290_events_total{indicator,priority}`, `_tr101290_in_sync`, `_pcr_drift_ppm{pid}`, `_pcr_jitter_ppm{pid}`, `_tap_lagged_total`, `_event_cursor_lagged_total`, `_scte35_cues_total`, `_scte35_malformed_total`, `_scte35_immediate_total`, `_scte35_pts_in_past_total`, `_scte35_unresolved_anchor_total`, `_scte35_no_reference_total`, `_structural_findings_total` |

## Design notes

- **Library, not a service.** No threads, sockets or exporters; the `metrics`
  facade is the only output, and its macros are `std`-only (compiled out in a
  `no_std` build).
- **No second implementation.** TR 101 290 is `dvb-conformance`, structural
  checks are `media-doctor`, SCTE-35 parsing is `scte35-splice` /
  `timed-metadata`, PCR wrap arithmetic is `clock33` (`tests/clock33_guard.rs`).
- **Honest gaps.** Every limitation above is stated rather than left as an
  always-zero metric.
- Public enums carry `name()` and `Display` (`tests/label_coverage.rs`).

## Related crates and license

Depends on `broadcast-common`, `dvb-conformance`, `mpeg-ts`, `scte35-splice`,
`timed-metadata`, `media-doctor`, `media-plane`. Designed to sit beside
`multimux`. Independently versioned (`0.x`). MSRV 1.95.0. Licensed under
MIT OR Apache-2.0.
