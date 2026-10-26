# dash-runtime

Sans-IO MPEG-DASH (ISO/IEC 23009-1): the MPD model and parser, segment addressing, the UTC clock and the live-edge arithmetic a DASH client is built on, and `DashClient`, a client state machine that plays static (VOD) presentations to ordered `Fmp4Demux` samples. No I/O, no threads, no clock reads.

[![crates.io](https://img.shields.io/crates/v/dash-runtime.svg)](https://crates.io/crates/dash-runtime)
[![docs.rs](https://docs.rs/dash-runtime/badge.svg)](https://docs.rs/dash-runtime)

Not yet published: version 0.1.0 is the core (design step W-c in `docs/dash-runtime/DESIGN.md`) plus the static-presentation client (step W-d1). Live presentations (W-d2) and the host adapters follow in later steps; a dynamic MPD is answered with a typed "live not yet supported" error and a fatal output.

## What it does / what it does not

**Does.**
- Parses an MPD (static or dynamic) into a typed, consume-only model with hostile-input bounds: input size, nesting depth, element count, attribute length and count, Periods, AdaptationSets, Representations, timeline entries and `BaseURL` alternates all have named limits and typed errors. DOCTYPE declarations are refused, so no entity can be declared.
- Resolves Segment information down Period > AdaptationSet > Representation (lower level wins attribute by attribute) and turns it into a lazy Segment plan: `SegmentTemplate` with width tags and `$$`, `SegmentTimeline` with negative `@r` and `@n`, `SegmentList`, and `SegmentBase` with a `sidx`.
- Computes, from instants you pass in, the availability window of every segment, the newest complete segment, the oldest one still in the time-shift window, the presentation delay and the segment to join.
- Maps your monotonic clock to UTC (`UtcClock`) from `UTCTiming` measurements, with a trust-ordered replacement rule and clock-step detection.
- Resolves URLs by RFC 3986 through `BaseURL` alternates ordered by DVB priority, and applies the minimal `urn:mpeg:dash:urlparam:2014` query.
- Chooses AdaptationSets and Representations by role, language, codec and bandwidth.
- Plays a static MPD (`DashClient`): fetches inits (one per distinct URL and range, concurrently) and `sidx` indexes, plans and requests segments inside a bounded in-flight window, demuxes each segment as `init ++ segment` with `transmux::Fmp4Demux`, delivers samples in order per track whatever order the responses arrive in, and plays multiple Periods back to back with `period-continuity` honoured (no discontinuity, no second init) and every other boundary announced as a `Discontinuity`.
- Classifies failures for the caller (`FailureClass`): retry with the attempt count, give up after `max_segment_failures` consecutive failures (fatal for a static presentation, the `broadcast-client` rule), or the missing tail of a Period. The backoff arithmetic stays with the caller.
- Refuses what it cannot play instead of misplaying it: encrypted, MPEG-2 TS, WebM, subtitle and trick-mode sets are `TrackDropped`, a progressive MP4 served as a segment is fatal, and with `cross_origin_urls = false` no URL the MPD derives may leave the MPD's origin.

**Does not.**
- Fetch anything, read a clock, or decide when to retry: the caller owns I/O and time.
- Play a dynamic MPD: no clock ladder, availability gating, live join or MPD refresh yet (design step W-d2). `Error::LiveNotYetSupported` is temporary.
- Seek, adapt automatically (the caller picks with `select_representation`), decrypt, or surface `EventStream` messages.
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
| | | 5.8.5.7 UTCTiming | Parsed and sampled | `ntp`/`sntp` recognised, not sampled (no UDP in a sans-IO core); `http-ntp` reads an 8-byte RFC 5905 timestamp |
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
use dash_runtime::client::{Action, DashClient, DashConfig};

let mut client = DashClient::new("https://origin.example/vod/manifest.mpd", DashConfig::default());
let Some(Action::FetchMpd { url }) = client.poll(0) else { unreachable!() };
assert_eq!(url, "https://origin.example/vod/manifest.mpd");
// GET it, then client.on_mpd(now_us, &body); poll() now yields FetchInit / FetchIndex /
// FetchSegment actions, each answered with on_init / on_index / on_segment (or
// on_failure), and next_output() yields Manifest, Tracks, Samples ... EndOfStream.
```

Examples (fixtures read through `std::fs` at run time):

```bash
cargo run -p dash-runtime --example list_segments
cargo run -p dash-runtime --example live_window
cargo run -p dash-runtime --example play_static -- --mpd fixtures/dash/multiperiod-restart/multiperiod.mpd
```

## Static presentations: behaviour worth knowing

- **One timeline.** The selected video and audio Representations of a Period are requested interleaved by media start time (the stream that is furthest behind goes first), inside one window of `max_inflight_segments` that counts requested and arrived-but-undelivered segments. Track ids are 1-based, video then audio, and a Representation switch keeps them.
- **Samples keep the container timeline** (`tfdt`), not a Period-rebased one; `Output::PeriodChanged` carries the PeriodStart for a host that wants to rebase. A restart Period (media timeline reset) is announced with `Discontinuity`; a `period-continuity` Period with an unchanged init joins silently.
- **The GPAC 13 ms tail (decision).** GPAC declares `Period@duration="PT6.013S"` for a Period whose files hold 6.000 s of media, so the segment plan has a fourth segment per stream that no file backs (`fixtures/dash/multiperiod-restart/`; the 404 is real, the plan follows 5.3.9.5.3's `PeriodEnd - start` rule). The client follows the plan, requests the segment once, and treats a 404 or 410 on the *last* segment of a Period whose planned duration is under 50 ms (a frame of the slowest common video rate, `MISSING_TAIL_MAX_US`) as the end of that Period: `FailureClass::EndTail`, not counted, not retried, no `Discontinuity`. Why not plan only the files that exist: the client cannot know what exists before asking, and trimming plans by a guessed threshold would also drop a genuinely short last segment. Why not retry: the 404 is permanent and a retry loop would end the whole presentation as `SegmentFetchFailed`. A missing last segment of full length, a missing middle segment, and any non-404/410 failure of the tail stay ordinary counted failures (tests `a_missing_full_length_last_segment_is_not_a_tail`, `a_short_segment_in_the_middle_is_not_a_tail`).
- **A `425` counts.** DESIGN section 5.4 never counts `425` (too early) for live; a static presentation has no availability window, so here it is an ordinary counted failure and cannot loop forever. W-d2 changes this for dynamic MPDs only.
- **Ranged responses must be exact.** A response to a byte-range request must have exactly the requested length (an adapter slices a `200` that ignored `Range` before delivering); anything else is `Rejection::WrongLength`, counted.

## Verification

Independent oracles, not this crate's own output:

- Real MPDs and media from ffmpeg 8.1 and GPAC 26.07 (`fixtures/dash/`), with MP4Box `-diso` and ffprobe output next to each: the expanded segment URLs equal the files on disk, `$Time$` equals the `sidx` earliest presentation time (and differs from `tfdt` where the real files do), and every `sidx` byte range starts at a `moof` and tiles the file.
- Real DASH-IF livesim2 MPDs (dynamic, `$Time$` and `$Number$` timelines, two consecutive refreshes): numbering stays stable across the refresh.
- `transmux::dash_parse` and its generator's goldens: both parsers agree on every shared field, segment number/time and resolved URL, and the lazy timeline equals `SegmentTimeline::resolve` on 20 000 random timelines.
- The IOP v4.3 Table 10 worked example and the `xs:dateTime` accept/reject table of the notes.
- A hostile corpus (entity expansion, depth, element count, huge `@r`, NUL bytes, invalid UTF-8) and a mutation table (`docs/mutation-table.md`).
- The client: every static fixture (ffmpeg and GPAC timelines, `SegmentList`, on-demand `sidx` including GPAC's nonsense `presentationTimeOffset`, multi-Period restart and continuous) plays through a loopback host that serves the real files by URL and byte range, and the delivered samples (payload, pts, dts, duration, flags, count per track) equal what `Fmp4Demux` makes of the concatenated init and segments, for four completion orders and window sizes 1 to 8. A failure matrix injects eleven faults (404, 410, 425, 429, 500, 503, transport, timeout, truncated, header-only, garbage 200) transiently and permanently on a segment, an init, the `sidx` and ranged subsegments, MPD included. 48 client mutants are all killed (`docs/mutation-table-client.md`); the fuzz target `dash_client_static` drives the whole client with arbitrary MPDs and steered, damaged answers.

Tripwires: `tests/no_clock_guard.rs` (no `now()` outside tests), `tests/no_handroll_guard.rs`, `tests/label_coverage.rs`, `tests/non_exhaustive_coverage.rs`. Fuzz targets `dash_mpd_parse`, `dash_address` and `dash_client_static` live in the workspace `fuzz/` crate.

## Feature flags

None. The crate is `std` (the XML reader, `quick-xml`, needs it).

## Related crates

`transmux` (the DASH generator this crate is tested against, `Fmp4Demux`, `sidx` and duration parsers it reuses), `container-probe` (the structural "is this body a media segment" check), `hls-runtime` (the HLS counterpart), `broadcast-client` (the client session that will drive this client).

## License

MIT OR Apache-2.0
