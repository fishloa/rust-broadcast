//! Round-7 finding F4: a PMT that changes what an existing PID carries replaces the kind the
//! start gate expects for it.

mod support;
use broadcast_client::events::DEFAULT_START_PENDING_TRACK_WAIT_US;
use broadcast_client::{BufferEvent, Config, Event};
use support::ts_host::Host;
use support::ts_pmt::{Es, find, rewrite};

const FIXTURE: &str = "ts/h264_aac_40s.ts";
const CHUNK: usize = 4 * TS_PACKET;
const TS_PACKET: usize = 188;
const PES_PRIVATE_DATA: u8 = 0x06;
const AAC_ADTS: u8 = 0x0f;
const AVC: u8 = 0x1b;
const AC3_DESCRIPTOR: [u8; 2] = [0x6a, 0x00];
const RELEASED_BOUND_US: u64 = 1_000_000;
const FLIP_AT: f64 = 0.004;

fn pid_of(p: &[u8]) -> u16 {
    (u16::from(p[1]) << 8 | u16::from(p[2])) & 0x1fff
}

fn run(bytes: &[u8]) -> Option<u64> {
    let mut host = Host::open(Config::default());
    let mut at = None;
    for part in bytes[..bytes.len() / TS_PACKET * TS_PACKET].chunks(CHUNK) {
        host.feed_playing(part, CHUNK);
        if at.is_none()
            && host
                .log
                .iter()
                .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
        {
            at = Some(host.now);
        }
    }
    for _ in 0..200 {
        host.now += 50_000;
        let pos = host.position();
        host.tick(pos);
        if at.is_none()
            && host
                .log
                .iter()
                .any(|e| matches!(e, Event::Buffer(BufferEvent::Start { .. })))
        {
            at = Some(host.now);
        }
    }
    at
}

fn audioless_with(plan: impl Fn(&[Es]) -> Option<Vec<Es>>) -> Vec<u8> {
    audioless_with_flip(FLIP_AT, plan)
}

fn audioless_with_flip(flip: f64, plan: impl Fn(&[Es]) -> Option<Vec<Es>>) -> Vec<u8> {
    let bytes = test_fixtures::read(FIXTURE);
    let pmt = find(&bytes).expect("a PMT");
    let audio = pmt.es[1].pid;
    let kept: Vec<u8> = bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != audio)
        .flatten()
        .copied()
        .collect();
    let v = pmt.version;
    let es = pmt.es.clone();
    rewrite(&kept, &pmt, &|at| {
        if at < flip {
            return None;
        }
        plan(&es).map(|e| (e, v.wrapping_add(1)))
    })
}

fn flip_audio_to(stream_type: u8, descriptors: Vec<u8>) -> Vec<u8> {
    audioless_with(|es| {
        let mut e = es.to_vec();
        e[1].stream_type = stream_type;
        e[1].descriptors = descriptors.clone();
        Some(e)
    })
}

fn waits(start: Option<u64>) -> bool {
    start.is_some_and(|t| t >= DEFAULT_START_PENDING_TRACK_WAIT_US)
}

fn released(start: Option<u64>) -> bool {
    start.is_some_and(|t| t <= RELEASED_BOUND_US)
}

#[test]
fn a_pid_the_pmt_stops_calling_audio_is_waited_for_only_while_it_is_audio() {
    let silent = run(&audioless_with(|_| None));
    assert!(waits(silent), "never sends: {silent:?}");
    let cases = [
        (
            "flipped to private data",
            flip_audio_to(PES_PRIVATE_DATA, vec![]),
        ),
        ("flipped to video", flip_audio_to(AVC, vec![])),
        (
            "dropped from the PMT",
            audioless_with(|es| Some(vec![es[0].clone()])),
        ),
    ];
    for (why, bytes) in cases {
        let start = run(&bytes);
        assert!(released(start), "{why}: {start:?}");
    }
}

#[test]
fn a_pid_flipped_from_ac3_to_aac_is_still_waited_for_as_audio() {
    let bytes = test_fixtures::read(FIXTURE);
    let pmt = find(&bytes).expect("a PMT");
    let audio = pmt.es[1].pid;
    let kept: Vec<u8> = bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != audio)
        .flatten()
        .copied()
        .collect();
    let version = pmt.version;
    let (video, audio_es) = (pmt.es[0].clone(), pmt.es[1].clone());
    let flipped = rewrite(&kept, &pmt, &|at| {
        let mut e = audio_es.clone();
        if at < FLIP_AT {
            e.stream_type = PES_PRIVATE_DATA;
            e.descriptors = AC3_DESCRIPTOR.to_vec();
            Some((vec![video.clone(), e], version))
        } else {
            e.stream_type = AAC_ADTS;
            e.descriptors = vec![];
            Some((vec![video.clone(), e], version.wrapping_add(1)))
        }
    });
    let start = run(&flipped);
    assert!(waits(start), "{start:?}");
}

#[test]
fn a_pid_flipped_from_private_data_to_audio_starts_being_waited_for() {
    let bytes = test_fixtures::read(FIXTURE);
    let pmt = find(&bytes).expect("a PMT");
    let audio = pmt.es[1].pid;
    let kept: Vec<u8> = bytes
        .chunks(TS_PACKET)
        .filter(|p| pid_of(p) != audio)
        .flatten()
        .copied()
        .collect();
    let version = pmt.version;
    let (video, audio_es) = (pmt.es[0].clone(), pmt.es[1].clone());
    let plan = |data_until: f64| {
        let (video, audio_es) = (video.clone(), audio_es.clone());
        rewrite(&kept, &pmt, &move |at| {
            let mut e = audio_es.clone();
            if at < data_until {
                e.stream_type = PES_PRIVATE_DATA;
                e.descriptors = vec![];
                Some((vec![video.clone(), e], version.wrapping_add(1)))
            } else {
                Some((vec![video.clone(), e], version.wrapping_add(2)))
            }
        })
    };
    assert!(released(run(&plan(1.1))), "data throughout");
    assert!(waits(run(&plan(FLIP_AT))), "audio from {FLIP_AT}");
}
