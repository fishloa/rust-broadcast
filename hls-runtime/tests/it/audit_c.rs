//! Bug audit C reproductions (hls-runtime). Each test fails on main.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
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

mod client {
    use hls_runtime::client::{Action, HlsClient, Output, ResourceId};

    const URL: &str = "http://origin/live/media.m3u8";

    const OPEN_WITH_TWO_PARTS: &str = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-PART:DURATION=0.5,URI=\"p0.0.m4s\",INDEPENDENT=YES\n#EXT-X-PART:DURATION=0.5,URI=\"p0.1.m4s\"\n";

    const SEGMENT_CLOSED_WITHOUT_PARTS: &str = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1.0,\nseg0.m4s\n";

    fn drain(client: &mut HlsClient) -> Vec<Action> {
        std::iter::from_fn(|| client.poll()).collect()
    }

    fn fetches(actions: &[Action]) -> Vec<ResourceId> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::FetchResource { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// A part whose fetch failed (`on_error`) while its segment was open is
    /// silently abandoned once the segment closes without listing parts: the
    /// segment is marked delivered because *another* part of it arrived.
    #[test]
    fn a_failed_part_is_not_abandoned_when_its_segment_closes() {
        let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
        let fragment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        client.on_playlist(OPEN_WITH_TWO_PARTS.as_bytes()).unwrap();
        let first = fetches(&drain(&mut client));
        assert!(first.contains(&ResourceId::Part { msn: 0, part: 0 }));
        assert!(first.contains(&ResourceId::Part { msn: 0, part: 1 }));
        client.on_resource(ResourceId::Init, &init).unwrap();
        client
            .on_resource(ResourceId::Part { msn: 0, part: 0 }, &fragment)
            .unwrap();
        let samples = std::iter::from_fn(|| client.next_output())
            .filter(|o| matches!(o, Output::Samples { .. }))
            .count();
        assert!(samples > 0, "part 0 must have been delivered");
        client.on_error(Some(ResourceId::Part { msn: 0, part: 1 }));

        client
            .on_playlist(SEGMENT_CLOSED_WITHOUT_PARTS.as_bytes())
            .unwrap();
        let second = fetches(&drain(&mut client));
        assert!(
            second.contains(&ResourceId::Part { msn: 0, part: 1 }),
            "part 1 failed and its segment closed: it must be re-requested, got {second:?}"
        );
    }

    const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000\nhi.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=500000\nlo.m3u8\n";

    fn live(first_msn: u64, uri_prefix: &str) -> String {
        let mut text = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first_msn}\n"
        );
        for i in 0..3 {
            text.push_str(&format!("#EXTINF:2.0,\n{uri_prefix}{}.ts\n", first_msn + i));
        }
        text
    }

    /// One variant switch is one discontinuity. `select_variant` already
    /// restarts the timeline, but leaves the old variant's media-sequence
    /// high-water mark, so the first playlist of a variant numbered lower is
    /// judged a second restart and a second `Discontinuity` is emitted.
    #[test]
    fn a_variant_switch_emits_exactly_one_discontinuity() {
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        client.on_playlist(MASTER.as_bytes()).unwrap();
        drain(&mut client);
        client.on_playlist(live(1000, "hi").as_bytes()).unwrap();
        drain(&mut client);
        let _ = std::iter::from_fn(|| client.next_output()).count();

        client.select_variant(1).unwrap();
        drain(&mut client);
        client.on_playlist(live(10, "lo").as_bytes()).unwrap();
        let discontinuities = std::iter::from_fn(|| client.next_output())
            .filter(|o| matches!(o, Output::Discontinuity))
            .count();
        assert_eq!(discontinuities, 1);
    }

    const OPEN_WITH_ONE_PART: &str = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXT-X-PART:DURATION=0.5,URI=\"p0.0.m4s\",INDEPENDENT=YES\n";

    const DISCONTINUOUS_SEGMENT_CLOSED: &str = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-DISCONTINUITY\n#EXTINF:1.0,\nseg0.m4s\n";

    /// A discontinuity that is only revealed when its segment closes, after
    /// every part of the segment was already delivered while it was open, is
    /// documented as "signalled late". It is never signalled: the only
    /// emission site is the delivery of a resource of that segment.
    #[test]
    fn a_discontinuity_revealed_at_segment_close_is_signalled_late_not_never() {
        let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
        let fragment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        client.on_playlist(OPEN_WITH_ONE_PART.as_bytes()).unwrap();
        drain(&mut client);
        client.on_resource(ResourceId::Init, &init).unwrap();
        client
            .on_resource(ResourceId::Part { msn: 0, part: 0 }, &fragment)
            .unwrap();
        client
            .on_playlist(DISCONTINUOUS_SEGMENT_CLOSED.as_bytes())
            .unwrap();
        let fetched = fetches(&drain(&mut client));
        assert!(fetched.is_empty(), "nothing left to fetch: {fetched:?}");
        let discontinuities = std::iter::from_fn(|| client.next_output())
            .filter(|o| matches!(o, Output::Discontinuity))
            .count();
        assert_eq!(discontinuities, 1);
    }
}
