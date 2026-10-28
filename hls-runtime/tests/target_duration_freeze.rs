#![cfg(feature = "std")]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
use media_plane::ServedEgress;
use media_plane::egress::{AwaitPolicy, EgressResponse};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with() -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    let writer = trunk.segment_writer().expect("first segment writer");
    (trunk, writer)
}

fn origin(trunk: &Arc<Trunk>) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .target_duration_secs(1.0)
        .window_segments(nz(8))
        .instance(7)
        .build()
        .expect("origin builds")
}

fn publish(w: &SegmentWriter, seq: u32, secs: u64) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(vec![seq as u8; 8]),
        SegmentSeq::new(seq),
        Duration::from_secs(secs),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic sequence number");
}

fn playlist(origin: &HlsOrigin) -> String {
    let (now, policy) = (
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    match origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: hls_runtime::server::BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected a playlist, got {other:?}"),
    }
}

fn target_duration(body: &str) -> u32 {
    body.lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .expect("target duration")
        .parse()
        .expect("integer")
}

#[test]
fn target_duration_is_frozen_after_first_publication() {
    let (trunk, w) = trunk_with();
    let o = origin(&trunk);
    publish(&w, 1, 2);
    let first = playlist(&o);
    assert_eq!(target_duration(&first), 2);
    publish(&w, 2, 4);
    let second = playlist(&o);
    assert_eq!(
        target_duration(&second),
        2,
        "EXT-X-TARGETDURATION must not change once published: {second}"
    );
}
