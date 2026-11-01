use crate::support;

use broadcast_client::{Config, Event};
use broadcast_common::Unpackage;
use support::rtsp_host::{BASE_URL, Host, Server, interleaved};

const FRAME_STEP_US: u64 = 10_000;
const IDLE_STEPS: usize = 200;
const VIDEO_CHANNEL: u8 = 0;
const FRAME_TICKS: i64 = 3_600;
const CLOCK_HZ: f64 = 90_000.0;
const RTP_TS_AT: usize = 4;
const RTP_TS_END: usize = 8;
const LEN_PREFIX: usize = 2;
const PYRAMID: &str = "rtp/ibbbp-pyramid";
const NO_PYRAMID: &str = "rtp/ibbbp-nopyramid";
const NO_B: &str = "rtp/ippp";
const PYRAMID_TS: &str = "rtp/ibbbp-pyramid.ts";
const WRAP_AT_AU: usize = 60;

struct Capture {
    sdp: String,
    packets: Vec<Vec<u8>>,
}

fn load(stem: &str) -> Option<Capture> {
    let dump = test_fixtures::require_bytes(&format!("{stem}.rtpdump"), "rtsp_dts")?;
    let sdp = test_fixtures::require_bytes(&format!("{stem}.sdp"), "rtsp_dts")?;
    let mut packets = Vec::new();
    let mut at = 0;
    while at < dump.len() {
        let len = usize::from(u16::from_be_bytes([dump[at], dump[at + 1]]));
        packets.push(dump[at + LEN_PREFIX..at + LEN_PREFIX + len].to_vec());
        at += LEN_PREFIX + len;
    }
    Some(Capture {
        sdp: String::from_utf8(sdp).expect("utf8 sdp"),
        packets,
    })
}

#[derive(Debug, Clone, Copy)]
struct Stamp {
    dts: i64,
    pts: i64,
    duration: u32,
}

fn play(sdp: &str, packets: &[Vec<u8>]) -> Vec<Stamp> {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(sdp));
    host.connect();
    let mut out = Vec::new();
    let mut collect = |host: &mut Host| {
        while let Some(e) = host.session.next_event() {
            if let Event::Sample { sample, .. } = e {
                out.push(Stamp {
                    dts: sample.dts.expect("dts"),
                    pts: sample.pts.expect("pts"),
                    duration: sample.duration.expect("duration"),
                });
            }
        }
    };
    collect(&mut host);
    for packet in packets {
        host.now += FRAME_STEP_US;
        host.deliver(&interleaved(VIDEO_CHANNEL, packet));
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
    out
}

fn reorder_depth_of(stamps: &[Stamp]) -> usize {
    (0..stamps.len())
        .map(|k| stamps[..k].iter().filter(|e| e.pts > stamps[k].pts).count())
        .max()
        .unwrap_or(0)
}

fn assert_monotonic_and_causal(stamps: &[Stamp], what: &str) {
    for (k, w) in stamps.windows(2).enumerate() {
        assert!(
            w[0].dts < w[1].dts,
            "{what}: dts not strictly increasing at {k}: {} then {}",
            w[0].dts,
            w[1].dts
        );
    }
    for (k, s) in stamps.iter().enumerate() {
        assert!(
            s.dts <= s.pts,
            "{what}: dts {} > pts {} at {k}",
            s.dts,
            s.pts
        );
    }
}

fn assert_frame_grid(stamps: &[Stamp], depth: usize, what: &str) {
    let mut sorted: Vec<i64> = stamps.iter().map(|s| s.pts).collect();
    sorted.sort_unstable();
    for (k, s) in stamps.iter().enumerate() {
        let want = if k >= depth {
            sorted[k - depth]
        } else {
            sorted[0] - FRAME_TICKS * i64::try_from(depth - k).expect("small")
        };
        assert_eq!(s.dts, want, "{what}: dts of decoded frame {k}");
    }
}

fn frame_rate(stamps: &[Stamp]) -> f64 {
    let mut steps: Vec<i64> = stamps.windows(2).map(|w| w[1].dts - w[0].dts).collect();
    steps.sort_unstable();
    CLOCK_HZ / steps[steps.len() / 2] as f64
}

fn b_frame_case(stem: &str, depth: usize) {
    let Some(cap) = load(stem) else { return };
    let stamps = play(&cap.sdp, &cap.packets);
    assert!(stamps.len() >= 170, "{stem}: only {} samples", stamps.len());
    assert_eq!(
        reorder_depth_of(&stamps),
        depth,
        "{stem}: fixture reorder depth"
    );
    assert_monotonic_and_causal(&stamps, stem);
    assert_frame_grid(&stamps, depth, stem);
    assert!(
        (frame_rate(&stamps) - 25.0).abs() < 1e-9,
        "{stem}: frame rate {}",
        frame_rate(&stamps)
    );
    let offsets: std::collections::BTreeSet<i64> = stamps
        .iter()
        .skip(depth)
        .map(|s| (s.pts - s.dts) / FRAME_TICKS)
        .collect();
    assert!(
        offsets
            .iter()
            .all(|&o| (0..=2 * i64::try_from(depth + 1).unwrap() + 1).contains(&o)),
        "{stem}: composition offsets (frames) {offsets:?}"
    );
    assert!(
        offsets.iter().any(|&o| o > 0),
        "{stem}: no composition offset"
    );
}

#[test]
fn pyramid_ibbbp_gets_monotonic_dts_on_the_pts_grid_shifted_by_two_frames() {
    b_frame_case(PYRAMID, 2);
}

#[test]
fn non_pyramid_ibbbp_gets_monotonic_dts_on_the_pts_grid_shifted_by_one_frame() {
    b_frame_case(NO_PYRAMID, 1);
}

#[test]
fn a_stream_without_b_frames_keeps_dts_equal_to_pts() {
    let Some(cap) = load(NO_B) else { return };
    let stamps = play(&cap.sdp, &cap.packets);
    assert!(stamps.len() >= 170);
    assert!(stamps.iter().all(|s| s.dts == s.pts));
    assert_monotonic_and_causal(&stamps, NO_B);
    assert!((frame_rate(&stamps) - 25.0).abs() < 1e-9);
}

#[test]
fn durations_stay_one_frame_period() {
    let Some(cap) = load(PYRAMID) else { return };
    let stamps = play(&cap.sdp, &cap.packets);
    let period = u32::try_from(FRAME_TICKS).unwrap();
    assert!(
        stamps.iter().filter(|s| s.duration == period).count() * 10 >= stamps.len() * 9,
        "durations: {:?}",
        stamps.iter().map(|s| s.duration).collect::<Vec<_>>()
    );
}

#[test]
fn a_lost_packet_keeps_dts_monotonic_and_causal() {
    let Some(cap) = load(PYRAMID) else { return };
    for lost in (1..cap.packets.len()).step_by(7) {
        let mut packets = cap.packets.clone();
        packets.remove(lost);
        let stamps = play(&cap.sdp, &packets);
        assert!(
            stamps.len() >= 150,
            "loss at {lost}: {} samples",
            stamps.len()
        );
        assert_monotonic_and_causal(&stamps, &format!("loss at {lost}"));
    }
}

fn au_start_indices(packets: &[Vec<u8>]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, p) in packets.iter().enumerate() {
        if p[1] & 0x80 != 0 && i + 1 < packets.len() {
            starts.push(i + 1);
        }
    }
    starts
}

#[test]
fn a_32_bit_rtp_timestamp_wrap_mid_stream_keeps_the_dts_grid() {
    let Some(cap) = load(PYRAMID) else { return };
    let starts = au_start_indices(&cap.packets);
    let at = starts[WRAP_AT_AU];
    let ts_at = |p: &Vec<u8>| u32::from_be_bytes(p[RTP_TS_AT..RTP_TS_END].try_into().unwrap());
    let offset = 0u32.wrapping_sub(ts_at(&cap.packets[at]));
    let shifted: Vec<Vec<u8>> = cap
        .packets
        .iter()
        .map(|p| {
            let mut q = p.clone();
            q[RTP_TS_AT..RTP_TS_END].copy_from_slice(&ts_at(p).wrapping_add(offset).to_be_bytes());
            q
        })
        .collect();
    assert!(
        shifted.iter().any(|p| ts_at(p) < ts_at(&shifted[0])),
        "the shifted capture must actually wrap"
    );
    let reference = play(&cap.sdp, &cap.packets);
    let stamps = play(&cap.sdp, &shifted);
    assert_eq!(stamps.len(), reference.len());
    assert_monotonic_and_causal(&stamps, "wrapped");
    assert_frame_grid(&stamps, 2, "wrapped");
    let base = stamps[0].dts - reference[0].dts;
    for (k, (a, b)) in stamps.iter().zip(&reference).enumerate() {
        assert_eq!(
            a.dts - base,
            b.dts,
            "dts of frame {k} differs across the wrap"
        );
        assert_eq!(
            a.pts - base,
            b.pts,
            "pts of frame {k} differs across the wrap"
        );
    }
}

#[test]
fn the_rtp_composition_offsets_match_the_transport_stream_of_the_same_encode() {
    let Some(cap) = load(PYRAMID) else { return };
    let Some(ts) = test_fixtures::require_bytes(PYRAMID_TS, "rtsp_dts") else {
        return;
    };
    let media = transmux::TsDemux::new()
        .unpackage(&ts[..])
        .expect("demux ts");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .expect("video track");
    let ts_offsets: Vec<i64> = video
        .samples
        .iter()
        .map(|s| (s.pts.unwrap() - s.dts.unwrap()) / FRAME_TICKS)
        .collect();
    let rtp = play(&cap.sdp, &cap.packets);
    let rtp_offsets: Vec<i64> = rtp.iter().map(|s| (s.pts - s.dts) / FRAME_TICKS).collect();
    let gop = 50;
    assert!(ts_offsets.len() >= gop && rtp_offsets.len() >= gop);
    assert_eq!(
        rtp_offsets[..gop],
        ts_offsets[..gop],
        "per-frame pts - dts (frames) over the first GOP, RTP-derived vs TS"
    );
    let ts_steps: Vec<i64> = video
        .samples
        .windows(2)
        .map(|w| w[1].dts.unwrap() - w[0].dts.unwrap())
        .collect();
    assert!(ts_steps.iter().all(|&s| s == FRAME_TICKS));
}
