# compliance-probe

[![Crates.io](https://img.shields.io/crates/v/compliance-probe.svg)](https://crates.io/crates/compliance-probe)
[![docs.rs](https://img.shields.io/docsrs/compliance-probe)](https://docs.rs/compliance-probe)

A live TR 101 290 + PCR-drift + SCTE-35 cue-sanity probe (issue #930,
`docs/IDEAS.md` item #4): point it at a continuous feed and it exports what
it finds through the [`metrics`](https://docs.rs/metrics) facade, so a host
process (e.g. `multimux`, which already installs a process-wide
`metrics_exporter_prometheus` recorder) can render it as Prometheus.

**This crate does not open a socket, run a background thread, or serve
`/metrics`.** It is a library: feed it TS packets / SCTE-35 sections /
`media_plane::ByteTap`+`EventCursor` items, and it records metrics through
whichever recorder the host already installed.

## What is measured

- **ETSI TR 101 290 priority 1/2/3 indicators** — every indicator
  [`dvb-conformance`](https://docs.rs/dvb-conformance) implements, fed one TS
  packet at a time via [`Probe::feed_ts_packet`]. Continuity-count errors are
  one of these indicators (`indicator="Continuity_count_error"`), not a
  separate metric.
- **PCR interval-error ("drift") and jitter**, per PID — this crate's own
  software-clock estimate of signalled-PCR-interval vs. wall-clock arrival.
  See [`conformance`] for the exact arithmetic and why it is *not* the
  spec's `PCR_accuracy_error` (2.4) under a different name.
- **SCTE-35 cue sanity** — well-formedness (wire path only), cues arriving,
  and future-vs-past `pts_time` judgement (ANSI/SCTE 35 §9.7.3.1). See
  [`scte35`].
- **Container/stream structural checks** — delegated to `media-doctor`'s
  already-shipped `Diagnostic` harness, not reimplemented. See
  [`structural`].

## What is deliberately NOT measured

A green dashboard must mean "checked and passing", never "not checked" — so
these are stated plainly rather than left to be discovered from an
always-zero metric:

- **TR 101 290 indicator 2.4, `PCR_accuracy_error`** — `dvb-conformance`
  itself never emits it (needs ±500 ns hardware arrival timing a sans-IO
  monitor cannot honestly provide).
- **T-STD buffer-model sub-checks** `dvb-conformance` documents as partial
  (`Buffer_error` TBn overflow; `Empty_buffer_error` MBn; `Data_delay_error`'s
  still-picture 60 s threshold) — this crate adds no buffer-model logic of
  its own.
- **SCTE-35 malformed-section detection on the Trunk-cursor path** — an event
  already published into a `media_plane::Trunk`'s event log was, by
  construction, already parsed successfully upstream; only the wire path
  ([`Probe::feed_scte35_section`]) can observe malformedness.
- **A fabricated "now" for an unresolved SCTE-35 anchor** — a
  `media_plane::EventAnchor::Segment`/`Utc` entry is left exactly that, never
  guessed into a `Media` time.

## The clock you feed is part of the measurement

`Probe::feed_ts_packet` takes an arrival timestamp, and most TR 101 290
indicators are **timeout-based** (PAT >500 ms, PID >5 s, SI 2/10/30 s, and the
T-STD buffer model's 1 Mbit/s TBsys drain). The clock is an *input to the
result*, not bookkeeping.

`tests/wasm_analyzer_equivalence.rs` pins a measured instance of this against
the `demo/` WASM analyzer over the same fixture. Both tools use the identical
default `dvb_conformance::Config` — there is no threshold difference — yet
they report 838 vs. 803 events on `fixtures/ts/m6-single.ts`. The reason:
that analyzer anchors its clock on observed PCR values with a `+1 ns`
per-packet fallback, and **the fixture contains no PCR at all**, so it models
1264 packets as spanning 1.264 µs (~1.5 Tbit/s). At that implied rate TBsys
cannot drain, so indicator 3.3 `Buffer_error` fires 35 times; at any realistic
bitrate it fires zero times. That accounts for the entire gap:

| Indicator | Degenerate clock (analyzer) | Realistic clock (40 µs/pkt) |
|---|---|---|
| `Continuity_count_error` | 803 | 803 |
| `Buffer_error` (T-STD 3.3) | 35 | 0 |
| **Total** | **838** | **803** |

So: **feed real arrival time** (`Probe::drain_byte_tap` does, using the
`ByteTap`'s recorded `Timestamp`), and treat
`Buffer_error`/`Empty_buffer_error`/`Data_delay_error` as only as trustworthy
as that clock. `Continuity_count_error` is clock-independent — the test
asserts it is 803 at every rate from frozen to 2 ms/packet.

## `media-plane` integration: one cursor, not a second copy

TR 101 290 needs raw wire bytes (a demuxed `transmux::Sample` has already
discarded `continuity_counter`/`tei`/`scrambling` — see
`media_plane::byte_tap`'s own docs), so [`Probe::drain_byte_tap`] attaches to
a [`media_plane::ByteTap`] at `TapPoint::Wire`. SCTE-35 sanity *can* use a
real `Trunk` cursor (the event log already carries a parsed
`timed_metadata::TimedEvent`), so [`Probe::drain_event_cursor`] attaches to a
[`media_plane::EventCursor`] via [`Trunk::subscribe_events`]. Both are
bounded, non-blocking observers — a probe never costs the ingest path a
second copy of the stream or a blocking reader.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library; enables metric recording through the `metrics` facade and the `media-plane` Trunk/ByteTap bridge (`trunk_bridge`). Without it the crate is `#![no_std]` + `alloc`: every check still runs, it is simply not reported. |

## Quick start

```rust,no_run
use std::time::Duration;
use compliance_probe::Probe;

let mut probe = Probe::new();
let packet: [u8; 188] = [0u8; 188]; // a real 188-byte TS packet
probe.feed_ts_packet(&packet, Duration::from_millis(0));

let stats = probe.conformance_stats();
println!("packets: {}, in sync: {}", stats.packets, stats.in_sync);
```

See `examples/fixture_report.rs` (a plain byte-buffer feed) and
`examples/byte_tap_live.rs` (the `media_plane::ByteTap` attachment point) for
runnable, fixture-driven demonstrations.

## Grafana dashboard

`dashboards/compliance-probe.json` — panels for every metric this crate
exports, a text panel listing what is deliberately not monitored, and a
priority-1 TR 101 290 alert rule. Import directly into Grafana against
whichever Prometheus scrapes the host process's `/metrics`.

## MSRV

Rust **1.95.0**.

## References

- ETSI TR 101 290 v1.4.1 (2020-06) — DVB Measurement Guidelines
- ANSI/SCTE 35:2023 — Digital Program Insertion Cueing Message for Cable
- ISO/IEC 13818-1 — MPEG-2 Systems (PCR, §2.4.2.2)
