mod support;

use broadcast_client::{BufferEvent, Config, Event};
use broadcast_common::Unpackage;
use support::rtsp_host::{BASE_URL, Host, MediaFixture, Server};
use transmux::{CodecConfig, TrackSpec, TsDemux};

const FRAME_STEP_US: u64 = 10_000;
const IDLE_STEPS: usize = 200;
const START_BOUND_US: u64 = 5_000_000;
const SPROP_ATTRIBUTE: &str = "sprop-parameter-sets=";

struct Run {
    events: Vec<(u64, Event)>,
}

impl Run {
    fn tracks(&self) -> Vec<&Vec<TrackSpec>> {
        self.events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Tracks(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn samples(&self) -> Vec<&transmux::Sample> {
        self.events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Sample { sample, .. } => Some(sample),
                _ => None,
            })
            .collect()
    }

    fn start_time(&self) -> Option<u64> {
        self.events.iter().find_map(|(t, e)| match e {
            Event::Buffer(BufferEvent::Start { .. }) => Some(*t),
            _ => None,
        })
    }
}

fn avc_dims(specs: &[TrackSpec]) -> Option<(u16, u16)> {
    specs.iter().find_map(|s| match &s.config {
        CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
        _ => None,
    })
}

fn ts_dims() -> (u16, u16) {
    let media = TsDemux::new()
        .unpackage(&test_fixtures::read("ts/h264_aac.ts"))
        .expect("demux");
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    avc_dims(&specs).expect("ts video spec")
}

const SPS_HEADER_LEN: usize = 4;

fn truncate_sprop_sps(sdp: &str) -> String {
    let at = sdp.find(SPROP_ATTRIBUTE).expect("sprop") + SPROP_ATTRIBUTE.len();
    let end = at + sdp[at..].find(['\r', '\n', ';']).expect("sprop end");
    let (sps_b64, rest) = sdp[at..end].split_once(',').expect("sps,pps");
    let sps = transmux::rtp::base64_decode(sps_b64).expect("sps b64");
    let truncated = transmux::rtp::base64_encode(&sps[..SPS_HEADER_LEN]);
    format!("{}{truncated},{rest}{}", &sdp[..at], &sdp[end..])
}

fn play(sdp: &str, frames: &[Vec<u8>]) -> Run {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(sdp));
    host.connect();
    let mut events = Vec::new();
    let mut collect = |host: &mut Host| {
        let now = host.now;
        events.extend(host.events().into_iter().map(|e| (now, e)));
    };
    collect(&mut host);
    let start = host.now;
    for frame in frames {
        host.now += FRAME_STEP_US;
        let conn = host.conn.expect("conn");
        host.session.on_data(conn, frame, host.now);
        host.session.on_playback_position(host.now, None);
        host.drain();
        collect(&mut host);
    }
    for _ in 0..IDLE_STEPS {
        host.now += FRAME_STEP_US;
        host.session.on_playback_position(host.now, None);
        host.drain();
        collect(&mut host);
    }
    for (t, _) in &mut events {
        *t -= start;
    }
    Run { events }
}

#[test]
fn rtsp_dims_come_from_sprop_parameter_sets() {
    let fixture = MediaFixture::load();
    assert!(
        fixture.sdp.contains(SPROP_ATTRIBUTE),
        "fixture SDP lacks sprop"
    );
    let run = play(&fixture.sdp, &fixture.frames());
    let tracks = run.tracks();
    let first = tracks.first().expect("Tracks announced");
    let dims = avc_dims(first).expect("video track in first announcement");
    assert_eq!(dims, ts_dims());
    assert!(dims.0 > 0 && dims.1 > 0);
}

#[test]
fn rtsp_dims_come_from_first_inband_sps_when_sprop_sps_is_undecodable() {
    let fixture = MediaFixture::load();
    let sdp = truncate_sprop_sps(&fixture.sdp);
    let run = play(&sdp, &fixture.frames());
    let with_video: Vec<_> = run
        .tracks()
        .into_iter()
        .filter(|t| avc_dims(t).is_some())
        .collect();
    assert_eq!(
        with_video.len(),
        2,
        "Tracks must be announced once with unknown dims and exactly once more with the in-band dims"
    );
    assert_eq!(avc_dims(with_video[0]), Some((0, 0)));
    assert_eq!(avc_dims(with_video[1]), Some(ts_dims()));
}

#[test]
fn rtsp_emits_buffer_start_with_default_config() {
    let fixture = MediaFixture::load();
    let run = play(&fixture.sdp, &fixture.frames());
    let at = run.start_time().expect("no BufferEvent::Start");
    assert!(
        at <= START_BOUND_US,
        "Start came {at} us after PLAY, bound {START_BOUND_US}"
    );
}

#[test]
fn rtsp_samples_carry_positive_durations_once_a_frame_period_is_known() {
    let fixture = MediaFixture::load();
    let run = play(&fixture.sdp, &fixture.frames());
    let samples = run.samples();
    assert!(!samples.is_empty(), "no samples");
    let zero = samples
        .iter()
        .filter(|s| s.duration.unwrap_or(0) == 0)
        .count();
    assert_eq!(
        zero,
        0,
        "{zero} of {} samples have zero duration",
        samples.len()
    );
}
