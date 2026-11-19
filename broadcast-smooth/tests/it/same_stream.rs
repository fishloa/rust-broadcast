//! `StreamIndex::same_stream` identity across a manifest refresh (MS-SSTR
//! §2.2.3): `Type` + `Name` + `Url` template, so two audio streams are not
//! collapsed onto the first.

use broadcast_smooth::{StreamIndex, StreamType};

fn stream(stream_type: StreamType, name: Option<&str>, url: &str) -> StreamIndex {
    let s = StreamIndex::new(stream_type, url, Vec::new());
    match name {
        Some(name) => s.with_name(name),
        None => s,
    }
}

#[test]
fn same_stream_requires_type_name_and_url_to_match() {
    let base = stream(StreamType::Video, None, "video-url");
    assert!(base.same_stream(&stream(StreamType::Video, None, "video-url")));
    assert!(!base.same_stream(&stream(StreamType::Audio, None, "video-url")));
    assert!(!base.same_stream(&stream(StreamType::Video, Some("main"), "video-url")));
    assert!(!base.same_stream(&stream(StreamType::Video, None, "other-url")));
}

#[test]
fn two_audio_streams_of_the_same_type_are_distinct_by_name() {
    let en = stream(StreamType::Audio, Some("audio-en"), "audio-url");
    let fr = stream(StreamType::Audio, Some("audio-fr"), "audio-url");
    assert!(!en.same_stream(&fr));
    assert!(!fr.same_stream(&en));
}
