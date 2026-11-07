#![cfg(feature = "std")]
//! The four rendition validators (RFC 8216bis §4.4.6.1) called directly: every
//! `HlsOriginBuildError` variant they can return, in the order they are checked.

use hls_runtime::server::{
    AudioRenditionDecl, ClosedCaptionDecl, HlsOriginBuildError, SubtitleRenditionDecl,
    validate_audio_renditions, validate_audio_renditions_relaxed,
    validate_closed_caption_overrides, validate_subtitle_renditions,
};

fn audio(group: &str, name: &str, language: &str, default: bool) -> AudioRenditionDecl {
    AudioRenditionDecl {
        group_id: group.into(),
        name: name.into(),
        language: language.into(),
        default,
        autoselect: true,
        channels: None,
        uri: format!("{group}-{name}.m3u8"),
        characteristics: Vec::new(),
    }
}

fn valid_audio() -> Vec<AudioRenditionDecl> {
    vec![
        audio("aac", "English", "en", true),
        audio("aac", "Deutsch", "de", false),
    ]
}

#[test]
fn a_well_formed_audio_group_validates_strictly_and_relaxed() {
    assert_eq!(validate_audio_renditions(&valid_audio()), Ok(()));
    assert_eq!(validate_audio_renditions_relaxed(&valid_audio()), Ok(()));
    assert_eq!(validate_audio_renditions(&[]), Ok(()));
}

fn expect_audio(mutate: impl FnOnce(&mut Vec<AudioRenditionDecl>), want: HlsOriginBuildError) {
    let mut decls = valid_audio();
    mutate(&mut decls);
    assert_eq!(validate_audio_renditions(&decls), Err(want));
}

#[test]
fn strict_audio_validation_reports_each_rule() {
    expect_audio(|d| d[1].group_id.clear(), HlsOriginBuildError::EmptyAudioRenditionGroupId);
    expect_audio(|d| d[1].name.clear(), HlsOriginBuildError::EmptyAudioRenditionName);
    expect_audio(|d| d[1].language.clear(), HlsOriginBuildError::EmptyAudioRenditionLanguage);
    expect_audio(|d| d[1].uri.clear(), HlsOriginBuildError::EmptyAudioRenditionUri);
    expect_audio(
        |d| d[0].autoselect = false,
        HlsOriginBuildError::AudioRenditionDefaultNotAutoselect,
    );
    expect_audio(
        |d| d[1].name = "English".into(),
        HlsOriginBuildError::DuplicateAudioRenditionName,
    );
    expect_audio(|d| d[1].default = true, HlsOriginBuildError::AudioRenditionDefaultCount);
    expect_audio(|d| d[0].default = false, HlsOriginBuildError::AudioRenditionDefaultCount);
}

#[test]
fn audio_groups_must_have_identical_members() {
    let mut decls = valid_audio();
    decls.push(audio("ec3", "English", "en", true));
    assert_eq!(
        validate_audio_renditions(&decls),
        Err(HlsOriginBuildError::AudioRenditionGroupsDiffer)
    );
    let mut same = valid_audio();
    same.push(audio("ec3", "English", "en", true));
    same.push(audio("ec3", "Deutsch", "de", false));
    assert_eq!(validate_audio_renditions(&same), Ok(()));
}

#[test]
fn relaxed_audio_validation_skips_the_name_and_language_rules_only() {
    let mut decls = valid_audio();
    decls[1].name.clear();
    decls[1].language.clear();
    assert_eq!(validate_audio_renditions_relaxed(&decls), Ok(()));
    assert_eq!(
        validate_audio_renditions(&decls),
        Err(HlsOriginBuildError::EmptyAudioRenditionName)
    );

    let mut no_group = valid_audio();
    no_group[1].group_id.clear();
    assert_eq!(
        validate_audio_renditions_relaxed(&no_group),
        Err(HlsOriginBuildError::EmptyAudioRenditionGroupId)
    );
    let mut no_uri = valid_audio();
    no_uri[0].uri.clear();
    assert_eq!(
        validate_audio_renditions_relaxed(&no_uri),
        Err(HlsOriginBuildError::EmptyAudioRenditionUri)
    );
    let mut not_auto = valid_audio();
    not_auto[0].autoselect = false;
    assert_eq!(
        validate_audio_renditions_relaxed(&not_auto),
        Err(HlsOriginBuildError::AudioRenditionDefaultNotAutoselect)
    );
    let mut two_defaults = valid_audio();
    two_defaults[1].default = true;
    assert_eq!(
        validate_audio_renditions_relaxed(&two_defaults),
        Err(HlsOriginBuildError::AudioRenditionDefaultCount)
    );
    let mut dup_names = valid_audio();
    dup_names[1].name = "English".into();
    assert_eq!(validate_audio_renditions_relaxed(&dup_names), Ok(()));
}

fn subtitle(name: &str, language: &str, uri: &str) -> SubtitleRenditionDecl {
    SubtitleRenditionDecl::new(name, language, uri)
}

#[test]
fn subtitle_validation_reports_each_rule() {
    let ok = vec![subtitle("English", "en", "en.m3u8"), subtitle("Deutsch", "de", "de.m3u8")];
    assert_eq!(validate_subtitle_renditions(&ok), Ok(()));
    assert_eq!(validate_subtitle_renditions(&[]), Ok(()));

    assert_eq!(
        validate_subtitle_renditions(&[subtitle("", "en", "en.m3u8")]),
        Err(HlsOriginBuildError::EmptySubtitleRenditionName)
    );
    assert_eq!(
        validate_subtitle_renditions(&[subtitle("English", "", "en.m3u8")]),
        Err(HlsOriginBuildError::EmptySubtitleRenditionLanguage)
    );
    assert_eq!(
        validate_subtitle_renditions(&[subtitle("English", "en", "")]),
        Err(HlsOriginBuildError::EmptySubtitleRenditionUri)
    );
    assert_eq!(
        validate_subtitle_renditions(&[
            subtitle("English", "en", "a.m3u8"),
            subtitle("English", "de", "b.m3u8")
        ]),
        Err(HlsOriginBuildError::DuplicateSubtitleRenditionName)
    );
    assert_eq!(
        validate_subtitle_renditions(&[
            subtitle("English", "en", "a.m3u8"),
            subtitle("Deutsch", "de", "a.m3u8")
        ]),
        Err(HlsOriginBuildError::DuplicateSubtitleRenditionUri)
    );
}

#[test]
fn subtitle_default_rules() {
    let mut a = subtitle("English", "en", "en.m3u8");
    a.default = true;
    let mut b = subtitle("Deutsch", "de", "de.m3u8");
    b.default = true;
    assert_eq!(
        validate_subtitle_renditions(&[a.clone(), b]),
        Err(HlsOriginBuildError::MultipleDefaultSubtitleRenditions)
    );
    assert_eq!(validate_subtitle_renditions(std::slice::from_ref(&a)), Ok(()));
    a.autoselect = false;
    assert_eq!(
        validate_subtitle_renditions(&[a]),
        Err(HlsOriginBuildError::SubtitleRenditionDefaultNotAutoselect)
    );
}

fn cc(id: &str) -> ClosedCaptionDecl {
    ClosedCaptionDecl {
        instream_id: id.into(),
        ..ClosedCaptionDecl::default()
    }
}

#[test]
fn closed_caption_overrides_must_be_well_formed_and_unique() {
    assert_eq!(validate_closed_caption_overrides(&[]), Ok(()));
    assert_eq!(
        validate_closed_caption_overrides(&[cc("CC1"), cc("CC4"), cc("SERVICE63")]),
        Ok(())
    );
    for bad in ["CC5", "cc1", "SERVICE0", "SERVICE64", "", "CC"] {
        assert_eq!(
            validate_closed_caption_overrides(&[cc(bad)]),
            Err(HlsOriginBuildError::InvalidClosedCaptionInstreamId),
            "{bad:?}"
        );
    }
    assert_eq!(
        validate_closed_caption_overrides(&[cc("CC1"), cc("CC2"), cc("CC1")]),
        Err(HlsOriginBuildError::DuplicateClosedCaptionInstreamId)
    );
}
