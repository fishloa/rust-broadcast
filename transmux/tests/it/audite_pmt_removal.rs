//! Bug audit E: when a PMT version change removes a video ES, the access unit
//! the demux still holds for that PID (the last whole frame, held one-behind
//! for its duration, or still inside the PES assembler) is dropped silently,
//! while a plain `finish()` at the same point emits it.

#![cfg(feature = "std")]

use transmux::pipeline::CodecConfig;
use transmux::{DemuxEvent, StreamingTsDemux};

const TS: usize = 188;

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1F) as u16) << 8) | p[2] as u16
}

fn pmt_pid(ts: &[u8]) -> u16 {
    let pat = ts
        .chunks_exact(TS)
        .find(|p| pid(p) == 0 && p[1] & 0x40 != 0)
        .expect("PAT");
    let s = &pat[5..];
    let len = (((s[1] & 0x0F) as usize) << 8) | s[2] as usize;
    let loops = &s[8..3 + len - 4];
    loops
        .chunks_exact(4)
        .map(|c| (((c[2] & 0x1F) as u16) << 8) | c[3] as u16)
        .next()
        .expect("program")
}

fn pmt_without_video(orig: &[u8], cc: u8) -> Vec<u8> {
    let s = &orig[5..];
    let len = (((s[1] & 0x0F) as usize) << 8) | s[2] as usize;
    let sec = &s[..3 + len];
    let pil = (((sec[10] & 0x0F) as usize) << 8) | sec[11] as usize;
    let head_end = 12 + pil;
    let mut body = Vec::new();
    let mut i = head_end;
    while i + 5 <= sec.len() - 4 {
        let st = sec[i];
        let il = (((sec[i + 3] & 0x0F) as usize) << 8) | sec[i + 4] as usize;
        let end = i + 5 + il;
        if st != 0x1B {
            body.extend_from_slice(&sec[i..end]);
        }
        i = end;
    }
    let mut out = sec[..head_end].to_vec();
    out.extend_from_slice(&body);
    let new_len = out.len() - 3 + 4;
    out[1] = (out[1] & 0xF0) | ((new_len >> 8) as u8 & 0x0F);
    out[2] = new_len as u8;
    out[5] = (out[5] & !0x3E) | ((((out[5] >> 1) & 0x1F) + 1) & 0x1F) << 1;
    let crc = broadcast_common::crc32_mpeg2::compute(&out);
    out.extend_from_slice(&crc.to_be_bytes());
    let mut pkt = vec![0x47, 0x40 | (orig[1] & 0x1F), orig[2], 0x10 | cc, 0x00];
    pkt.extend_from_slice(&out);
    pkt.resize(TS, 0xFF);
    pkt
}

fn video_samples(demux: &mut StreamingTsDemux) -> usize {
    let mut video = None;
    let mut n = 0;
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::TrackAdded(spec) if matches!(spec.config, CodecConfig::Avc { .. }) => {
                video = Some(spec.track_id);
            }
            DemuxEvent::Sample { track_id, .. } if Some(track_id) == video => n += 1,
            _ => {}
        }
    }
    n
}

#[test]
fn pmt_removal_of_video_flushes_the_held_access_unit() {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let pmt = pmt_pid(&ts);
    let mid = (ts.len() / TS) / 2;
    let split = ts
        .chunks_exact(TS)
        .enumerate()
        .skip(mid)
        .find(|(_, p)| {
            let at = if (p[3] >> 4) & 3 == 3 { 5 + p[4] as usize } else { 4 };
            p[1] & 0x40 != 0 && at + 4 <= TS && p[at..at + 4] == [0, 0, 1, 0xE0]
        })
        .map(|(i, _)| i * TS)
        .expect("a video PES start after the midpoint");
    let (head, tail) = ts.split_at(split);
    let last_pmt = head
        .chunks_exact(TS)
        .filter(|p| pid(p) == pmt)
        .last()
        .expect("PMT in first half");

    let mut reference = StreamingTsDemux::new();
    reference.feed(head);
    reference.finish();
    let expected = video_samples(&mut reference);

    let mut with_change = StreamingTsDemux::new();
    with_change.feed(head);
    with_change.feed(&pmt_without_video(last_pmt, (last_pmt[3] + 1) & 0x0F));
    let tail_no_pmt: Vec<u8> = tail
        .chunks_exact(TS)
        .filter(|p| pid(p) != pmt)
        .flat_map(|p| p.iter().copied())
        .collect();
    with_change.feed(&tail_no_pmt);
    with_change.finish();
    let got = video_samples(&mut with_change);

    assert!(
        got >= expected,
        "video samples before the PMT removal: {got}, but finish() at the same point yields {expected}: the held last access unit is lost"
    );
}
