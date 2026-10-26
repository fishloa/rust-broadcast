# dash-runtime

Sans-IO MPEG-DASH (ISO/IEC 23009-1): the MPD model and parser, segment addressing, the UTC clock and the live-edge arithmetic a DASH client is built on, and `DashClient`, a client state machine that plays static (VOD) and dynamic (live) presentations to ordered `Fmp4Demux` samples. No I/O, no threads, no clock reads.

[![crates.io](https://img.shields.io/crates/v/dash-runtime.svg)](https://crates.io/crates/dash-runtime)
[![docs.rs](https://docs.rs/dash-runtime/badge.svg)](https://docs.rs/dash-runtime)

Not yet published: version 0.1.0 is the core (design step W-c in `docs/dash-runtime/DESIGN.md`), the static-presentation client (step W-d1) and the live client (step W-d2). `broadcast-client` plays DASH through it (step W-e); `multimux` follows in a later step.

## What it does / what it does not

**Does.**
- Parses an MPD (static or dynamic) into a typed, consume-only model with hostile-input bounds: input size, nesting depth, element count, attribute length and count, Periods, AdaptationSets, Representations, timeline entries and `BaseURL` alternates all have named limits and typed errors. DOCTYPE declarations are refused, so no entity can be declared.
- Resolves Segment information down Period > AdaptationSet > Representation (lower level wins attribute by attribute) and turns it into a lazy Segment plan: `SegmentTemplate` with width tags and `$$`, `SegmentTimeline` with negative `@r` and `@n`, `SegmentList`, and `SegmentBase` with a `sidx`.
- Computes, from instants you pass in, the availability window of every segment, the newest complete segment, the oldest one still in the time-shift window, the presentation delay and the segment to join.
- Maps your monotonic clock to UTC (`UtcClock`) from `UTCTiming` measurements, with a trust-ordered replacement rule and clock-step detection.
- Resolves URLs by RFC 3986 through `BaseURL` alternates ordered by DVB priority, and applies the minimal `urn:mpeg:dash:urlparam:2014` query.
- Chooses AdaptationSets and Representations by role, language, codec and bandwidth, and switches them while playing: `select_representation` for video, `select_audio` for the audio AdaptationSet (or `None` to stop requesting audio, and a later `Some` to start it again), each continuing from the segment that contains the next unrequested media time.
- Plays a static MPD (`DashClient`): fetches inits (one per distinct URL and range, concurrently) and `sidx` indexes, plans and requests segments inside a bounded in-flight window, demuxes each segment as `init ++ segment` with `transmux::Fmp4Demux`, delivers samples in order per track whatever order the responses arrive in, and plays multiple Periods back to back with `period-continuity` honoured (no discontinuity, no second init) and every other boundary announced as a `Discontinuity`.
- Plays a dynamic MPD (live): finds UTC through a ladder (`UTCTiming` sampled through the client's own requests, `set_wall_clock`, or the MPD response `Date`), joins near the live edge at the presentation delay, never requests a segment before it exists, refreshes the MPD (conditional requests, `minimumUpdatePeriod` floored at one second, stale `publishTime` ignored, `Location` and redirects followed), follows Periods as they are added and expire, handles MPD Reset and the dynamic-to-static transition, and tells an early segment (`425`, or `404`/`410` inside the grace window) from an expired one and from a missing one.
- Classifies failures for the caller (`FailureClass`): retry with the attempt count, give up after `max_segment_failures` consecutive failures (fatal for a static presentation, the `broadcast-client` rule; a lost segment with a `Discontinuity` for a live one), not-yet-available, expired, or the missing tail of a Period. The backoff arithmetic stays with the caller; the early-request cadence is the client's.
- Refuses what it cannot play instead of misplaying it: encrypted, MPEG-2 TS, WebM, subtitle and trick-mode sets are `TrackDropped`, a progressive MP4 served as a segment is fatal, and with `cross_origin_urls = false` no URL the MPD derives may leave the MPD's origin.

**Does not.**
- Fetch anything, read a clock, or decide when to retry: the caller owns I/O and time.
- Deliver low-latency DASH at chunk granularity: an LL-DASH MPD plays at segment granularity with `Notice::LowLatencyIgnored`; `DashConfig::use_availability_time_offset` requests early but still delivers whole segments.
- Seek, adapt automatically (the caller picks with `select_representation` and `select_audio`), decrypt, or surface `EventStream` messages.
- Serialize an MPD. The model is consume-only; the round-trip invariant is met by a differential against `transmux`'s DASH generator (see Verification).
- Demultiplex media, decrypt, or handle MPEG-2 TS / WebM segments, subtitles, trick modes, EventStream surfacing, MPD patch or xlink resolution (xlink `resolve-to-zero` is honoured, other remote elements are reported).
- Implement segment sequences (`S@k` above 1, `$SubNumber$`), `MPD@type="list"`, `data:` addresses, open-ended byte ranges or hierarchical `sidx`: each is a typed `Unsupported` error that drops the Representation instead of misreading it.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ISO/IEC 23009-1 | 2022 (5th ed.) | 5.3.1-5.3.7 MPD, Period, AdaptationSet, Representation | Full (consumed) | `Preselection`, `EventStream`, `ServiceDescription`, `Resync` are flagged, not modelled |
| | | 5.3.9 Segment information, 5.3.9.4 templates, 5.3.9.5 derivation, 5.3.9.6 timeline | Full | `$Number$` is `startNumber + k - 1` (see DESIGN section 11); `$Time$` uses the raw `S@t` even with a non-zero `@presentationTimeOffset` (documented choice) |
| | | 5.3.2.1 PeriodStart rules, Period kinds | Full | |
| | | 5.6 BaseURL, 5.6.5 alternates | Full | `@weight` parsed, not used (no random source) |
| | | 5.8.5.7 UTCTiming | Parsed, sampled, used | `direct`, `http-xsdate`, `http-iso`, `http-ntp` and `http-head` (re-issued as `GET` after `405`/`501`) drive the clock; `ntp`/`sntp` are skipped with `Notice::UtcSchemeUnsupported` (no UDP in a sans-IO core); `http-ntp` reads an 8-byte RFC 5905 timestamp |
| | | 5.4 MPD updates, 5.4.2 MPD Reset | Client behaviour | stale `publishTime` ignored, `MPD@id`/`availabilityStartTime` change or the reset descriptor rejoins the live edge |
| | | Annex A.2, A.7, A.11 | Client behaviour | FetchTime at the exchange midpoint; early/expired/missing classification with a clock re-sync and MPD refresh on the first in-window error (at most one re-sync per 30 s); `Location` over redirect over the original URL |
| RFC 8470 | 2018 | 425 Too Early | Used | classified as not-yet-available for a live presentation |
| | | Annex A.3 availability, Annex I.2 URL parameters | Full / minimal | Only `$querypart$` or no `@queryTemplate` |
| DASH-IF IOP | v4.3 | 4.3.3.2 live timing, 4.7 UTC timing | Full | Table 10 worked example is a test; the last (short) segment follows 5.3.9.5.3, not the IOP's nominal duration |
| ETSI TS 103 285 | 1.4.1 | 10.8.2 BaseURL `@dvb:priority` | Priority, rotation after 2 failures | `@weight` parsed, never used |
| ISO/IEC 23009-1 | 2022 (5th ed.) | 5.3.2.4 continuity, A.9 playback across Periods | Continuity honoured, connectivity treated as a discontinuity | the client; `docs/specs/dash/23009-1/` |
| ISO/IEC 14496-12 | 2022 | 8.8.4 `moof`/`mdat`, 8.16.3 `sidx` | Used | structural segment check via `container-probe`; `sidx` ranges via `transmux` |
| W3C XML Schema Part 2 | 2004 | 3.2.7 `xs:dateTime` | Accepted subset | `docs/specs/dash/xs-datetime-subset.md` |

## Quick start

```rust
use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;

let xml = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
                  mediaPresentationDuration="PT6S">
  <Period><AdaptationSet mimeType="video/mp4">
    <SegmentTemplate timescale="1000" duration="2000" startNumber="0"
                     media="$RepresentationID$/$Number%03d$.m4s"/>
    <Representation id="v1" bandwidth="500000"/>
  </AdaptationSet></Period>
</MPD>"#;
let mpd = Mpd::parse(xml.as_bytes()).unwrap();
let planned = plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
for seg in planned.plan.iter() {
    println!("{} {}", seg.number, seg.reference);
}
```

Playing a static MPD (the host performs the I/O):

```rust
use dash_runtime::client::{Action, DashClient, DashConfig, PullLimits};

let mut client = DashClient::new("https://origin.example/vod/manifest.mpd", DashConfig::default(), PullLimits::default());
let Some(Action::FetchMpd { url, .. }) = client.poll(0) else { unreachable!() };
assert_eq!(url, "https://origin.example/vod/manifest.mpd");
// GET it, then client.on_mpd(now_us, &body); poll() now yields FetchInit / FetchIndex /
// FetchSegment actions, each answered with on_init / on_index / on_segment (or
// on_failure), and next_output() yields Manifest, Tracks, Samples ... EndOfStream.
```

Playing a live MPD (the host performs the I/O and tells the client the time):

```rust
use dash_runtime::client::{Action, DashClient, DashConfig, PullLimits};

let mut client = DashClient::new("https://origin.example/live/manifest.mpd", DashConfig::default(), PullLimits::default());
// Optional: the host's UTC clock, read back to back with the monotonic clock. Without it
// (and without a UTCTiming element) the client falls back to the MPD response's Date.
client.set_wall_clock(0, 1_788_000_000_000_000);
let Some(Action::FetchMpd { .. }) = client.poll(0) else { unreachable!() };
// After on_mpd_with(now, &body, &ResponseMeta::new(sent, received).with_date(date)), poll()
// yields FetchUtcTiming (answer with on_utc_response), FetchInit and FetchSegment
// actions; call poll() again whenever a response arrives or next_wake() passes.
```

Examples (fixtures read through `std::fs` at run time):

```bash
cargo run -p dash-runtime --example list_segments
cargo run -p dash-runtime --example live_window
cargo run -p dash-runtime --example play_static -- --mpd fixtures/dash/multiperiod-restart/multiperiod.mpd
cargo run -p dash-runtime --example play_live
cargo run -p dash-runtime --example play_live -- --mpd fixtures/dash/live-timeline/live.mpd --seconds 4 --skew-ms 1500
```

## Static presentations: behaviour worth knowing

- **One timeline.** The selected video and audio Representations of a Period are requested interleaved by media start time (the stream that is furthest behind goes first), inside one window of `max_inflight_segments` that counts requested and arrived-but-undelivered segments. Track ids are 1-based, video then audio, and a Representation switch keeps them.
- **Samples keep the container timeline** (`tfdt`), not a Period-rebased one; `Output::PeriodChanged` carries the PeriodStart for a host that wants to rebase. A restart Period (media timeline reset) is announced with `Discontinuity`; a `period-continuity` Period with an unchanged init joins silently.
- **The GPAC 13 ms tail (decision).** GPAC declares `Period@duration="PT6.013S"` for a Period whose files hold 6.000 s of media, so the segment plan has a fourth segment per stream that no file backs (`fixtures/dash/multiperiod-restart/`; the 404 is real, the plan follows 5.3.9.5.3's `PeriodEnd - start` rule). The client follows the plan, requests the segment once, and treats a 404 or 410 on the *last* segment of a Period whose planned duration is under 50 ms (a frame of the slowest common video rate, `MISSING_TAIL_MAX_US`) as the end of that Period: `FailureClass::EndTail`, not counted, not retried, no `Discontinuity`. Why not plan only the files that exist: the client cannot know what exists before asking, and trimming plans by a guessed threshold would also drop a genuinely short last segment. Why not retry: the 404 is permanent and a retry loop would end the whole presentation as `SegmentFetchFailed`. A missing last segment of full length, a missing middle segment, and any non-404/410 failure of the tail stay ordinary counted failures (tests `a_missing_full_length_last_segment_is_not_a_tail`, `a_short_segment_in_the_middle_is_not_a_tail`).
- **A `425` counts for a static presentation only.** DESIGN section 5.4 never counts `425` (too early) for live; a static presentation has no availability window, so there it is an ordinary counted failure and cannot loop forever. A dynamic presentation treats it as not-yet-available (never counted, retried at 250 ms doubling to 1 s).
- **Ranged responses must be exact.** A response to a byte-range request must have exactly the requested length (an adapter slices a `200` that ignored `Range` before delivering); anything else is `Rejection::WrongLength`, counted.

## Live presentations: behaviour worth knowing

- **The clock ladder.** `Auto` (default): `UTCTiming` (it is the packager's own time authority, sampled at the midpoint of the exchange, uncertainty half the round trip plus the source's resolution) over the host clock (`set_wall_clock`, 500 ms uncertainty by default) over the MPD response `Date` (whole seconds, so +-0.5 s more). `PreferHost` takes the host clock first. No source at all is `FatalKind::ClockUnavailable`. A static presentation needs none. Inits are fetched while the UTC sample is taken; the join waits at most `utc_wait_us`.
- **Never early.** A segment is requested at its availability start (plus `availability_margin_us`, and minus `availabilityTimeOffset` only with `use_availability_time_offset`). Every timeline-listed segment is gated by the formula too, stricter than the IOP's "listed means available" note. With a wrong clock the error is absorbed: early answers are `NotYetAvailable`, never counted; a `404` that persists past the grace window is `Missing`.
- **Joining.** `T_join = utc - delay - (AST + PeriodStart)`; each Representation joins on the segment that contains `T_join` on its own timeline (audio and video boundaries differ), clamped to the window. The delay is `suggestedPresentationDelay`, else `max(4 s, 3 x maxSegmentDuration)`, clamped to the time-shift buffer.
- **Falling behind.** A segment that left the time-shift window is `Expired` (not counted): the client skips to the oldest segment still there with a `Discontinuity` and `Notice::FellBehind`; three expiries in a row rejoin the live edge. A segment still answered `425` once its window has closed is expired the same way when the clock is the origin's own (`UTCTiming`); under a host or `Date` clock a `425` stays evidence that the clock is ahead, so the client keeps waiting for it.
- **Refresh.** Next fetch at `FetchTime + max(minimumUpdatePeriod, min_mpd_refresh_us)`; also forced (at most once per floor) when the listed segments run out, on the first in-window error of a Representation, on a clock step and on a BaseURL rotation. The request is conditional (`FetchMpd { validators }`, `304` is `on_mpd_unchanged`), a byte-identical body changes nothing, an older `publishTime` is ignored and retried at the floor.
- **State is bounded.** A day of live playback, with or without a new Period every ten minutes, leaves every internal collection at a constant size (`DashClient::state_size`, tests `live_longrun`).
- **A wrong host clock.** More than a time-shift buffer ahead of or behind the packager cannot be repaired without a better source: ahead, the client waits for the future and then skips; behind, it plays late. A `UTCTiming` element or the response `Date` is the cure; `Auto` prefers the host clock over `Date`, so a host with a bad clock and no `UTCTiming` plays wrongly rather than silently trusting `Date`.

## Verification

Independent oracles, not this crate's own output:

- Real MPDs and media from ffmpeg 8.1 and GPAC 26.07 (`fixtures/dash/`), with MP4Box `-diso` and ffprobe output next to each: the expanded segment URLs equal the files on disk, `$Time$` equals the `sidx` earliest presentation time (and differs from `tfdt` where the real files do), and every `sidx` byte range starts at a `moof` and tiles the file.
- Real DASH-IF livesim2 MPDs (dynamic, `$Time$` and `$Number$` timelines, two consecutive refreshes): numbering stays stable across the refresh.
- `transmux::dash_parse` and its generator's goldens: both parsers agree on every shared field, segment number/time and resolved URL, and the lazy timeline equals `SegmentTimeline::resolve` on 20 000 random timelines.
- The IOP v4.3 Table 10 worked example and the `xs:dateTime` accept/reject table of the notes.
- A hostile corpus (entity expansion, depth, element count, huge `@r`, NUL bytes, invalid UTF-8) and a mutation table (`docs/mutation-table.md`).
- The client: every static fixture (ffmpeg and GPAC timelines, `SegmentList`, on-demand `sidx` including GPAC's nonsense `presentationTimeOffset`, multi-Period restart and continuous) plays through a loopback host that serves the real files by URL and byte range, and the delivered samples (payload, pts, dts, duration, flags, count per track) equal what `Fmp4Demux` makes of the concatenated init and segments, for four completion orders and window sizes 1 to 8. A failure matrix injects eleven faults (404, 410, 425, 429, 500, 503, transport, timeout, truncated, header-only, garbage 200) transiently and permanently on a segment, an init, the `sidx` and ranged subsegments, MPD included. 48 client mutants are all killed (`docs/mutation-table-client.md`); the fuzz target `dash_client_static` drives the whole client with arbitrary MPDs and steered, damaged answers.

- The live client runs in the DESIGN 3.6 `World` simulation (true UTC, a host clock with skew, drift and steps, a fake origin that decides availability from TRUE time): a synthesised origin serving the four real fMP4 segments of the ffmpeg live capture recycled under a rewritten MPD (number, `$Number$` timeline, `$Time$` timeline, multi-Period), the two real ffmpeg live captures played exactly as written (samples equal a direct `Fmp4Demux`), and the two real livesim2 MPD refreshes. Cases: host skew +-0.5/5/60 s with and without `UTCTiming`, a skewed `UTCTiming` server, `Date` only, clock steps of +-30 s, six hours at +-100 ppm with re-sync, round trips over the limit, garbage/empty/huge UTC bodies, `HEAD` answered `405`, a UTC timeout falling to the next element and then to the host clock, `direct`, `PreferHost`, a static MPD with no clock. A failure-injection matrix (22 fault kinds x MPD refresh, segment, init and UTC x transient and permanent), a 24-hour bounded-memory run, a 4 000-session randomised soak and the fuzz target `dash_client_live`. Live mutants are in `docs/mutation-table-live.md`.

- Through `broadcast-client`: a fault matrix of 45 fault kinds (the 22 transport and status faults plus UTCTiming, MPD-refresh, Period-churn, `sidx`, `425`/`Retry-After`, container and clock faults) over six request classes, eight presentation shapes and transient and permanent occurrences, every cell asserting that the fault was applied; a real-wire run of 30 s of live DASH from DASH-IF livesim2 (pinned at v1.15.2) in six shapes; and the fuzz target `dash_session`. The matrix found two defects now fixed here: an MPD whose Periods share an `@id` replayed a Period's segments (it is rejected as invalid, 23009-1 5.3.2.2), and a segment still answered `425` after its availability window closed was retried forever under a `UTCTiming` clock (it is now expired like a `404`/`410`).

Tripwires: `tests/no_clock_guard.rs` (no `now()` outside tests; `broadcast-client` carries one over both crates' sources), `tests/no_handroll_guard.rs`, `tests/label_coverage.rs`, `tests/non_exhaustive_coverage.rs`. Fuzz targets `dash_mpd_parse`, `dash_address`, `dash_client_static`, `dash_client_live` and `dash_session` live in the workspace `fuzz/` crate.

## Feature flags

None. The crate is `std` (the XML reader, `quick-xml`, needs it).

## Related crates

`transmux` (the DASH generator this crate is tested against, `Fmp4Demux`, `sidx` and duration parsers it reuses), `container-probe` (the structural "is this body a media segment" check), `hls-runtime` (the HLS counterpart), `broadcast-client` (the client session that will drive this client).

## License

MIT OR Apache-2.0
