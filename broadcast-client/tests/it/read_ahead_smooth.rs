//! Read-ahead gate for the Smooth Streaming source (issue #4086): a VOD
//! presentation stops being fetched once the media already held runs the
//! configured cap ahead of the playback position, and fetching resumes at half
//! the cap. The default, [`ReadAhead::Unlimited`], makes the same requests it
//! made before the gate existed.

use crate::support::hls_host::Host;
use crate::support::read_ahead_pace::{Media, assert_gate_held, clock, drive, requests};
use broadcast_client::{Config, ReadAhead, SourceKind};
use broadcast_common::clock33::PCR_HZ;

const MANIFEST: &str = "http://origin.test/Manifest";
const VIDEO_STEM: &str = "http://origin.test/QualityLevels(64528)/Fragments(video=";
const AUDIO_STEM: &str = "http://origin.test/QualityLevels(98429)/Fragments(audio=";
/// Fragment start times (`{start time}` in the MS-SSTR URL) of the committed
/// presentation.
const VIDEO_STARTS: [u64; 3] = [0, 10_000_000, 20_000_000];
const AUDIO_STARTS: [u64; 3] = [0, 10_214_580, 20_199_161];

fn seconds(n: u64) -> u64 {
    n * PCR_HZ
}

fn config(read_ahead: ReadAhead) -> Config {
    let mut config = Config::default();
    config.kind = SourceKind::Smooth;
    config.ts_http = config.ts_http.with_read_ahead(read_ahead);
    config
}

fn fragment_url(stem: &str, start: u64) -> String {
    format!("{stem}{start})")
}

fn fragment_body(stem: &str, start: u64) -> Vec<u8> {
    let path = if stem == VIDEO_STEM {
        format!("smooth/av/QualityLevels(64528)/Fragments(video={start})")
    } else {
        format!("smooth/av/QualityLevels(98429)/Fragments(audio={start})")
    };
    test_fixtures::read(&path)
}

/// Every response body the origin serves, keyed by URL.
fn bodies() -> Vec<(String, Vec<u8>)> {
    let mut bodies = vec![(
        MANIFEST.to_string(),
        test_fixtures::read("smooth/av/Manifest"),
    )];
    for start in VIDEO_STARTS {
        bodies.push((
            fragment_url(VIDEO_STEM, start),
            fragment_body(VIDEO_STEM, start),
        ));
    }
    for start in AUDIO_STARTS {
        bodies.push((
            fragment_url(AUDIO_STEM, start),
            fragment_body(AUDIO_STEM, start),
        ));
    }
    bodies
}

fn host(read_ahead: ReadAhead) -> Host {
    let mut host = Host::new(MANIFEST, config(read_ahead));
    for (url, body) in bodies() {
        host.ok(&url, body);
    }
    host
}

/// Response bytes the origin fed for the requests in `requested`.
fn fed_bytes(requested: &[String]) -> u64 {
    let bodies = bodies();
    requested
        .iter()
        .filter_map(|url| bodies.iter().find(|(served, _)| served == url))
        .map(|(_, body)| u64::try_from(body.len()).unwrap_or(0))
        .sum()
}

/// The biggest fragment body of the committed presentation: the most one
/// fragment in flight can add.
fn largest_fragment() -> u64 {
    VIDEO_STARTS
        .iter()
        .map(|start| fragment_body(VIDEO_STEM, *start).len())
        .chain(
            AUDIO_STARTS
                .iter()
                .map(|start| fragment_body(AUDIO_STEM, *start).len()),
        )
        .max()
        .and_then(|len| u64::try_from(len).ok())
        .unwrap_or(0)
}

fn fragment_count(requested: &[String]) -> u64 {
    u64::try_from(
        requested
            .iter()
            .filter(|url| url.contains("Fragments("))
            .count(),
    )
    .unwrap_or(0)
}

#[test]
fn a_vod_presentation_stops_fetching_at_the_time_cap() {
    let cap = seconds(2);
    let mut host = host(ReadAhead::time(core::time::Duration::from_secs(2)));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    assert_gate_held(&log, cap);
    let fetched = fragment_count(&requests(&log));
    assert!(fetched >= 1, "no fragment was fetched");
    assert!(
        media.end_ticks() >= cap,
        "the presentation stopped before the cap: {} ticks delivered",
        media.end_ticks()
    );
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let anchor = media.end_ticks();
    let stalled = drive(&mut host, &mut media, 50, move |_| {
        Some(clock(anchor.saturating_sub(cap)))
    });
    assert!(
        requests(&stalled).is_empty(),
        "a lead still at the cap resumed fetching: {:?}",
        requests(&stalled)
    );

    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    assert_gate_held(&resumed, cap);
    let refetched = fragment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "a lead under half the cap did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed run never stopped again"
    );
}

#[test]
fn a_vod_presentation_stops_fetching_at_the_byte_cap() {
    let cap = 32 * 1024u64;
    let mut host = host(ReadAhead::bytes(cap));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    let fetched = fragment_count(&requests(&log));
    assert!(fetched >= 1, "no fragment was fetched");
    assert!(
        media.samples > 0,
        "the fetched fragments delivered no media"
    );
    let largest = largest_fragment();
    let requested = requests(&log);
    let fed = fed_bytes(&requested);
    assert!(fed >= cap, "the byte cap was never reached: {fed} bytes");
    assert!(
        fed <= cap + 2 * largest,
        "{fed} bytes were fetched, over the {cap}-byte cap plus the one fragment each stream had \
         in flight ({largest})"
    );
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let anchor = media.end_ticks();
    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    let refetched = fragment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "consuming every delivered byte did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed byte-capped run never stopped again"
    );
}

/// The request sequence the committed Smooth presentation produced before the
/// read-ahead gate existed for Smooth (pinned by
/// `smooth_pull::smooth_pull_matches_direct_fragment_demux`'s input).
const COMMITTED_REQUESTS: [&str; 7] = [
    MANIFEST,
    "http://origin.test/QualityLevels(64528)/Fragments(video=0)",
    "http://origin.test/QualityLevels(98429)/Fragments(audio=0)",
    "http://origin.test/QualityLevels(64528)/Fragments(video=10000000)",
    "http://origin.test/QualityLevels(98429)/Fragments(audio=10214580)",
    "http://origin.test/QualityLevels(64528)/Fragments(video=20000000)",
    "http://origin.test/QualityLevels(98429)/Fragments(audio=20199161)",
];

#[test]
fn an_unlimited_smooth_session_fetches_the_committed_presentation_as_before() {
    let mut host = host(ReadAhead::Unlimited);
    let mut media = Media::default();
    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    assert_eq!(requests(&log), COMMITTED_REQUESTS);
    assert!(media.samples > 0, "the presentation delivered no sample");
}
