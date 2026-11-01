#![cfg(feature = "std")]
//! `CLOSED-CAPTIONS` declaration in the origin's multivariant playlist
//! (issue #1428; RFC 8216bis sections 4.4.6.1 and 4.4.6.2).

use std::num::NonZeroUsize;

use broadcast_hls::{ClosedCaptionsRef, InstreamId, MasterPlaylist, MediaType};
use hls_runtime::server::{ClosedCaptionDecl, HlsOrigin};
use media_plane::trunk::{Trunk, TrunkConfig};

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn origin(overrides: Vec<ClosedCaptionDecl>, none: bool) -> HlsOrigin {
    let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
    HlsOrigin::builder(trunk)
        .target_duration_secs(4.0)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
        .closed_caption_overrides(overrides)
        .closed_captions_none(none)
        .build()
        .expect("origin builds")
}

fn master(origin: &HlsOrigin) -> MasterPlaylist {
    let text = origin.master_playlist("media.m3u8").expect("master");
    MasterPlaylist::parse(&text).expect("parse")
}

#[test]
fn unscanned_origin_claims_nothing_about_captions() {
    let m = master(&origin(Vec::new(), false));
    assert!(
        m.media
            .iter()
            .all(|r| r.media_type != MediaType::ClosedCaptions)
    );
    assert!(m.variants.iter().all(|v| v.closed_captions.is_none()));
}

#[test]
fn scanned_without_captions_declares_none_on_every_variant() {
    let o = origin(Vec::new(), false);
    o.set_detected_closed_captions(Vec::new());
    let text = o.master_playlist("media.m3u8").expect("master");
    let m = MasterPlaylist::parse(&text).expect("parse");
    assert!(!m.variants.is_empty());
    assert!(
        m.variants
            .iter()
            .all(|v| v.closed_captions == Some(ClosedCaptionsRef::None))
    );
    assert!(text.contains("CLOSED-CAPTIONS=NONE"));
    assert!(
        m.media
            .iter()
            .all(|r| r.media_type != MediaType::ClosedCaptions)
    );
}

#[test]
fn detected_streams_replace_none_with_one_rendition_each() {
    let o = origin(Vec::new(), true);
    o.set_detected_closed_captions(vec![
        InstreamId::Cc(1),
        InstreamId::Cc(3),
        InstreamId::Service(1),
    ]);
    let text = o.master_playlist("media.m3u8").expect("master");
    let m = MasterPlaylist::parse(&text).expect("parse");
    let ids: Vec<_> = m
        .media
        .iter()
        .filter(|r| r.media_type == MediaType::ClosedCaptions)
        .map(|r| r.instream_id.clone().expect("instream id"))
        .collect();
    assert_eq!(
        ids,
        [InstreamId::Cc(1), InstreamId::Cc(3), InstreamId::Service(1)]
    );
    assert!(
        m.variants
            .iter()
            .all(|v| v.closed_captions == Some(ClosedCaptionsRef::Group("cc".into())))
    );
    assert!(!text.contains("CLOSED-CAPTIONS=NONE"));
    assert!(text.contains(
        "#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"cc\",NAME=\"CC3\",DEFAULT=NO,AUTOSELECT=NO,INSTREAM-ID=\"CC3\""
    ));
}

#[test]
fn a_later_detection_flips_none_to_the_group() {
    let o = origin(Vec::new(), false);
    o.set_detected_closed_captions(Vec::new());
    assert!(
        o.master_playlist("media.m3u8")
            .expect("master")
            .contains("CLOSED-CAPTIONS=NONE")
    );
    o.set_detected_closed_captions(vec![InstreamId::Cc(2)]);
    let text = o.master_playlist("media.m3u8").expect("master");
    assert!(text.contains("CLOSED-CAPTIONS=\"cc\""));
    assert!(text.contains("INSTREAM-ID=\"CC2\""));
    assert!(!text.contains("CLOSED-CAPTIONS=NONE"));
}

#[test]
fn override_names_and_languages_apply_only_to_detected_streams() {
    let decl = |id: &str, name: &str, lang: &str| ClosedCaptionDecl {
        instream_id: id.into(),
        name: name.into(),
        language: lang.into(),
    };
    let o = origin(
        vec![decl("CC1", "English", "en"), decl("CC4", "Francais", "fr")],
        false,
    );
    o.set_detected_closed_captions(vec![InstreamId::Cc(1)]);
    let m = master(&o);
    let cc: Vec<_> = m
        .media
        .iter()
        .filter(|r| r.media_type == MediaType::ClosedCaptions)
        .collect();
    assert_eq!(cc.len(), 1);
    assert_eq!(cc[0].name, "English");
    assert_eq!(cc[0].language.as_deref(), Some("en"));
}

fn languages(m: &MasterPlaylist) -> Vec<(InstreamId, Option<String>)> {
    m.media
        .iter()
        .filter(|r| r.media_type == MediaType::ClosedCaptions)
        .map(|r| {
            (
                r.instream_id.clone().expect("instream id"),
                r.language.clone(),
            )
        })
        .collect()
}

#[test]
fn stream_declared_languages_land_on_the_matching_renditions() {
    let o = origin(Vec::new(), false);
    o.set_detected_closed_captions(vec![
        InstreamId::Cc(1),
        InstreamId::Service(1),
        InstreamId::Service(2),
    ]);
    o.set_declared_closed_caption_languages(vec![
        (InstreamId::Cc(1), "en".into()),
        (InstreamId::Service(2), "es".into()),
    ]);
    assert_eq!(
        languages(&master(&o)),
        [
            (InstreamId::Cc(1), Some("en".into())),
            (InstreamId::Service(1), None),
            (InstreamId::Service(2), Some("es".into())),
        ]
    );
}

#[test]
fn no_declared_language_means_no_language_attribute() {
    let o = origin(Vec::new(), false);
    o.set_detected_closed_captions(vec![InstreamId::Cc(1)]);
    let text = o.master_playlist("media.m3u8").expect("master");
    let line = text
        .lines()
        .find(|l| l.contains("TYPE=CLOSED-CAPTIONS"))
        .expect("rendition");
    assert!(!line.contains("LANGUAGE"));
}

#[test]
fn override_language_wins_over_the_stream_declared_one() {
    let o = origin(
        vec![ClosedCaptionDecl {
            instream_id: "CC1".into(),
            name: String::new(),
            language: "fr".into(),
        }],
        false,
    );
    o.set_detected_closed_captions(vec![InstreamId::Cc(1), InstreamId::Cc(3)]);
    o.set_declared_closed_caption_languages(vec![
        (InstreamId::Cc(1), "en".into()),
        (InstreamId::Cc(3), "de".into()),
    ]);
    assert_eq!(
        languages(&master(&o)),
        [
            (InstreamId::Cc(1), Some("fr".into())),
            (InstreamId::Cc(3), Some("de".into())),
        ]
    );
}
