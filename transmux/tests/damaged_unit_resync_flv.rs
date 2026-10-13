//! Audit T16 (#1144) — FLV: a corrupt tag must not fail the whole file. Real
//! committed capture (`fixtures/flv/av.flv`), damage in the middle, valid tags
//! after it decoded byte-identically.

use broadcast_common::Unpackage;
use transmux::FlvDemux;
use transmux::ir::InputDegradation;
use transmux::media::Media;

const FLV_HEADER_PLUS_PREV: usize = 13;
const TAG_HEADER: usize = 11;

fn av_flv() -> Vec<u8> {
    let path = format!("{}/../fixtures/flv/av.flv", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `(offset, tag_type, data_size)` of every tag, walked by `DataSize`.
fn tags(d: &[u8]) -> Vec<(usize, u8, usize)> {
    let mut v = Vec::new();
    let mut off = FLV_HEADER_PLUS_PREV;
    while off + TAG_HEADER <= d.len() {
        let size = u32::from_be_bytes([0, d[off + 1], d[off + 2], d[off + 3]]) as usize;
        v.push((off, d[off], size));
        off += TAG_HEADER + size + 4;
    }
    v
}

fn samples(m: &Media, track: usize) -> Vec<Vec<u8>> {
    m.tracks[track]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect()
}

#[test]
fn corrupt_data_size_in_the_middle_resyncs_on_the_next_tag() {
    let clean_bytes = av_flv();
    let clean = FlvDemux::new().unpackage(&clean_bytes[..]).unwrap();
    assert!(clean.degraded.is_empty());
    let all = tags(&clean_bytes);
    // A video NALU tag (type 9) in the middle.
    let (victim, _, size) = all
        .iter()
        .copied()
        .skip(all.len() / 2)
        .find(|&(_, t, s)| t == 9 && s > 20)
        .expect("mid video tag");

    // Inflate the tag's DataSize by 5 bytes: its body now ends mid-next-tag, so
    // what follows is no tag header.
    let mut bytes = clean_bytes.clone();
    let bad = (size + 5) as u32;
    bytes[victim + 1..victim + 4].copy_from_slice(&bad.to_be_bytes()[1..]);

    let damaged = FlvDemux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt tag must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    assert_eq!(damaged.tracks.len(), clean.tracks.len());
    for t in 0..clean.tracks.len() {
        let (d, c) = (samples(&damaged, t), samples(&clean, t));
        assert_eq!(d.first(), c.first());
        assert_eq!(d.last(), c.last(), "later tags still decoded");
    }
    let total = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert_eq!(
        total(&damaged) + 1,
        total(&clean),
        "exactly the bad tag lost"
    );
}

#[test]
fn corrupt_nal_framing_inside_a_tag_drops_that_sample_only() {
    let clean_bytes = av_flv();
    let clean = FlvDemux::new().unpackage(&clean_bytes[..]).unwrap();
    let all = tags(&clean_bytes);
    let (victim, _, _) = all
        .iter()
        .copied()
        .skip(all.len() / 2)
        .find(|&(o, t, s)| t == 9 && s > 20 && clean_bytes[o + TAG_HEADER + 1] == 1)
        .expect("mid video NALU tag");

    // VideoTagHeader(1) + AVCPacketType(1) + CompositionTime(3) precede the
    // first NAL length prefix; make it claim far more than the tag holds.
    let mut bytes = clean_bytes.clone();
    let prefix = victim + TAG_HEADER + 5;
    bytes[prefix..prefix + 4].copy_from_slice(&[0x7F, 0xFF, 0xFF, 0xFF]);

    let damaged = FlvDemux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt NALU tag must not fail the file");
    assert_eq!(
        damaged.degraded,
        vec![InputDegradation::MalformedUnit {
            position: victim as u64
        }]
    );
    let (d, c) = (samples(&damaged, 0), samples(&clean, 0));
    assert_eq!(d.len() + 1, c.len());
    assert_eq!(d.last(), c.last());
    assert_eq!(d.first(), c.first());
}
