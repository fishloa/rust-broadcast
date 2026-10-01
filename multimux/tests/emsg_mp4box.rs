//! Independent oracles for the DASH inband SCTE-35 `emsg` injection (#969,
//! audit run 7 W9/E): the boxes `crate::origin::resource` splices into a
//! served fMP4 segment must be readable by **`mp4dump`** (Bento4) *and* the
//! `message_data` of each box must decode, with the independent
//! **`scte35-splice`** parser, back to the exact SCTE-35 section published.
//!
//! Drives the REAL production segmenting path — `ProgramSegmenter::pump`
//! (where `note_segment_start` records each segment's source-clock start) —
//! over real samples with a **non-zero PTS origin**, then serves the segment
//! over a real loopback HTTP socket and inspects the returned bytes.
//!
//! Skips cleanly when `mp4dump` is not on `PATH`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Parse;
use media_plane::ProgramId;
use media_plane::trunk::{EventAnchor, RetentionClass, Trunk, TrunkConfig};
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::segment::ProgramSegmenter;
use scte35_splice::SpliceInfoSection;
use timed_metadata::MediaTime;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

const TARGET_DURATION_SECS: f64 = 1.0;
const TRACK_ID: u32 = 1;
const TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = TIMESCALE / 30;
const SYNC_INTERVAL: u32 = 15;

/// A real SCTE-35 `splice_insert` (event_id 2002) — the fixture
/// `timed-metadata`'s own tests use.
const SPLICE_INSERT_HEX: &str =
    "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

fn mp4dump_available() -> bool {
    std::process::Command::new("mp4dump")
        .arg("--version")
        .output()
        .is_ok()
}

/// Require an external oracle tool. A missing tool is a HARD FAILURE unless
/// `MULTIMUX_ALLOW_ORACLE_SKIP=1` is set in the environment — a silent skip
/// would let this oracle test pass without ever running the oracle (issue
/// #1083, item 5). The escape hatch exists only for environments that cannot
/// install the tool.
fn assert_oracle(tool: &str) {
    let present = match tool {
        "mp4dump" => mp4dump_available(),
        other => panic!("no availability probe for oracle {other}"),
    };
    if present {
        return;
    }
    if std::env::var("MULTIMUX_ALLOW_ORACLE_SKIP").as_deref() == Ok("1") {
        eprintln!(
            "SKIP: {tool} not on PATH (MULTIMUX_ALLOW_ORACLE_SKIP=1). This oracle did NOT run."
        );
        return;
    }
    panic!(
        "required oracle tool `{tool}` is not on PATH. Install it, or set          MULTIMUX_ALLOW_ORACLE_SKIP=1 to skip this oracle explicitly."
    );
}

fn track_spec() -> TrackSpec {
    let sprop = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    let config = transmux::avc_config_from_sprop(sprop).expect("valid sprop");
    TrackSpec::new(
        TRACK_ID,
        TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 64,
            height: 64,
        },
    )
}

fn sample_at(pts: i64, is_sync: bool) -> Sample {
    let nal = [0x65u8, 0xAA, (pts % 256) as u8];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    Sample::new(data, Some(pts), Some(pts), Some(FRAME_DUR), is_sync)
}

/// Build a real trunk filled with `frames` samples starting at `pts_origin`,
/// driven through the production `ProgramSegmenter::pump`.
fn build_real_trunk(pts_origin: i64, frames: u32) -> (Arc<RouteHandle>, Arc<Trunk>) {
    let trunk = Trunk::new(TrunkConfig::new(
        std::num::NonZeroUsize::new(4096).unwrap(),
        std::num::NonZeroUsize::new(64).unwrap(),
        std::num::NonZeroUsize::new(64).unwrap(),
        std::num::NonZeroUsize::new(64).unwrap(),
        std::num::NonZeroUsize::new(64).unwrap(),
    ));
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(vec![track_spec()]);

    let route = RouteHandle::new(TARGET_DURATION_SECS, 200, 8);
    // Publish the program into a route's serving state FIRST, so its
    // `HlsOrigin` subscribes from the backlog and observes every segment the
    // segmenter below produces — otherwise its live-edge cursor starts after
    // the segments already written and serves them via the in-progress
    // (chunked) path instead of the resource path that injects emsg.
    route.publish_program(ProgramId(0), Arc::clone(&trunk));

    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, 200).expect("segmenter");

    for i in 0..frames {
        let pts = pts_origin + i64::from(i) * i64::from(FRAME_DUR);
        writer.publish(
            TRACK_ID,
            RetentionClass::Timed,
            sample_at(pts, i % SYNC_INTERVAL == 0),
        );
    }
    segmenter.pump(ProgramId(0), &trunk, &route);
    segmenter.flush();
    (Arc::new(route), trunk)
}

/// Serve `trunk`'s `seg-{TRACK_ID}-{seq}.m4s` over a real loopback socket.
async fn fetch_segment(route: Arc<RouteHandle>, seq: u32) -> Vec<u8> {
    let mut streams = HashMap::new();
    streams.insert(
        "live".to_string(),
        (
            route,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/live/seg-{TRACK_ID}-{seq}.m4s"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .expect("fetch segment");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "segment must serve");
    let bytes = resp.bytes().await.expect("bytes").to_vec();
    server.abort();
    bytes
}

fn parse_emsg_boxes(bytes: &[u8]) -> Vec<transmux::EmsgBox<'_>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 8 <= bytes.len() {
        let size =
            u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        if size < 8 || i + size > bytes.len() {
            break;
        }
        if &bytes[i + 4..i + 8] == b"emsg"
            && let Ok(e) = transmux::EmsgBox::parse(&bytes[i..i + size])
        {
            out.push(e);
        }
        i += size;
    }
    out
}

#[tokio::test]
async fn served_segment_carries_a_decodable_emsg_with_a_nonzero_pts_origin() {
    assert_oracle("mp4dump");
    // A deliberately NON-ZERO PTS origin: the segment start must be anchored
    // to the real source clock, not to a 0-based internal timeline.
    const PTS_ORIGIN: i64 = 5 * 90_000; // 5 s in

    let (route, trunk) = build_real_trunk(PTS_ORIGIN, 90);

    let writer = trunk.writer().expect("trunk writer");
    let mut timeline = timed_metadata::Timeline::new();
    let ev = timeline
        .push_scte35(&hex_bytes(SPLICE_INSERT_HEX))
        .expect("parse splice");
    let declared_id = ev.id.expect("a splice_insert has an event id");
    writer.publish_event(ev, EventAnchor::Media(MediaTime(PTS_ORIGIN as u64)));

    let last = trunk.last_closed_segment().expect("a segment closed");
    let attributed: Vec<u32> = (1..=last)
        .filter(|seq| !trunk.events_in_segment(*seq).is_empty())
        .collect();
    assert_eq!(
        attributed,
        vec![1],
        "the event at the PTS origin must attribute to segment 1 only, got {attributed:?}"
    );

    let bytes = fetch_segment(Arc::clone(&route), 1).await;

    // mp4dump (independent) must see the box, between styp and moof.
    let dir = std::env::temp_dir().join(format!("multimux-emsg-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("seg.m4s");
    std::fs::write(&file, &bytes).expect("write");
    let dump = std::process::Command::new("mp4dump")
        .args(["--verbosity", "2"])
        .arg(file.to_str().expect("path"))
        .output()
        .expect("mp4dump");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&dump.stdout).into_owned();
    assert!(
        text.contains("[emsg]"),
        "mp4dump must see the emsg box: {text}"
    );
    let styp = text.find("[styp]").expect("styp");
    let emsg = text.find("[emsg]").expect("emsg");
    let moof = text.find("[moof]").expect("moof");
    assert!(
        styp < emsg && emsg < moof,
        "emsg must sit between styp and moof: {text}"
    );

    // `message_data` must decode (with the INDEPENDENT `scte35-splice`
    // parser) back to the exact source section.
    let boxes = parse_emsg_boxes(&bytes);
    assert_eq!(boxes.len(), 1, "exactly one emsg box");
    let box0 = &boxes[0];
    assert_eq!(box0.scheme_id_uri, "urn:scte:scte35:2013:bin");
    assert_eq!(
        box0.id, declared_id,
        "the declared splice_event_id is kept verbatim"
    );
    assert_eq!(
        box0.message_data,
        hex_bytes(SPLICE_INSERT_HEX).as_slice(),
        "message_data must be the source section, byte-for-byte"
    );
    let reparsed = SpliceInfoSection::parse(box0.message_data).expect("independent parse");
    assert_eq!(
        reparsed,
        SpliceInfoSection::parse(&hex_bytes(SPLICE_INSERT_HEX)).expect("source parse"),
        "the message_data must decode to the source section"
    );

    // `presentation_time` must be EXACTLY the media time the event was
    // anchored at — the trunk's 90 kHz clock, which equals the trunk's own
    // `timescale` (so no rescale). A value merely "at/after the origin" would
    // pass even if the box carried the segment start rather than the event's
    // own instant.
    let pt = match box0.presentation_time {
        transmux::PresentationTime::Absolute(t) => t,
        other => panic!("expected an absolute presentation time, got {other:?}"),
    };
    assert_eq!(
        pt, PTS_ORIGIN as u64,
        "presentation_time must be exactly the anchored media time"
    );
    assert_eq!(
        box0.timescale, 90_000,
        "the emsg timescale must be the trunk's 90 kHz clock"
    );
}
