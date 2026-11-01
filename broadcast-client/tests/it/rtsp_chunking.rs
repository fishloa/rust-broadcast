use crate::support;

use broadcast_client::{Config, Event};
use support::rtsp_host::{BASE_URL, Chunking, Host, MediaFixture, Server};

const FRAME_STEP_US: u64 = 10_000;
const IDLE_STEPS: usize = 200;

fn play_with(
    sdp: &str,
    chunking: Chunking,
    poll_every_frame: bool,
    idle_poll_every: bool,
) -> Vec<(u32, i64, i64, u32)> {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(sdp));
    host.chunking = chunking;
    host.connect();
    let mut events = Vec::new();
    let mut collect = |host: &mut Host| {
        while let Some(e) = host.session.next_event() {
            events.push(e);
        }
    };
    collect(&mut host);
    let fixture = MediaFixture::load();
    let frames = fixture.frames();
    for frame in &frames {
        host.now += FRAME_STEP_US;
        host.deliver(frame);
        if poll_every_frame {
            host.session.on_playback_position(host.now, None);
        }
        host.drain();
        collect(&mut host);
    }
    for _ in 0..IDLE_STEPS {
        host.now += FRAME_STEP_US;
        if idle_poll_every {
            host.session.on_playback_position(host.now, None);
        }
        host.drain();
        collect(&mut host);
    }
    host.drain();
    collect(&mut host);

    let mut out = Vec::new();
    for e in &events {
        if let Event::Sample { track, sample, .. } = e {
            out.push((
                track.value(),
                sample.pts.unwrap_or(0),
                sample.dts.unwrap_or(0),
                sample.duration.unwrap_or(0),
            ));
        }
    }
    out
}

fn rnd_sizes(seed: u64, n: usize) -> Vec<usize> {
    let mut x = seed.wrapping_add(0x9E3779B97F4A7C15);
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        let sz = ((x >> 33) as usize % 17) + 1;
        v.push(sz);
    }
    v
}

#[test]
fn chunking_invariance() {
    let fixture = MediaFixture::load();
    let sdp = fixture.sdp.clone();

    let baseline = play_with(&sdp, Chunking::Whole, true, true);

    // 1-byte chunks
    let one_byte = play_with(&sdp, Chunking::Cycle(vec![1]), true, true);
    assert_eq!(baseline, one_byte, "1-byte chunking differs from whole");

    // random chunk sizes from several fixed seeds
    for seed in [1u64, 2, 3, 42, 99, 7, 12345] {
        let sizes = rnd_sizes(seed, 8);
        let run = play_with(&sdp, Chunking::Cycle(sizes.clone()), true, true);
        assert_eq!(baseline, run, "seed {seed} sizes {sizes:?} differs");
    }

    // poll/on_playback_position frequency variations
    let no_per_frame_poll = play_with(&sdp, Chunking::Whole, false, true);
    assert_eq!(baseline, no_per_frame_poll, "no per-frame poll differs");
    let no_idle_poll = play_with(&sdp, Chunking::Whole, true, false);
    assert_eq!(baseline, no_idle_poll, "no idle poll differs");

    // no sample may have zero duration
    let zero = baseline.iter().filter(|t| t.3 == 0).count();
    assert_eq!(
        zero,
        0,
        "{zero} of {} samples have zero duration",
        baseline.len()
    );
    assert!(!baseline.is_empty());
}
