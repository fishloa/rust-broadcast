mod support;

use broadcast_client::{BufferEvent, Config, Event};
use broadcast_common::{Serialize, Unpackage};
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

fn strip_sprop(sdp: &str) -> String {
    let attr = "sprop-parameter-sets=";
    let start = sdp.find(attr).expect("sprop");
    let end = sdp[start..]
        .find(['\r', '\n', ';'])
        .map(|k| start + k)
        .unwrap_or(sdp.len());
    let mut out = String::new();
    out.push_str(&sdp[..start]);
    out.push_str(&sdp[end..]);
    out
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

#[test]
fn rtsp_no_sprop_announces_video_with_inband_avcc() {
    let fixture = MediaFixture::load();
    assert!(
        fixture.sdp.contains(SPROP_ATTRIBUTE),
        "fixture SDP lacks sprop"
    );
    let sdp = strip_sprop(&fixture.sdp);
    assert!(!sdp.contains(SPROP_ATTRIBUTE), "sprop not stripped");
    let run = play(&sdp, &fixture.frames());
    let ref_run = play(&fixture.sdp, &fixture.frames());

    let mut video_cfg = None;
    for specs in run.tracks() {
        if let Some((cfg, w, h)) = specs.iter().find_map(|s| match &s.config {
            CodecConfig::Avc {
                config,
                width,
                height,
            } if !config.config.sps.is_empty() => Some((config.clone(), *width, *height)),
            _ => None,
        }) {
            video_cfg = Some((cfg, w, h));
            break;
        }
    }
    let (cfg, w, h) = video_cfg.expect("no ready video track announced");
    assert!(w > 0 && h > 0, "dims zero");
    // parse it back with transmux's avcC parser
    let mut buf = vec![0u8; cfg.serialized_len()];
    cfg.serialize_into(&mut buf).expect("serialize avcC");
    let parsed = transmux::AVCConfigurationBox::parse_body(&buf[8..]).expect("parse avcC");
    assert_eq!(parsed, cfg);

    let mut ref_cfg = None;
    for specs in ref_run.tracks() {
        if let Some((cfg, w, h)) = specs.iter().find_map(|s| match &s.config {
            CodecConfig::Avc {
                config,
                width,
                height,
            } if !config.config.sps.is_empty() => Some((config.clone(), *width, *height)),
            _ => None,
        }) {
            ref_cfg = Some((cfg, w, h));
            break;
        }
    }
    let (ref_c, ref_w, ref_h) = ref_cfg.expect("ref video track");
    assert_eq!(cfg, ref_c);
    assert_eq!((w, h), (ref_w, ref_h));

    assert!(!run.samples().is_empty(), "no samples flow");
    assert!(run.start_time().is_some(), "no BufferEvent::Start");
}

#[test]
fn rtsp_no_sprop_emits_no_video_sample_before_config_known() {
    let fixture = MediaFixture::load();
    let sdp = strip_sprop(&fixture.sdp);
    let run = play(&sdp, &fixture.frames());

    // find first Tracks announcement carrying a ready (non-empty) video avcC
    let mut video_track = None;
    let mut first_ready_idx = None;
    for (idx, (_, e)) in run.events.iter().enumerate() {
        if let Event::Tracks(specs) = e
            && video_track.is_none()
            && let Some(id) = specs.iter().find_map(|s| match &s.config {
                CodecConfig::Avc { config, .. } if !config.config.sps.is_empty() => {
                    Some(s.track_id)
                }
                _ => None,
            })
        {
            video_track = Some(id);
            first_ready_idx = Some(idx);
        }
    }
    let vt = video_track.expect("no ready video track announced");
    let idx = first_ready_idx.unwrap();
    let before = run.events[..idx]
        .iter()
        .filter(|(_, e)| matches!(e, Event::Sample { track, .. } if *track == vt))
        .count();
    assert_eq!(before, 0, "video samples emitted before config known");
}
