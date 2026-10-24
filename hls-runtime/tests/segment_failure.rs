//! Round-8 F1/F2 core contract: a body that is not media is [`Error::NotMedia`], a failed
//! Part/Segment is re-requested by [`HlsClient::retry_resource`] with its original URL and byte
//! range and no playlist reload, and [`HlsClient::skip_resource`] gives one up with a
//! discontinuity.
//!
//! Spec: ISO/IEC 13818-1 §2.4.3.2 (TS `sync_byte`, 188-byte packets), ISO/IEC 14496-12 §8.8.4
//! (`moof` then `mdat`), RFC 8216 §4.3.4.3.

use hls_runtime::client::{Action, Error, HlsClient, Output, ResourceId};

const URL: &str = "http://origin/v/media.m3u8";
const TS_PACKET_LEN: usize = 188;
const TS_SYNC_BYTE: u8 = 0x47;
const GARBAGE_LEN: usize = 4096;
const SEG0: ResourceId = ResourceId::Segment { msn: 0 };
const SEG1: ResourceId = ResourceId::Segment { msn: 1 };

fn ts(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/ts-hls/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn vod() -> String {
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\nindex1.ts\n#EXT-X-ENDLIST\n"
        .to_string()
}

fn live() -> String {
    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\nindex1.ts\n"
        .to_string()
}

fn fetches(client: &mut HlsClient) -> Vec<(ResourceId, String, Option<(u64, u64)>)> {
    let mut out = Vec::new();
    while let Some(action) = client.poll() {
        if let Action::FetchResource {
            id,
            url,
            byte_range,
        } = action
        {
            out.push((id, url, byte_range));
        }
    }
    out
}

fn outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

fn garbage() -> Vec<u8> {
    (0..GARBAGE_LEN)
        .map(|i| u8::try_from(i.wrapping_mul(37) % 251).unwrap())
        .collect()
}

#[test]
fn a_body_that_is_not_media_is_not_media_and_can_be_retried_without_a_playlist() {
    for body in [
        garbage(),
        Vec::new(),
        b"<html>503</html>".to_vec(),
        vec![TS_SYNC_BYTE; 1],
    ] {
        let mut client = HlsClient::new(URL);
        let _ = fetches(&mut client);
        client.on_playlist(vod().as_bytes()).unwrap();
        let first = fetches(&mut client);
        assert_eq!(first.len(), 2);

        let err = client.on_resource(SEG0, &body).unwrap_err();
        assert!(
            matches!(err, Error::NotMedia { id } if id == SEG0),
            "{err:?}"
        );
        assert!(outputs(&mut client).is_empty(), "nothing emitted for it");
        client.on_error(Some(SEG0));

        assert!(client.retry_resource(SEG0));
        let again = fetches(&mut client);
        assert_eq!(again, vec![first[0].clone()], "same url, same range");
        assert!(!client.retry_resource(SEG0), "already requested");

        client.on_resource(SEG0, &ts("index0.ts")).unwrap();
        client.on_resource(SEG1, &ts("index1.ts")).unwrap();
        assert!(
            outputs(&mut client)
                .iter()
                .any(|o| matches!(o, Output::EndOfStream))
        );
    }
}

#[test]
fn media_that_yields_no_samples_is_accepted_and_never_retried() {
    let one_null_packet = {
        let mut packet = vec![0xFF; TS_PACKET_LEN];
        packet[..4].copy_from_slice(&[TS_SYNC_BYTE, 0x1F, 0xFF, 0x10]);
        packet
    };
    let psi_only = ts("index0.ts")[..2 * TS_PACKET_LEN].to_vec();
    for body in [one_null_packet, psi_only] {
        let mut client = HlsClient::new(URL);
        let _ = fetches(&mut client);
        client.on_playlist(vod().as_bytes()).unwrap();
        let _ = fetches(&mut client);
        client.on_resource(SEG0, &body).unwrap();
        assert!(!client.retry_resource(SEG0), "delivered, nothing to retry");
    }
}

#[test]
fn a_lane_lattice_below_half_sync_is_not_media() {
    let mut body = vec![0u8; 4 * TS_PACKET_LEN];
    body[0] = TS_SYNC_BYTE;
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(vod().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    assert!(matches!(
        client.on_resource(SEG0, &body),
        Err(Error::NotMedia { .. })
    ));
}

#[test]
fn retry_resource_has_nothing_for_unknown_or_delivered_ids_and_never_for_init() {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    assert!(!client.retry_resource(SEG0));
    client.on_playlist(vod().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    assert!(!client.retry_resource(SEG0), "in flight");
    assert!(!client.retry_resource(ResourceId::Init));
    client.on_resource(SEG0, &ts("index0.ts")).unwrap();
    assert!(!client.retry_resource(SEG0), "delivered");
}

#[test]
fn a_retried_byte_range_is_requested_with_its_original_range() {
    let ranged = "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                  #EXTINF:2.0,\n#EXT-X-BYTERANGE:1000@500\nall.ts\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(ranged.as_bytes()).unwrap();
    let first = fetches(&mut client);
    assert_eq!(first[0].2, Some((500, 1000)));
    assert!(matches!(
        client.on_resource(SEG0, &garbage()),
        Err(Error::NotMedia { .. })
    ));
    client.on_error(Some(SEG0));
    assert!(client.retry_resource(SEG0));
    assert_eq!(fetches(&mut client), first);
}

#[test]
fn skipping_a_segment_signals_a_discontinuity_and_it_is_never_requested_again() {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(live().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    client.on_resource(SEG0, &ts("index0.ts")).unwrap();
    let _ = outputs(&mut client);
    client.on_error(Some(SEG1));

    client.skip_resource(SEG1);
    let after = outputs(&mut client);
    assert_eq!(
        after
            .iter()
            .filter(|o| matches!(o, Output::Discontinuity))
            .count(),
        1,
        "{after:?}"
    );
    assert!(!client.retry_resource(SEG1));
    client.on_playlist(live().as_bytes()).unwrap();
    let again = fetches(&mut client);
    assert!(
        !again.iter().any(|(id, _, _)| *id == SEG1),
        "a later reload must not ask for the skipped segment: {again:?}"
    );
}

#[test]
fn skipping_the_last_open_segment_of_an_ended_playlist_ends_the_stream() {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(vod().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    client.on_resource(SEG0, &ts("index0.ts")).unwrap();
    client.on_error(Some(SEG1));
    client.skip_resource(SEG1);
    let after = outputs(&mut client);
    assert!(
        after.iter().any(|o| matches!(o, Output::EndOfStream)),
        "{after:?}"
    );
}

#[test]
fn init_cannot_be_skipped() {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.skip_resource(ResourceId::Init);
    assert!(outputs(&mut client).is_empty());
}
