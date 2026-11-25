//! Audit T16 (#1144) — FLV damage. The streaming demuxer has no
//! resynchronisation point after a corrupt tag (a malformed FLV tag header
//! leaves none), so damage after the good prefix is reported as a typed error
//! and the samples decoded before it are still delivered. Real committed
//! capture (`fixtures/flv/av.flv`), damage in the middle.
use transmux::media::Media;
use transmux::{DemuxEvent, FlvError, StreamingFlvDemux, TrackId};

const FLV_HEADER_PLUS_PREV: usize = 13;
const TAG_HEADER: usize = 11;

fn av_flv() -> Vec<u8> {
    let path = test_fixtures::path("flv/av.flv");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
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

/// Feed the whole buffer, then drain every sample emitted before the error.
fn streamed_prefix(bytes: &[u8]) -> (Vec<Vec<u8>>, FlvError) {
    let mut demux = StreamingFlvDemux::new();
    let error = demux.feed(bytes).expect_err("the damage is reported");
    demux.finish();
    let mut per_track: std::collections::BTreeMap<TrackId, Vec<Vec<u8>>> =
        std::collections::BTreeMap::new();
    while let Some(event) = demux.poll_event() {
        if let DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        {
            per_track
                .entry(track_id)
                .or_default()
                .push(sample.data.to_vec());
        }
    }
    let first_track = per_track
        .values()
        .find(|samples| !samples.is_empty())
        .cloned()
        .unwrap_or_default();
    (first_track, error)
}

fn samples(m: &Media, track: usize) -> Vec<Vec<u8>> {
    m.tracks[track]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect()
}

#[test]
fn corrupt_data_size_in_the_middle_is_reported_and_the_good_prefix_kept() {
    let clean_bytes = av_flv();
    let clean = test_harness::collect::media_from_flv(&clean_bytes[..]).unwrap();
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

    let (prefix, error) = streamed_prefix(&bytes);
    assert!(
        matches!(
            error,
            FlvError::Tag(rtmp_runtime::RtmpError::Malformed {
                what: "FLV tag PreviousTagSize"
            })
        ),
        "the mis-framed tag is reported: {error:?}"
    );
    let (_, clean_video) = ((), samples(&clean, 0));
    assert_eq!(
        prefix.first(),
        clean_video.first(),
        "the good prefix is kept"
    );
    assert!(
        prefix.len() < clean_video.len(),
        "the damage and what follows it are not decoded"
    );
}

#[test]
fn corrupt_nal_framing_inside_a_tag_is_reported_and_the_good_prefix_kept() {
    let clean_bytes = av_flv();
    let clean = test_harness::collect::media_from_flv(&clean_bytes[..]).unwrap();
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

    let (emitted, error) = streamed_prefix(&bytes);
    assert!(
        matches!(error, FlvError::Codec(transmux::Error::BufferTooShort(_))),
        "the corrupt NAL length is reported: {error:?}"
    );
    let clean_video = samples(&clean, 0);
    assert_eq!(
        emitted.first(),
        clean_video.first(),
        "the good prefix is kept"
    );
    assert!(emitted.len() < clean_video.len());
}
