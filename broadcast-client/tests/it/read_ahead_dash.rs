//! Read-ahead gate for the DASH source (issue #4086): a static presentation
//! stops being fetched once the media already held runs the configured cap
//! ahead of the playback position, and fetching resumes at half the cap. The
//! default, [`ReadAhead::Unlimited`], makes the same requests it made before the
//! gate existed.
//!
//! The capped tests serve a one-representation presentation built over the
//! committed `dash/segmentlist` segment files, so exactly one media request is
//! in flight when the gate closes.

use std::collections::VecDeque;

use crate::support::dash_common;
use crate::support::dash_common::synth::{Addressing, MPD_URL, Synth};
use crate::support::dash_common::world::{MICROS, World};
use crate::support::dash_sim::{DashSim, SEC, WorldBackend};
use crate::support::read_ahead_pace::{Media, clock};
use broadcast_client::{
    Action, Config, ConnId, Event, ReadAhead, Session, SourceKind, TransportError,
};
use broadcast_common::clock33::{Clock27, PCR_HZ};
use broadcast_common::stage::Timestamp;
use dash_common::loopback::{Origin, Req};
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

/// The committed two-representation static presentation, for the request
/// sequence the default configuration must keep producing.
const COMMITTED_MPD: &str = "http://dash.test/segmentlist/list.mpd";
/// A one-representation MPD over the same directory's segment files.
const ONE_REP_MPD: &str = "http://dash.test/segmentlist/read-ahead.mpd";
const CHUNK: usize = 65_536;
const AST_US: i64 = 1_700_000_000 * MICROS;
const SEGMENTS: u64 = 6;
const SEGMENT_SECS: u64 = 2;

fn seconds(n: u64) -> u64 {
    n * PCR_HZ
}

fn config(read_ahead: ReadAhead) -> Config {
    let mut config = Config::default();
    config.kind = SourceKind::Dash;
    config.ts_http = config.ts_http.with_read_ahead(read_ahead);
    config
}

/// One video AdaptationSet over `dash/segmentlist`'s own segment files.
fn one_representation_mpd() -> Vec<u8> {
    let mut segments = String::new();
    for n in 1..=SEGMENTS {
        segments.push_str(&format!("<SegmentURL media=\"seg-0-{n:03}.m4s\" />\n"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" \
         profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" type=\"static\" \
         mediaPresentationDuration=\"PT{}S\" maxSegmentDuration=\"PT{SEGMENT_SECS}S\" \
         minBufferTime=\"PT4.0S\">\n\
         <Period id=\"0\" start=\"PT0.0S\">\n\
         <AdaptationSet id=\"0\" contentType=\"video\" segmentAlignment=\"true\" \
         bitstreamSwitching=\"true\" frameRate=\"49/2\" maxWidth=\"320\" maxHeight=\"240\" \
         par=\"4:3\" lang=\"und\">\n\
         <Representation id=\"0\" mimeType=\"video/mp4\" codecs=\"avc1.4d400d\" \
         bandwidth=\"99692\" width=\"320\" height=\"240\" sar=\"1:1\">\n\
         <SegmentList timescale=\"1000000\" duration=\"{}\" startNumber=\"1\">\n\
         <Initialization sourceURL=\"init-0.m4s\" />\n\
         {segments}\
         </SegmentList>\n\
         </Representation>\n\
         </AdaptationSet>\n\
         </Period>\n\
         </MPD>\n",
        SEGMENTS * SEGMENT_SECS,
        SEGMENT_SECS * 1_000_000
    )
    .into_bytes()
}

/// One request the session made: its connection, URL and byte range.
type Fetched = (ConnId, String, Option<(u64, u64)>);

/// A DASH session driven by hand, so a test reports the playback position the
/// conformance harness never does.
struct Static {
    session: Session,
    origin: Origin,
    inflight: VecDeque<Fetched>,
    pauses: usize,
    /// Response body bytes fed to the session.
    fed: u64,
    /// Largest single segment body fed: one media request in flight adds at most
    /// this.
    largest_segment: u64,
    /// Largest single control body fed (the MPD or an initialization section).
    largest_control: u64,
}

impl Static {
    fn open(url: &str, read_ahead: ReadAhead, origin: Origin) -> Self {
        Self {
            session: Session::open(
                Url::parse(url).expect("a test names a URL"),
                config(read_ahead),
                ts(0),
            )
            .expect("open a DASH session"),
            origin,
            inflight: VecDeque::new(),
            pauses: 0,
            fed: 0,
            largest_segment: 0,
            largest_control: 0,
        }
    }

    fn committed(read_ahead: ReadAhead) -> Self {
        Self::open(COMMITTED_MPD, read_ahead, Origin::new())
    }

    fn one_representation(read_ahead: ReadAhead) -> Self {
        let origin = Origin::new().doc(ONE_REP_MPD, one_representation_mpd());
        Self::open(ONE_REP_MPD, read_ahead, origin)
    }

    fn collect(&mut self) {
        while let Some(action) = self.session.poll(ts(0)) {
            match action {
                Action::HttpRequest {
                    conn,
                    url,
                    byte_range,
                    ..
                } => self.inflight.push_back((
                    conn,
                    url.to_string(),
                    byte_range.map(|span| (span.offset, span.len.get())),
                )),
                Action::PauseRead { .. } | Action::ResumeRead { .. } => {
                    self.pauses = self.pauses.saturating_add(1);
                }
                _ => {}
            }
        }
    }

    fn respond(&mut self, conn: ConnId, url: &str, range: Option<(u64, u64)>) {
        let request = Req {
            url: url.to_string(),
            range,
        };
        let response = match self.origin.serve(&request) {
            Ok(response) => response,
            Err(_) => {
                self.session.on_closed(
                    conn,
                    Some(TransportError::Other {
                        reason: "transport".to_string(),
                    }),
                    ts(0),
                );
                return;
            }
        };
        let mut headers = http::HeaderMap::new();
        if response.status == 206
            && let Some((offset, len)) = range
        {
            headers.insert(
                http::header::CONTENT_RANGE,
                http::HeaderValue::from_str(&format!(
                    "bytes {offset}-{}/*",
                    offset + len.saturating_sub(1)
                ))
                .expect("a test writes a valid header value"),
            );
        }
        self.session.on_response(
            conn,
            crate::support::status(response.status),
            &headers,
            ts(0),
        );
        let fed = u64::try_from(response.body.len()).unwrap_or(0);
        self.fed = self.fed.saturating_add(fed);
        if url.contains("/seg-") {
            self.largest_segment = self.largest_segment.max(fed);
        } else {
            self.largest_control = self.largest_control.max(fed);
        }
        for chunk in response.body.chunks(CHUNK) {
            self.session.on_data(conn, chunk, ts(0));
        }
        self.session.on_closed(conn, None, ts(0));
    }
}

/// One paced step: the request it answered, and the lead the session's measure
/// had when the step began.
struct Step {
    requests: Vec<String>,
    lead: Option<u64>,
}

fn drain_events(host: &mut Static, media: &mut Media) {
    while let Some(event) = host.session.next_event() {
        media.observe(&event);
    }
}

fn drive(
    host: &mut Static,
    media: &mut Media,
    steps: usize,
    mut pace: impl FnMut(&Media) -> Option<Clock27>,
) -> Vec<Step> {
    let mut log = Vec::new();
    for _ in 0..steps {
        host.collect();
        let position = pace(media);
        let lead = position.map(|position| media.end_ticks().saturating_sub(position.ticks()));
        host.session.on_playback_position(ts(0), position);
        host.collect();
        drain_events(host, media);
        match host.inflight.pop_front() {
            Some((conn, url, range)) => {
                host.respond(conn, &url, range);
                host.collect();
                drain_events(host, media);
                log.push(Step {
                    requests: vec![url],
                    lead,
                });
            }
            None => {
                log.push(Step {
                    requests: Vec::new(),
                    lead,
                });
                break;
            }
        }
    }
    log
}

fn requests(log: &[Step]) -> Vec<String> {
    log.iter()
        .flat_map(|step| step.requests.iter().cloned())
        .collect()
}

fn segment_count(requested: &[String]) -> u64 {
    u64::try_from(requested.iter().filter(|url| url.contains("/seg-")).count()).unwrap_or(0)
}

/// The gate allowance: media held may reach the cap plus the one segment already
/// in flight, so at most one request may appear at a step whose entry lead is at
/// or over the cap.
fn assert_gate_held(log: &[Step], cap: u64) {
    let over = log
        .iter()
        .filter(|step| !step.requests.is_empty() && step.lead.is_some_and(|lead| lead >= cap))
        .count();
    assert!(
        over <= 1,
        "{over} requests were issued with the lead at or over the {cap}-tick cap"
    );
}

#[test]
fn a_static_presentation_stops_fetching_at_the_time_cap() {
    let cap = seconds(5);
    let mut host = Static::one_representation(ReadAhead::time(core::time::Duration::from_secs(5)));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    assert_gate_held(&log, cap);
    let fetched = segment_count(&requests(&log));
    assert!(fetched >= 1, "no segment was fetched");
    assert!(
        media.end_ticks() >= cap,
        "the presentation stopped before the cap: {} ticks delivered",
        media.end_ticks()
    );
    assert_eq!(host.pauses, 0, "a segment-pull source pauses no connection");
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let anchor = media.end_ticks();
    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    assert_gate_held(&resumed, cap);
    let refetched = segment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "a lead under half the cap did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed run never stopped again"
    );
    assert!(
        fetched >= 1 && refetched >= 1,
        "the gate closed once without a resumed fetch"
    );
}

#[test]
fn a_static_presentation_stops_fetching_at_the_byte_cap() {
    let cap = 64 * 1024u64;
    let mut host = Static::one_representation(ReadAhead::bytes(cap));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    let fetched = segment_count(&requests(&log));
    assert!(fetched >= 1, "no segment was fetched");
    assert!(media.samples > 0, "the fetched segments delivered no media");
    assert!(
        host.fed >= cap,
        "the byte cap was never reached: {} bytes",
        host.fed
    );
    let allowance = cap + host.largest_segment + host.largest_control;
    assert!(
        host.fed <= allowance,
        "{} bytes were fetched, over the {cap}-byte cap plus the one segment ({}) and control \
         section ({}) in flight",
        host.fed,
        host.largest_segment,
        host.largest_control
    );
    assert_eq!(host.pauses, 0, "a segment-pull source pauses no connection");
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let anchor = media.end_ticks();
    let before = host.fed;
    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    let refetched = segment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "consuming every delivered byte did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed byte-capped run never stopped again"
    );
    assert!(
        host.fed - before <= allowance,
        "{} bytes were re-fetched, over the {cap}-byte cap plus the responses in flight",
        host.fed - before
    );
}

/// The request sequence the committed static presentation produced before the
/// read-ahead gate existed for DASH (its inputs are pinned by `dash_static`'s
/// `segment_list_equals_the_demux_oracle`).
const COMMITTED_REQUESTS: [&str; 15] = [
    COMMITTED_MPD,
    "http://dash.test/segmentlist/init-0.m4s",
    "http://dash.test/segmentlist/init-1.m4s",
    "http://dash.test/segmentlist/seg-0-001.m4s",
    "http://dash.test/segmentlist/seg-1-001.m4s",
    "http://dash.test/segmentlist/seg-0-002.m4s",
    "http://dash.test/segmentlist/seg-1-002.m4s",
    "http://dash.test/segmentlist/seg-0-003.m4s",
    "http://dash.test/segmentlist/seg-1-003.m4s",
    "http://dash.test/segmentlist/seg-0-004.m4s",
    "http://dash.test/segmentlist/seg-1-004.m4s",
    "http://dash.test/segmentlist/seg-0-005.m4s",
    "http://dash.test/segmentlist/seg-1-005.m4s",
    "http://dash.test/segmentlist/seg-0-006.m4s",
    "http://dash.test/segmentlist/seg-1-006.m4s",
];

#[test]
fn an_unlimited_dash_session_fetches_the_committed_presentation_as_before() {
    let mut host = Static::committed(ReadAhead::Unlimited);
    let mut media = Media::default();
    let log = drive(&mut host, &mut media, 400, |media| {
        Some(clock(media.end_ticks()))
    });
    assert_eq!(requests(&log), COMMITTED_REQUESTS);
}

/// A live presentation driven by the true-clock harness, under two read-ahead
/// settings.
fn live_requests(read_ahead: ReadAhead) -> (Vec<String>, usize) {
    let synth = Synth::new(AST_US, Addressing::Number);
    let world = World::new(AST_US);
    let mut sim = DashSim::new(
        MPD_URL,
        config(read_ahead),
        world,
        Box::new(WorldBackend(synth)),
    );
    sim.give_host_clock();
    sim.run_for(20 * SEC);
    assert!(sim.fatal().is_none(), "fatal: {:?}", sim.fatal());
    (
        sim.log.iter().map(|logged| logged.url.clone()).collect(),
        sim.count(|event| matches!(event, Event::Sample { .. })),
    )
}

#[test]
fn a_live_presentation_with_a_cap_over_its_window_makes_the_same_requests() {
    let (unlimited, unlimited_samples) = live_requests(ReadAhead::Unlimited);
    let (capped, capped_samples) =
        live_requests(ReadAhead::time(core::time::Duration::from_secs(30)));
    assert!(
        unlimited.iter().any(|url| url.contains(".m4s")),
        "the live run fetched no segment"
    );
    assert!(unlimited_samples > 0, "the live run delivered no sample");
    assert_eq!(capped, unlimited);
    assert_eq!(capped_samples, unlimited_samples);
}
