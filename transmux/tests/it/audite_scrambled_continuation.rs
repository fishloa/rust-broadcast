//! Bug audit E: a PES whose continuation packets arrive with
//! `transport_scrambling_control != 0` (an undescrambled crypto-period
//! boundary) must not surface as a clean sample of garbled bytes.

#![cfg(feature = "std")]

use transmux::pipeline::CodecConfig;
use transmux::{DemuxEvent, StreamingTsDemux};

const TS: usize = 188;

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1F) as u16) << 8) | p[2] as u16
}

fn payload_start(p: &[u8]) -> usize {
    match (p[3] >> 4) & 3 {
        3 => 5 + p[4] as usize,
        2 | 0 => TS,
        _ => 4,
    }
}

struct Run {
    samples: Vec<Vec<u8>>,
    degraded: usize,
}

fn run(ts: &[u8]) -> Run {
    let mut d = StreamingTsDemux::new();
    d.feed(ts);
    d.finish();
    let mut video = None;
    let mut samples = Vec::new();
    let mut degraded = 0;
    while let Some(ev) = d.poll_event() {
        match ev {
            DemuxEvent::TrackAdded(s) if matches!(s.config, CodecConfig::Avc { .. }) => {
                video = Some(s.track_id)
            }
            DemuxEvent::Sample { track_id, sample, .. } if Some(track_id) == video => {
                samples.push(sample.data.to_vec())
            }
            DemuxEvent::InputDegraded { .. } => degraded += 1,
            _ => {}
        }
    }
    Run { samples, degraded }
}

#[test]
fn scrambled_continuation_packet_is_not_delivered_as_clean_video() {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let mut vpid = None;
    let mut target = None;
    let mut seen = 0;
    for (i, p) in ts.chunks_exact(TS).enumerate() {
        let at = payload_start(p);
        let pusi = p[1] & 0x40 != 0;
        if pusi && at + 4 <= TS && p[at..at + 3] == [0, 0, 1] && (0xE0..0xF0).contains(&p[at + 3])
        {
            vpid.get_or_insert(pid(p));
            if Some(pid(p)) == vpid {
                seen += 1;
            }
        }
        if seen >= 4 && !pusi && Some(pid(p)) == vpid && at < TS - 16 {
            target = Some(i);
            break;
        }
    }
    let target = target.expect("a video continuation packet");
    let base = run(&ts);

    let mut bad = ts.clone();
    let p = &mut bad[target * TS..(target + 1) * TS];
    let at = payload_start(p);
    p[3] |= 0x80;
    for b in &mut p[at..] {
        *b ^= 0x55;
    }
    let got = run(&bad);

    let corrupt_clean = got.samples.iter().filter(|s| !base.samples.contains(s)).count();
    assert!(
        corrupt_clean == 0 || got.degraded > base.degraded,
        "{corrupt_clean} video sample(s) built from a scrambled packet were delivered as clean (degraded events {} vs baseline {})",
        got.degraded,
        base.degraded
    );
}
