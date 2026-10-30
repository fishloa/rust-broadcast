mod support;

use broadcast_client::{Config, Event};
use support::rtsp_host::{BASE_URL, Host, MediaFixture, Server};

const FRAME_STEP_US: u64 = 10_000;
const IDLE_STEPS: usize = 200;
const JOIN_OFFSETS: usize = 60;
const FRAMES_BEFORE_RECONNECT: [usize; 6] = [1, 2, 3, 4, 6, 9];

struct Played {
    samples: Vec<(u32, i64, u32)>,
}

impl Played {
    fn zero_duration(&self) -> Vec<&(u32, i64, u32)> {
        self.samples.iter().filter(|s| s.2 == 0).collect()
    }
}

fn collect(host: &mut Host, samples: &mut Vec<(u32, i64, u32)>) {
    while let Some(e) = host.session.next_event() {
        if let Event::Sample { track, sample, .. } = e {
            samples.push((
                track.value(),
                sample.pts.unwrap_or(0),
                sample.duration.unwrap_or(0),
            ));
        }
    }
}

fn play(skip_frames: usize, reconnect_after: Option<usize>) -> Played {
    let fixture = MediaFixture::load();
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture.sdp));
    host.connect();
    let mut samples = Vec::new();
    collect(&mut host, &mut samples);
    let frames: Vec<_> = fixture.frames().into_iter().skip(skip_frames).collect();
    for (i, frame) in frames.iter().enumerate() {
        if reconnect_after == Some(i) {
            host.session.reconnect(host.now);
            host.drain();
            collect(&mut host, &mut samples);
            break;
        }
        host.now += FRAME_STEP_US;
        host.deliver(frame);
        host.session.on_playback_position(host.now, None);
        host.drain();
        collect(&mut host, &mut samples);
    }
    if reconnect_after.is_none() {
        for _ in 0..IDLE_STEPS {
            host.now += FRAME_STEP_US;
            host.session.on_playback_position(host.now, None);
            host.drain();
            collect(&mut host, &mut samples);
        }
    }
    Played { samples }
}

#[test]
fn joining_a_reordered_stream_mid_gop_emits_no_zero_duration_sample() {
    for skip in 0..JOIN_OFFSETS {
        let played = play(skip, None);
        assert!(
            !played.samples.is_empty(),
            "join at frame {skip}: no samples"
        );
        let zero = played.zero_duration();
        assert!(
            zero.is_empty(),
            "join at frame {skip}: zero-duration {zero:?}"
        );
    }
}

#[test]
fn a_reconnect_early_in_the_stream_flushes_no_zero_duration_sample() {
    for after in FRAMES_BEFORE_RECONNECT {
        let played = play(0, Some(after));
        let zero = played.zero_duration();
        assert!(
            zero.is_empty(),
            "reconnect after {after} frames: zero-duration {zero:?}"
        );
    }
}
