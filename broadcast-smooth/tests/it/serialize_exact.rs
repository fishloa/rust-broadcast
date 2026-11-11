//! The serializer writes exactly the model: `parse(to_xml(m)) == m` for
//! hand-built models, including the chunk shapes the ffmpeg fixtures never
//! carry (no `n`, a `t` + `r` run, an entry with only `d`) and the live /
//! protection / text surfaces.

use broadcast_smooth::{C, QualityLevel, SmoothManifest, StreamIndex, StreamType};

fn round_trips(model: &SmoothManifest) {
    let xml = model.to_xml();
    let reparsed = SmoothManifest::parse(&xml).unwrap_or_else(|e| panic!("re-parse: {e}\n{xml}"));
    assert_eq!(&reparsed, model, "model changed:\n{xml}");
    assert_eq!(reparsed.to_xml(), xml, "serialization is not a fixed point");
}

fn quality(four_cc: &str) -> QualityLevel {
    QualityLevel::new(four_cc, 128_000, alloc_bytes(&[0x12, 0x10]))
}

fn alloc_bytes(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

fn video_stream(chunks: Vec<C>) -> StreamIndex {
    StreamIndex::new(
        StreamType::Video,
        "QualityLevels({bitrate})/Fragments(video={start time})",
        vec![quality("H264").with_size(320, 240)],
    )
    .with_counts(chunks.len() as u32, 1)
    .with_chunks(chunks)
}

#[test]
fn chunks_without_n_stay_without_n() {
    let model = SmoothManifest::new(vec![video_stream(vec![
        C::new(20_000_000).at(0),
        C::new(10_000_000),
    ])])
    .with_duration(30_000_000);
    round_trips(&model);
    let xml = model.to_xml();
    assert!(!xml.contains(" n=\""), "no c@n may be invented:\n{xml}");
}

#[test]
fn a_t_and_r_run_is_one_element() {
    let model = SmoothManifest::new(vec![video_stream(vec![
        C::new(5_000_000).at(1_000_000).repeated(3),
    ])])
    .with_duration(20_000_000);
    round_trips(&model);
    let xml = model.to_xml();
    assert_eq!(
        xml.matches("<c ").count(),
        1,
        "one element per chunk:\n{xml}"
    );
    assert!(
        xml.contains(r#"<c t="1000000" d="5000000" r="3"/>"#),
        "{xml}"
    );
}

#[test]
fn a_chunk_with_only_d_round_trips() {
    let model = SmoothManifest::new(vec![video_stream(vec![C::new(7_000_000)])]);
    round_trips(&model);
    assert!(model.to_xml().contains(r#"<c d="7000000"/>"#));
}

#[test]
fn a_live_manifest_keeps_its_live_attributes() {
    let model = SmoothManifest::new(vec![video_stream(vec![
        C::new(20_000_000).at(0).numbered(0),
        C::new(20_000_000).numbered(1),
    ])])
    .with_duration(40_000_000)
    .live(120_000_000, 2);
    round_trips(&model);
    let xml = model.to_xml();
    for attr in [
        r#"IsLive="TRUE""#,
        r#"LookAheadFragmentCount="2""#,
        r#"DVRWindowLength="120000000""#,
    ] {
        assert!(xml.contains(attr), "{attr} missing:\n{xml}");
    }
}

#[test]
fn a_protection_header_round_trips() {
    let model = SmoothManifest::new(vec![video_stream(vec![C::new(20_000_000).at(0)])])
        .with_duration(20_000_000)
        .with_protection("AAABAwBXAFcAVwAB//8A4wAA");
    round_trips(&model);
    assert!(
        model
            .to_xml()
            .contains("<ProtectionHeader>AAABAwBXAFcAVwAB//8A4wAA</ProtectionHeader>")
    );
}

#[test]
fn a_text_stream_round_trips() {
    let model = SmoothManifest::new(vec![
        video_stream(vec![C::new(20_000_000).at(0)]),
        StreamIndex::new(
            StreamType::Text,
            "QualityLevels({bitrate})/Fragments(text={start time})",
            vec![quality("stpp")],
        )
        .with_subtype("SUBT")
        .with_counts(1, 1)
        .with_chunks(vec![C::new(20_000_000).at(0)]),
    ])
    .with_duration(20_000_000);
    round_trips(&model);
    let xml = model.to_xml();
    assert!(xml.contains(r#"Type="text" Subtype="SUBT""#), "{xml}");
}

#[test]
fn a_named_second_stream_of_a_type_round_trips() {
    let model = SmoothManifest::new(vec![
        StreamIndex::new(
            StreamType::Audio,
            "QualityLevels({bitrate})/Fragments(audio={start time})",
            vec![quality("AACL").with_audio(44_100, 2, 16, 0, 255)],
        )
        .with_subtype("")
        .with_counts(1, 1)
        .with_chunks(vec![C::new(20_000_000).at(0)]),
        StreamIndex::new(
            StreamType::Audio,
            "QualityLevels({bitrate})/Fragments(audio_2={start time})",
            vec![quality("AACL").with_audio(48_000, 6, 16, 0, 255)],
        )
        .with_name("audio_2")
        .with_subtype("")
        .with_counts(1, 1)
        .with_chunks(vec![C::new(20_000_000).at(0)]),
    ])
    .with_duration(20_000_000);
    round_trips(&model);
}
