# dash-runtime

Sans-IO MPEG-DASH core (ISO/IEC 23009-1): the MPD model and parser, segment addressing, the UTC clock and the live-edge arithmetic a DASH client is built on. No I/O, no threads, no clock reads.

[![crates.io](https://img.shields.io/crates/v/dash-runtime.svg)](https://crates.io/crates/dash-runtime)
[![docs.rs](https://docs.rs/dash-runtime/badge.svg)](https://docs.rs/dash-runtime)

Not yet published: version 0.1.0 is the core only (design step W-c in `docs/dash-runtime/DESIGN.md`). The client state machine and the host adapters follow in later steps.

## What it does / what it does not

**Does.**
- Parses an MPD (static or dynamic) into a typed, consume-only model with hostile-input bounds: input size, nesting depth, element count, attribute length and count, Periods, AdaptationSets, Representations, timeline entries and `BaseURL` alternates all have named limits and typed errors. DOCTYPE declarations are refused, so no entity can be declared.
- Resolves Segment information down Period > AdaptationSet > Representation (lower level wins attribute by attribute) and turns it into a lazy Segment plan: `SegmentTemplate` with width tags and `$$`, `SegmentTimeline` with negative `@r` and `@n`, `SegmentList`, and `SegmentBase` with a `sidx`.
- Computes, from instants you pass in, the availability window of every segment, the newest complete segment, the oldest one still in the time-shift window, the presentation delay and the segment to join.
- Maps your monotonic clock to UTC (`UtcClock`) from `UTCTiming` measurements, with a trust-ordered replacement rule and clock-step detection.
- Resolves URLs by RFC 3986 through `BaseURL` alternates ordered by DVB priority, and applies the minimal `urn:mpeg:dash:urlparam:2014` query.
- Chooses AdaptationSets and Representations by role, language, codec and bandwidth.

**Does not.**
- Fetch anything, read a clock, or decide when to retry: the caller owns I/O and time.
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
| ETSI TS 103 285 | 1.4.1 | 10.8.2 BaseURL `@dvb:priority` | Priority only | |
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

Examples (fixtures read through `std::fs` at run time):

```bash
cargo run -p dash-runtime --example list_segments
cargo run -p dash-runtime --example live_window
```

## Verification

Independent oracles, not this crate's own output:

- Real MPDs and media from ffmpeg 8.1 and GPAC 26.07 (`fixtures/dash/`), with MP4Box `-diso` and ffprobe output next to each: the expanded segment URLs equal the files on disk, `$Time$` equals the `sidx` earliest presentation time (and differs from `tfdt` where the real files do), and every `sidx` byte range starts at a `moof` and tiles the file.
- Real DASH-IF livesim2 MPDs (dynamic, `$Time$` and `$Number$` timelines, two consecutive refreshes): numbering stays stable across the refresh.
- `transmux::dash_parse` and its generator's goldens: both parsers agree on every shared field, segment number/time and resolved URL, and the lazy timeline equals `SegmentTimeline::resolve` on 20 000 random timelines.
- The IOP v4.3 Table 10 worked example and the `xs:dateTime` accept/reject table of the notes.
- A hostile corpus (entity expansion, depth, element count, huge `@r`, NUL bytes, invalid UTF-8) and a mutation table (`docs/mutation-table.md`).

Tripwires: `tests/no_clock_guard.rs` (no `now()` outside tests), `tests/no_handroll_guard.rs`, `tests/label_coverage.rs`, `tests/non_exhaustive_coverage.rs`. Fuzz targets `dash_mpd_parse` and `dash_address` live in the workspace `fuzz/` crate.

## Feature flags

None. The crate is `std` (the XML reader, `quick-xml`, needs it).

## Related crates

`transmux` (the DASH generator this crate is tested against, `Fmp4Demux`, `sidx` and duration parsers it reuses), `hls-runtime` (the HLS counterpart), `broadcast-client` (the client session that will drive this core).

## License

MIT OR Apache-2.0
