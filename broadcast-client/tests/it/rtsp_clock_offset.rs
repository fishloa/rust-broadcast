use crate::support;

use broadcast_client::{BufferEvent, Config, Event};
use support::rtsp_host::{BASE_URL, Host, MediaFixture, Server, channel_of, interleaved};
use transmux::rtp::RtpMediaKind;

const FRAME_STEP_US: u64 = 10_000;
const IDLE_STEPS: usize = 400;
const RTP_TIMESTAMP_AT: usize = 4;
const RTP_TIMESTAMP_LEN: usize = 4;
const TRACK_CLOCK_OFFSET_TICKS: u32 = 4_000_000_000;
const START_TOLERANCE_US: u64 = 500_000;

fn frames(fixture: &MediaFixture, video_offset: u32, audio_offset: u32) -> Vec<Vec<u8>> {
    let longest = fixture
        .tracks
        .iter()
        .map(|t| t.packets.len())
        .max()
        .unwrap_or(0);
    let mut frames = Vec::new();
    for i in 0..longest {
        for (n, track) in fixture.tracks.iter().enumerate() {
            let Some(packet) = track.packets.get(i) else {
                continue;
            };
            let mut packet = packet.clone();
            let offset = if track.kind == RtpMediaKind::H264 {
                video_offset
            } else {
                audio_offset
            };
            if offset != 0 {
                let at = RTP_TIMESTAMP_AT..RTP_TIMESTAMP_AT + RTP_TIMESTAMP_LEN;
                let ts = u32::from_be_bytes(packet[at.clone()].try_into().expect("ts"));
                packet[at].copy_from_slice(&ts.wrapping_add(offset).to_be_bytes());
            }
            frames.push(interleaved(channel_of(n), &packet));
        }
    }
    frames
}

fn start_time(fixture: &MediaFixture, video_offset: u32, audio_offset: u32) -> Option<u64> {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture.sdp));
    host.connect();
    let begin = host.now;
    let mut start = None;
    let mut note = |host: &mut Host| {
        let now = host.now;
        for e in host.events() {
            if matches!(e, Event::Buffer(BufferEvent::Start { .. })) && start.is_none() {
                start = Some(now - begin);
            }
        }
    };
    note(&mut host);
    for frame in frames(fixture, video_offset, audio_offset) {
        host.now += FRAME_STEP_US;
        let conn = host.conn.expect("conn");
        host.session.on_data(conn, &frame, host.now);
        host.session.on_playback_position(host.now, None);
        host.drain();
        note(&mut host);
    }
    for _ in 0..IDLE_STEPS {
        host.now += FRAME_STEP_US;
        host.session.on_playback_position(host.now, None);
        host.drain();
        note(&mut host);
    }
    start
}

#[test]
fn start_does_not_depend_on_the_rtp_clock_offset_between_tracks() {
    let fixture = MediaFixture::load();
    let aligned = start_time(&fixture, 0, 0).expect("aligned start");
    for (video, audio) in [(0, TRACK_CLOCK_OFFSET_TICKS), (TRACK_CLOCK_OFFSET_TICKS, 0)] {
        let shifted = start_time(&fixture, video, audio).unwrap_or_else(|| {
            panic!("no Start with RTP clock offsets video {video} audio {audio}")
        });
        assert!(
            shifted <= aligned + START_TOLERANCE_US,
            "RTP clock offsets video {video} audio {audio} delay Start to {shifted} us, aligned {aligned} us"
        );
    }
}
