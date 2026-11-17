//! Audit 21 finding 54 (issue #2949): the hostile-input caps on a fetched
//! resource body are inclusive boundaries, asserted through the public API.
use broadcast_hls::MediaSequence;
use broadcast_hls::limits::Limits;
use hls_runtime::client::{Error, HlsClient, HlsClientConfig, ResourceId};
use url::Url;

fn config() -> HlsClientConfig {
    HlsClientConfig::new(Url::parse("http://origin/live/media.m3u8").expect("URL"))
}

/// One segment playlist carrying an `EXT-X-MAP` (so the segment body is
/// buffered until the init, never demuxed).
fn playlist_with_map(first_msn: u64, count: u64) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n\
         #EXT-X-MEDIA-SEQUENCE:{first_msn}\n#EXT-X-MAP:URI=\"init.mp4\"\n"
    );
    for msn in first_msn..first_msn + count {
        text.push_str(&format!("#EXTINF:2.0,\nseg{msn}.m4s\n"));
    }
    text
}

/// A syntactically valid fMP4 media fragment (`moof` + `mdat`) of exactly
/// `len` bytes (ISO/IEC 14496-12:2026 §4.2: each box is a 32-bit size then a
/// four-character type).
fn fmp4_fragment(len: usize) -> Vec<u8> {
    assert!(len >= 16);
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&8u32.to_be_bytes());
    bytes.extend_from_slice(b"moof");
    let mdat_len = u32::try_from(len - 8).expect("fits");
    bytes.extend_from_slice(&mdat_len.to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.resize(len, 0);
    bytes
}

#[test]
fn the_resource_body_cap_is_an_inclusive_boundary() {
    let mut limits = Limits::default();
    limits.max_segment_bytes = 400;
    let mut client = HlsClient::new(config().with_limits(limits));
    client
        .on_playlist(playlist_with_map(0, 2).as_bytes())
        .expect("playlist parses");

    let at_cap = ResourceId::Segment {
        msn: MediaSequence::new(0),
    };
    client
        .on_resource(at_cap, &fmp4_fragment(400))
        .expect("a body of exactly max_segment_bytes must be accepted");

    let over_cap = ResourceId::Segment {
        msn: MediaSequence::new(1),
    };
    let err = client
        .on_resource(over_cap, &fmp4_fragment(401))
        .expect_err("one byte over the cap must be refused");
    assert!(
        matches!(
            err,
            Error::ResourceTooLarge {
                id,
                len: 401,
                max: 400
            } if id == over_cap
        ),
        "{err:?}"
    );
    assert!(
        client.retry_resource(over_cap),
        "a refused resource becomes requestable again"
    );
}
