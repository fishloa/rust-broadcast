//! Bug audit E: a TS packet lost at the TAIL of a video PES (the gap is
//! detected on the next PUSI packet) must damage that access unit exactly as a
//! loss in the middle of the PES does; it must not surface as a clean sample.

#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::TsDemux;
use transmux::pipeline::CodecConfig;

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

fn video_pes_packet_ranges(ts: &[u8]) -> (u16, Vec<Vec<usize>>) {
    let mut vpid = None;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, p) in ts.chunks_exact(TS).enumerate() {
        let pusi = p[1] & 0x40 != 0;
        let at = payload_start(p);
        if pusi && at + 4 <= TS && p[at..at + 3] == [0, 0, 1] && (0xE0..0xF0).contains(&p[at + 3])
        {
            vpid.get_or_insert(pid(p));
        }
        if Some(pid(p)) == vpid {
            if pusi {
                groups.push(vec![i]);
            } else if let Some(g) = groups.last_mut() {
                g.push(i);
            }
        }
    }
    (vpid.expect("video pid"), groups)
}

fn without(ts: &[u8], drop: usize) -> Vec<u8> {
    ts.chunks_exact(TS)
        .enumerate()
        .filter(|(i, _)| *i != drop)
        .flat_map(|(_, p)| p.iter().copied())
        .collect()
}

fn video_sample_lens(ts: &[u8]) -> Vec<usize> {
    let media = TsDemux::new().unpackage(ts).expect("demux");
    let v = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("avc track");
    v.samples.iter().map(|s| s.data.len()).collect()
}

#[test]
fn lost_last_packet_of_a_video_pes_does_not_yield_a_truncated_clean_sample() {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let (_, groups) = video_pes_packet_ranges(&ts);
    let k = groups
        .iter()
        .enumerate()
        .skip(3)
        .find(|(_, g)| g.len() >= 3)
        .map(|(k, _)| k)
        .expect("a multi-packet video PES");
    let g = &groups[k];

    let base = video_sample_lens(&ts);
    let mid = video_sample_lens(&without(&ts, g[1]));
    let tail = video_sample_lens(&without(&ts, *g.last().unwrap()));

    assert_eq!(
        mid.len() + 1,
        base.len(),
        "control: a mid-PES loss drops the damaged access unit"
    );
    assert!(
        tail.len() < base.len() || tail.iter().all(|l| base.contains(l)),
        "tail loss: {} samples vs {} baseline; lengths {:?} include a truncated sample not in {:?}",
        tail.len(),
        base.len(),
        tail,
        base
    );
}
