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

type Fetch = (ResourceId, String, Option<(u64, u64)>);

fn fetches(client: &mut HlsClient) -> Vec<Fetch> {
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

const MID_PACKET_CUT: usize = 100;
const PACKETS_PER_PROBE: usize = 40;

fn accepted(body: &[u8]) -> bool {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(vod().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    match client.on_resource(SEG0, body) {
        Ok(()) => true,
        Err(Error::NotMedia { .. }) => false,
        Err(other) => panic!("unexpected {other:?}"),
    }
}

fn samples_of(body: &[u8]) -> usize {
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(vod().as_bytes()).unwrap();
    let _ = fetches(&mut client);
    client.on_resource(SEG0, body).unwrap();
    client.on_resource(SEG1, &ts("index1.ts")).unwrap();
    outputs(&mut client)
        .iter()
        .map(|o| match o {
            Output::Samples { samples, .. } => samples.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn ts_segments_at_any_packet_phase_are_media() {
    let whole = ts("index0.ts");
    let aligned = &whole[..PACKETS_PER_PROBE * TS_PACKET_LEN];
    let cut = &whole[MID_PACKET_CUT..MID_PACKET_CUT + PACKETS_PER_PROBE * TS_PACKET_LEN];
    let mut junk = vec![0xA5; TS_PACKET_LEN - 1];
    junk.extend_from_slice(aligned);
    for (name, body) in [
        ("aligned", aligned.to_vec()),
        ("cut 100 bytes into a packet", cut.to_vec()),
        ("187 bytes of junk first", junk),
        ("whole file cut 100 in", whole[MID_PACKET_CUT..].to_vec()),
    ] {
        assert!(accepted(&body), "{name}");
    }
}

#[test]
fn tiny_ts_segments_need_every_lane_synced() {
    let whole = ts("index0.ts");
    assert!(accepted(&whole[..TS_PACKET_LEN]), "one packet");
    assert!(accepted(&whole[..2 * TS_PACKET_LEN]), "two packets");
    let nulls = {
        let mut packet = vec![0xFF; TS_PACKET_LEN];
        packet[..4].copy_from_slice(&[TS_SYNC_BYTE, 0x1F, 0xFF, 0x10]);
        packet.repeat(3)
    };
    assert!(accepted(&nulls), "only null packets");
    let mut second_lane_broken = whole[..2 * TS_PACKET_LEN].to_vec();
    second_lane_broken[TS_PACKET_LEN] = 0;
    assert!(!accepted(&second_lane_broken), "2 lanes, one without sync");
    let mut one_lane_late = vec![0u8; 3];
    one_lane_late.extend_from_slice(&whole[..TS_PACKET_LEN]);
    assert!(
        !accepted(&one_lane_late),
        "one lane is only trusted at phase 0: any of ~188 phases could match noise"
    );
}

#[test]
fn a_mid_packet_cut_ts_segment_plays_like_the_aligned_one() {
    let whole = ts("index0.ts");
    let aligned = samples_of(&whole);
    let cut = samples_of(&whole[MID_PACKET_CUT..]);
    assert!(cut > 0, "the cut segment yields samples");
    assert!(
        aligned - cut <= 2,
        "aligned {aligned}, cut {cut}: only the partial first packet is lost"
    );
}

#[test]
fn random_bytes_at_any_size_are_never_ts() {
    for len in [0usize, 1, 187, 188, 189, 375, 376, 377, 564, 4096, 65_536] {
        let body: Vec<u8> = (0..len)
            .map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 13 & 0xFF).unwrap())
            .collect();
        assert!(!accepted(&body), "{len} random bytes");
    }
    assert!(!accepted(b"<html>x</html>"));
}

fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = u32::try_from(8 + body.len())
        .unwrap()
        .to_be_bytes()
        .to_vec();
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

fn fmp4_client() -> HlsClient {
    let pl = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
              #EXT-X-MAP:URI=\"i.mp4\"\n#EXTINF:1.0,\ns0.m4s\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(URL);
    let _ = fetches(&mut client);
    client.on_playlist(pl.as_bytes()).unwrap();
    let _ = fetches(&mut client);
    client
}

#[test]
fn fmp4_fragments_after_any_leading_boxes_or_in_pairs_are_media() {
    let moof = mp4_box(b"moof", &[]);
    let mdat = mp4_box(b"mdat", &[0; 4]);
    for (name, lead) in [
        ("styp", vec![mp4_box(b"styp", b"msdh")]),
        (
            "styp prft",
            vec![mp4_box(b"styp", b"msdh"), mp4_box(b"prft", &[0; 24])],
        ),
        ("emsg", vec![mp4_box(b"emsg", &[0; 12])]),
        ("free", vec![mp4_box(b"free", &[0; 3])]),
        ("sidx", vec![mp4_box(b"sidx", &[0; 20])]),
        ("none", vec![]),
    ] {
        let mut body: Vec<u8> = lead.concat();
        body.extend_from_slice(&moof);
        body.extend_from_slice(&mdat);
        let mut client = fmp4_client();
        let r = client.on_resource(SEG0, &body);
        assert!(!matches!(r, Err(Error::NotMedia { .. })), "{name}: {r:?}");
    }
    let mut pairs = Vec::new();
    for _ in 0..3 {
        pairs.extend_from_slice(&moof);
        pairs.extend_from_slice(&mdat);
    }
    let r = fmp4_client().on_resource(SEG0, &pairs);
    assert!(!matches!(r, Err(Error::NotMedia { .. })), "pairs: {r:?}");
    for (name, bad) in [
        ("moof only", moof.clone()),
        ("mdat before moof", [mdat.clone(), moof.clone()].concat()),
        (
            "moof then a truncated mdat",
            [moof.clone(), mdat[..mdat.len() - 1].to_vec()].concat(),
        ),
    ] {
        let r = fmp4_client().on_resource(SEG0, &bad);
        assert!(matches!(r, Err(Error::NotMedia { .. })), "{name}: {r:?}");
    }
}
