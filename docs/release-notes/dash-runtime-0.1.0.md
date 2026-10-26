---
status: DRAFT
---
# dash-runtime 0.1.0

_Released 2026-10-26._

Initial release. `dash-runtime` is a **sans-IO MPEG-DASH client toolkit** (ISO/IEC 23009-1): a bounded MPD parser and typed model, a lazy segment plan for every addressing mode, the UTC clock and live-edge arithmetic a live client needs, and `DashClient`, a state machine that plays static (VOD) and dynamic (live) presentations to ordered, demuxed samples. It performs no I/O, spawns nothing and reads no clock: the host fetches what the client asks for, tells it the time, and reports what happened. `broadcast-client` is the first consumer (`SourceKind::Dash`); the design, including every decision where the standard is silent or contradicts itself, is `docs/dash-runtime/DESIGN.md`.

## Quick start

```rust
use dash_runtime::client::{Action, DashClient, DashConfig, PullLimits};

let mut client = DashClient::new("https://origin.example/live/manifest.mpd", DashConfig::default(), PullLimits::default());
client.set_wall_clock(0, 1_788_000_000_000_000);
let Some(Action::FetchMpd { .. }) = client.poll(0) else { unreachable!() };
```

This is a README snippet. `examples/play_static.rs` and `examples/play_live.rs` play committed fixtures through a loopback host with no network; `examples/list_segments.rs` and `examples/live_window.rs` show the segment plan and the live window.

## Surface

| Piece | Provided |
|---|---|
| `mpd` | `Mpd`: a consume-only model behind a bounded, namespace-aware parser (size, depth, element and attribute limits, DOCTYPE refused), attribute inheritance down Period, AdaptationSet and Representation, `BaseURL` alternates ordered by DVB priority, descriptors, `urn:mpeg:dash:urlparam:2014`. An MPD whose Periods share an `@id` is rejected. |
| `address` | `SegmentPlan`: `SegmentTemplate` (width tags, `$$`), `SegmentTimeline` (negative `@r`, `@n`), `SegmentList`, `SegmentBase` + `sidx`, each segment with its availability window |
| `timing` | `UtcClock`, `UTCTiming` sampling (`direct`, `http-xsdate`, `http-iso`, `http-ntp`, `http-head`), `xs:dateTime`, HTTP-date and NTP readers, PeriodStart, availability and live-edge arithmetic |
| `selection` | Pure choice of AdaptationSet and Representation by role, language, codec and bandwidth |
| `client::DashClient` | `poll`/`next_output`, `on_mpd_with`, `on_mpd_unchanged`, `on_init`, `on_index`, `on_segment`, `on_failure`, `on_utc_response`, `set_wall_clock`, `select_representation`, `select_audio`, `next_wake`, `awaits`, `state_size` |
| `client::DashConfig` | The DASH knobs: clock policy, `UTCTiming` timeouts and round-trip limit, resync period, presentation delay, availability margin, early-request cadence and grace, refresh floor, cross-origin pin |

## Behaviour guarantees

- **No clock, no I/O.** Every instant is an argument; `tests/no_clock_guard.rs` bans `Instant::now`, `SystemTime::now` and the other readers outside tests, and `broadcast-client` carries the same guard over both crates.
- **Never early.** A live segment is requested at its availability start, measured on a clock the client chose; an early answer (`425`, or `404`/`410` inside the grace window) is retried at 250 ms doubling to 1 s and never counted against the segment.
- **Falling behind is announced.** A segment that left the time-shift window is skipped to the oldest one still there with a `Discontinuity`; three in a row rejoin the live edge. A segment still answered `425` after its window closed is expired under a `UTCTiming` clock instead of being retried forever.
- **Every clock has a fallback and a failure.** `UTCTiming`, then the host clock, then the MPD response `Date`; no source at all is a typed fatal `ClockUnavailable`.
- **Refuses what it cannot play.** Encrypted, MPEG-2 TS, WebM, subtitle and trick-mode sets are `TrackDropped`; a progressive MP4 served as a segment is fatal; with `cross_origin_urls = false` no URL the MPD derives may leave the MPD's origin.
- **Bounded state.** A day of live playback leaves every internal collection at a constant size.

## Validation

- Real MPDs and media from ffmpeg 8.1 and GPAC 26.07, and two real DASH-IF livesim2 MPD refreshes, with MP4Box and ffprobe output next to each; every static fixture plays to samples equal to a direct `Fmp4Demux` of the same files.
- The IOP v4.3 Table 10 worked example, a differential against `transmux`'s DASH generator, and a hostile-input corpus.
- The `World` simulation (true UTC, a host clock with skew, drift and steps, an origin that decides availability from true time): clock ladder, refresh, Periods, failures, a 24-hour run and a 4 000-session randomised soak. Mutation tables for the core, the static client and the live client.
- Through `broadcast-client`: a 45-fault matrix over eight presentation shapes, the fuzz target `dash_session`, and 30 s of live DASH from livesim2 (pinned v1.15.2) over real HTTP. The matrix found two defects fixed in this release (a duplicated Period `@id`, and a `425` retried past its window).

## Known limits

- Low-latency DASH plays at segment granularity (`Notice::LowLatencyIgnored`).
- No seeking, no automatic bitrate adaptation, no decryption, no `EventStream` surfacing, no MPD patch or xlink resolution (xlink `resolve-to-zero` is honoured).
- Segment sequences (`S@k` above 1, `$SubNumber$`), `MPD@type="list"`, `data:` addresses, open-ended byte ranges and hierarchical `sidx` are typed `Unsupported` errors.
- A host clock more than a time-shift buffer wrong, with no `UTCTiming` and no better source, cannot be repaired: the client waits for the future, or plays late.

## Dependencies

`broadcast-common` 10.0, `transmux` 0.28, `container-probe` (the minor that carries the `segment` module), `quick-xml`, `url`, `jiff`, `thiserror`. `std` only; MSRV 1.95.0.

## Release order

`container-probe` first (the `segment` module), then `dash-runtime`, then `broadcast-client`.

## Migration

New crate; nothing to migrate. `dash-runtime = "0.1"`.

---

Published from tag `dash-runtime-v0.1.0`.
