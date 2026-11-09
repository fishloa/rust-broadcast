//! Bug audit C reproductions (hls-runtime). Each test fails on main.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::{PartIndex, SegmentSeq};
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn setup() -> (Arc<Trunk>, SegmentWriter, HlsOrigin) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let w = trunk.segment_writer().expect("segment writer");
    let o = HlsOrigin::builder(Arc::clone(&trunk))
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(7)
        .build()
        .expect("origin builds");
    (trunk, w, o)
}

fn part(w: &SegmentWriter, seg: u32, idx: u32) {
    w.publish_part(PartEntry::new(
        Bytes::from(vec![idx as u8; 4]),
        SegmentSeq::new(seg),
        PartIndex::new(idx),
        Duration::from_millis(500),
        idx == 0,
    ));
}

fn seg(w: &SegmentWriter, seq: u32) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(vec![seq as u8; 8]),
        SegmentSeq::new(seq),
        Duration::from_secs(4),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic");
}

fn request(o: &HlsOrigin, msn: u64, part: Option<u32>) -> EgressResponse<HlsBody> {
    o.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery {
                hls_msn: Some(msn),
                hls_part: part,
                hls_skip: None,
                hls_primary: None,
            },
        },
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

/// A blocking reload for a part that already exists in a just-closed segment
/// (the closing publish follows the last part immediately) must be answered
/// from the playlist that already lists it, not parked until the next
/// segment's first part appears.
#[test]
fn blocking_reload_for_an_existing_part_of_the_just_closed_segment_is_answered() {
    let (_trunk, w, o) = setup();
    part(&w, 1, 0);
    part(&w, 1, 1);
    seg(&w, 1);
    let response = request(&o, 1, Some(1));
    assert!(
        matches!(response, EgressResponse::Ready { .. }),
        "got {response:?}"
    );
}
