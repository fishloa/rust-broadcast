//! Gate tests for HLS playlist output.
//!
//! Validates the generated `#EXTM3U` against the `media_doctor::check_playlist`
//! RFC-8216 validator and against structural invariants.

use transmux::{CencScheme, MasterPlaylist, MediaPlaylist, MediaSegment, Variant, cenc_ext_x_key};

#[test]
fn media_playlist_rfc_valid() {
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg0.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg1.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg2.m4s".into(),
                duration: 3.003,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
        ],
        endlist: true,
        extra_tags: vec![
            "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                .into(),
        ],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8();

    // Structural assertions.
    assert!(m3u8.starts_with("#EXTM3U\n"), "must start with #EXTM3U");
    assert_eq!(
        m3u8.matches("#EXTINF:").count(),
        3,
        "must have exactly 3 #EXTINF: lines"
    );
    assert!(
        m3u8.ends_with("#EXT-X-ENDLIST\n"),
        "must end with #EXT-X-ENDLIST"
    );

    // RFC-8216 validation via media-doctor.
    let mut report = media_doctor::Report::new();
    media_doctor::check_playlist(&m3u8, &mut report);
    assert!(
        report.is_empty(),
        "media playlist must be RFC-valid but got: {report}",
    );
}

#[test]
fn media_playlist_invalid_target_duration_reported() {
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "long.m4s".into(),
            duration: 15.0,
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }],
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8();

    let mut report = media_doctor::Report::new();
    media_doctor::check_playlist(&m3u8, &mut report);
    assert!(
        !report.is_empty(),
        "segment duration > target_duration should produce findings"
    );
}

#[test]
fn master_playlist_structure() {
    let pl = MasterPlaylist {
        version: 6,
        variants: vec![
            Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e,mp4a.40.2".into(),
                resolution: Some((640, 360)),
                uri: "v300/index.m3u8".into(),
            },
            Variant {
                bandwidth: 800_000,
                codecs: "avc1.640028,mp4a.40.2".into(),
                resolution: Some((1280, 720)),
                uri: "v800/index.m3u8".into(),
            },
        ],
        iframe_variants: vec![],
    };

    let m3u8 = pl.to_m3u8();

    assert!(m3u8.starts_with("#EXTM3U"), "must start with #EXTM3U");
    assert_eq!(
        m3u8.matches("#EXT-X-STREAM-INF:").count(),
        2,
        "must have exactly 2 EXT-X-STREAM-INF lines"
    );
    assert!(
        m3u8.contains("v300/index.m3u8"),
        "must contain first variant URI"
    );
    assert!(
        m3u8.contains("v800/index.m3u8"),
        "must contain second variant URI"
    );
    assert!(
        m3u8.contains("RESOLUTION=640x360"),
        "must contain first resolution"
    );
    assert!(
        m3u8.contains("RESOLUTION=1280x720"),
        "must contain second resolution"
    );
}

// ---------------------------------------------------------------------------
// CENC/CBCS HLS signalling (issue #564): #EXT-X-KEY for cbcs, none for cenc.
// ---------------------------------------------------------------------------

const TEST_KID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];

#[test]
fn cbcs_emits_ext_x_key_sample_aes() {
    let tag = cenc_ext_x_key(
        CencScheme::Cbcs,
        &TEST_KID,
        "https://keyserver.example.com/key",
    )
    .expect("cbcs must emit an EXT-X-KEY tag");
    assert_eq!(
        tag,
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://keyserver.example.com/key\",\
         KEYFORMAT=\"urn:mpeg:dash:mp4protection:2011\",KEYFORMATVERSIONS=\"1\",\
         KEYID=0x0123456789abcdef0123456789abcdef"
    );

    // Wired into a real playlist via `extra_tags` (the established hook for
    // arbitrary tag lines, e.g. #EXT-X-DATERANGE) renders before the segments.
    let pl = MediaPlaylist {
        version: 6,
        target_duration: 6,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg0.m4s".into(),
            duration: 6.0,
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }],
        endlist: true,
        extra_tags: vec![tag],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8();
    let key_pos = m3u8.find("#EXT-X-KEY:").expect("EXT-X-KEY line present");
    let extinf_pos = m3u8.find("#EXTINF:").expect("EXTINF line present");
    assert!(
        key_pos < extinf_pos,
        "#EXT-X-KEY must precede the segments it protects"
    );
    assert_eq!(m3u8.matches("#EXT-X-KEY:").count(), 1);
}

#[test]
fn cenc_ctr_emits_no_ext_x_key() {
    // `cenc` (AES-CTR) is not a valid HLS METHOD — DASH-only.
    assert_eq!(
        cenc_ext_x_key(
            CencScheme::Cenc,
            &TEST_KID,
            "https://keyserver.example.com/key"
        ),
        None,
        "cenc (CTR) must not produce an HLS EXT-X-KEY tag"
    );
}
