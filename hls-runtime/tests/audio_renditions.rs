#![cfg(feature = "std")]
//! Separate audio renditions in the origin's multivariant playlist
//! (issue #1428 slice 1, RFC 8216bis §4.4.6.1-§4.4.6.2).

use std::num::NonZeroUsize;
use std::path::PathBuf;

use broadcast_hls::MasterPlaylist;
use hls_runtime::server::{
    AudioRenditionDecl, HlsMasterError, HlsOrigin, HlsOriginBuildError, HlsOriginBuilder,
};
use media_plane::trunk::{Trunk, TrunkConfig};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn builder() -> HlsOriginBuilder {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
}

fn decl(group: &str, name: &str, lang: &str, default: bool, uri: &str) -> AudioRenditionDecl {
    AudioRenditionDecl {
        group_id: group.into(),
        name: name.into(),
        language: lang.into(),
        default,
        autoselect: true,
        channels: Some("2".into()),
        uri: uri.into(),
        characteristics: Vec::new(),
    }
}

fn two_groups() -> Vec<AudioRenditionDecl> {
    vec![
        decl("aac", "English", "en", true, "audio/aac/en.m3u8"),
        decl("aac", "Deutsch", "de", false, "audio/aac/de.m3u8"),
        decl("ec3", "English", "en", true, "audio/ec3/en.m3u8"),
        decl("ec3", "Deutsch", "de", false, "audio/ec3/de.m3u8"),
    ]
}

fn master_with(decls: Vec<AudioRenditionDecl>, cc_none: bool) -> String {
    builder()
        .audio_renditions(decls)
        .closed_captions_none(cc_none)
        .build()
        .expect("origin builds")
        .master_playlist("media.m3u8")
        .expect("master")
}

fn build_err(decls: Vec<AudioRenditionDecl>) -> HlsOriginBuildError {
    match builder().audio_renditions(decls).build() {
        Ok(_) => panic!("build unexpectedly succeeded"),
        Err(e) => e,
    }
}

#[test]
fn two_group_two_language_master_matches_golden() {
    let text = master_with(two_groups(), true);
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/audio_renditions_master.golden"
    ));
    let golden = std::fs::read_to_string(&path).expect("golden");
    assert_eq!(text, golden);
}

#[test]
fn golden_master_parses_and_reserialises_byte_identically() {
    let text = master_with(two_groups(), true);
    let parsed = MasterPlaylist::parse(&text).expect("parse");
    assert_eq!(parsed.media.len(), 4);
    assert_eq!(parsed.variants.len(), 2);
    let groups: Vec<_> = parsed.variants.iter().map(|v| v.audio.as_deref()).collect();
    assert_eq!(groups, [Some("aac"), Some("ec3")]);
    assert_eq!(parsed.to_m3u8().expect("serialize"), text);
}

#[test]
fn every_declared_group_is_referenced_by_a_variant() {
    let text = master_with(two_groups(), false);
    let parsed = MasterPlaylist::parse(&text).expect("parse");
    for group in ["aac", "ec3"] {
        assert!(
            parsed
                .variants
                .iter()
                .any(|v| v.audio.as_deref() == Some(group)),
            "group {group} unreferenced in:\n{text}"
        );
    }
    assert!(!text.contains("CLOSED-CAPTIONS"));
}

#[test]
fn no_declarations_and_flag_off_keeps_plain_master() {
    let origin = builder().build().expect("builds");
    assert_eq!(
        origin.master_playlist("media.m3u8").expect("master"),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000\nmedia.m3u8\n"
    );
}

#[test]
fn same_name_in_two_groups_is_accepted() {
    assert!(builder().audio_renditions(two_groups()).build().is_ok());
}

#[test]
fn empty_group_rejected() {
    let mut d = two_groups();
    d[0].group_id.clear();
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::EmptyAudioRenditionGroupId
    ));
}

#[test]
fn empty_name_rejected() {
    let mut d = two_groups();
    d[0].name.clear();
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::EmptyAudioRenditionName
    ));
}

#[test]
fn duplicate_name_in_group_rejected() {
    let d = vec![
        decl("aac", "English", "en", true, "a.m3u8"),
        decl("aac", "English", "de", false, "b.m3u8"),
    ];
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::DuplicateAudioRenditionName
    ));
}

#[test]
fn zero_defaults_in_group_rejected() {
    let mut d = two_groups();
    d[0].default = false;
    d[2].default = false;
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::AudioRenditionDefaultCount
    ));
}

#[test]
fn two_defaults_in_group_rejected() {
    let mut d = two_groups();
    d[1].default = true;
    d[3].default = true;
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::AudioRenditionDefaultCount
    ));
}

#[test]
fn empty_language_rejected() {
    let mut d = two_groups();
    d[1].language.clear();
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::EmptyAudioRenditionLanguage
    ));
}

#[test]
fn empty_uri_rejected() {
    let mut d = two_groups();
    d[1].uri.clear();
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::EmptyAudioRenditionUri
    ));
}

#[test]
fn default_without_autoselect_rejected() {
    let mut d = two_groups();
    d[0].autoselect = false;
    d[2].autoselect = false;
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::AudioRenditionDefaultNotAutoselect
    ));
}

#[test]
fn groups_with_different_members_rejected() {
    let d = vec![
        decl("aac", "English", "en", true, "a.m3u8"),
        decl("ec3", "Deutsch", "de", true, "b.m3u8"),
    ];
    assert!(matches!(
        build_err(d),
        HlsOriginBuildError::AudioRenditionGroupsDiffer
    ));
}

#[test]
fn uri_equal_to_media_playlist_name_rejected_at_render() {
    let origin = builder()
        .audio_renditions(vec![decl("aac", "English", "en", true, "media.m3u8")])
        .build()
        .expect("builds");
    assert!(matches!(
        origin.master_playlist("media.m3u8"),
        Err(HlsMasterError::AudioRenditionUriEqualsMediaPlaylist)
    ));
    assert!(origin.master_playlist("other.m3u8").is_ok());
}

#[test]
fn single_group_master_carries_one_variant() {
    let d = vec![
        decl("aac", "English", "en", true, "a.m3u8"),
        decl("aac", "Deutsch", "de", false, "b.m3u8"),
    ];
    let text = master_with(d, false);
    let parsed = MasterPlaylist::parse(&text).expect("parse");
    assert_eq!(parsed.variants.len(), 1);
    assert_eq!(parsed.variants[0].audio.as_deref(), Some("aac"));
}
