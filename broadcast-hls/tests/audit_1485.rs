//! Regression tests for audit issue #1485 items 17-19: strict `INSTREAM-ID`
//! grammar, render-time `INSTREAM-ID` validation, and the RFC 8216bis
//! §4.2 / §4.3.2.4 / §4.4.6.1.1 attribute and Rendition Group rules.

use broadcast_hls::{InstreamId, MasterPlaylist, MediaPlaylist};

fn master(media: &str) -> String {
    format!("#EXTM3U\n{media}\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n")
}

const CC: &str = "#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"g\",NAME=\"n\"";

#[test]
fn instream_id_strict_grammar_rejects_signs_and_leading_zeros() {
    for bad in [
        "CC01",
        "CC+1",
        "SERVICE+9",
        "SERVICE09",
        "CC",
        "SERVICE",
        "CC1 ",
        "CC 1",
    ] {
        let r = InstreamId::parse_closed_captions(bad);
        assert!(r.is_err(), "{bad:?} must be rejected, got {r:?}");
    }
    assert_eq!(
        InstreamId::parse_closed_captions("CC1").unwrap(),
        InstreamId::Cc(1)
    );
    assert_eq!(
        InstreamId::parse_closed_captions("SERVICE63").unwrap(),
        InstreamId::Service(63)
    );
}

fn render_with(id: InstreamId, media_type: &str) -> Result<String, broadcast_hls::Error> {
    let line = if media_type == "CLOSED-CAPTIONS" {
        format!("{CC},INSTREAM-ID=\"CC1\"")
    } else {
        format!("#EXT-X-MEDIA:TYPE={media_type},GROUP-ID=\"g\",NAME=\"n\",URI=\"a.m3u8\"")
    };
    let mut p = MasterPlaylist::parse(&master(&line)).unwrap();
    p.media[0].instream_id = Some(id);
    p.to_m3u8()
}

#[test]
fn render_rejects_out_of_range_cc_and_service() {
    for id in [
        InstreamId::Cc(0),
        InstreamId::Cc(5),
        InstreamId::Cc(9),
        InstreamId::Service(0),
        InstreamId::Service(64),
        InstreamId::Service(200),
    ] {
        let r = render_with(id.clone(), "CLOSED-CAPTIONS");
        assert!(r.is_err(), "{id:?} must not render, got {r:?}");
    }
    assert!(render_with(InstreamId::Cc(4), "CLOSED-CAPTIONS").is_ok());
    assert!(render_with(InstreamId::Service(63), "CLOSED-CAPTIONS").is_ok());
}

#[test]
fn render_rejects_other_on_closed_captions_and_cc_on_other_types() {
    let r = render_with(InstreamId::Other("x".into()), "CLOSED-CAPTIONS");
    assert!(
        r.is_err(),
        "Other on CLOSED-CAPTIONS must not render, got {r:?}"
    );
    let r = render_with(InstreamId::Cc(1), "AUDIO");
    assert!(r.is_err(), "Cc(1) on AUDIO re-parses as Other, got {r:?}");
    let r = render_with(InstreamId::Service(1), "AUDIO");
    assert!(r.is_err(), "Service(1) on AUDIO must not render, got {r:?}");
    assert!(render_with(InstreamId::Other("x".into()), "AUDIO").is_ok());
}

#[test]
fn two_default_yes_in_one_group_rejected_on_parse() {
    let text = master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"2\",DEFAULT=YES,AUTOSELECT=YES",
    );
    let r = MasterPlaylist::parse(&text);
    assert!(r.is_err(), "two DEFAULT=YES in one group, got {r:?}");
}

#[test]
fn same_group_id_different_type_is_a_different_group() {
    let text = master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES\n\
         #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"a\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"b\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES",
    );
    let p = MasterPlaylist::parse(&text).unwrap();
    assert_eq!(
        p.to_m3u8().unwrap(),
        MasterPlaylist::parse(&p.to_m3u8().unwrap())
            .unwrap()
            .to_m3u8()
            .unwrap()
    );
}

#[test]
fn duplicate_name_in_one_group_rejected_on_parse() {
    let text = master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"same\"\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"same\"",
    );
    let r = MasterPlaylist::parse(&text);
    assert!(r.is_err(), "duplicate NAME in one group, got {r:?}");
}

#[test]
fn group_rules_enforced_on_render() {
    let base = master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"2\"",
    );
    let mut p = MasterPlaylist::parse(&base).unwrap();
    p.media[1].default = true;
    p.media[1].autoselect = true;
    let r = p.to_m3u8();
    assert!(r.is_err(), "two DEFAULT=YES must not render, got {r:?}");
    let mut p = MasterPlaylist::parse(&base).unwrap();
    p.media[1].name = "1".into();
    let r = p.to_m3u8();
    assert!(r.is_err(), "duplicate NAME must not render, got {r:?}");
}

#[test]
fn boolean_attributes_accept_only_exact_yes_and_no() {
    for attr in [
        "DEFAULT=yes",
        "DEFAULT=TRUE",
        "DEFAULT=Yes",
        "AUTOSELECT=maybe",
        "AUTOSELECT=no",
        "FORCED=yes",
        "FORCED=1",
    ] {
        let text = master(&format!(
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"g\",NAME=\"n\",URI=\"s.m3u8\",{attr}"
        ));
        let r = MasterPlaylist::parse(&text);
        assert!(r.is_err(), "{attr} must be rejected, got {r:?}");
    }
    let ok = master(
        "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"g\",NAME=\"n\",URI=\"s.m3u8\",DEFAULT=NO,AUTOSELECT=YES,FORCED=YES",
    );
    let p = MasterPlaylist::parse(&ok).unwrap();
    assert!(!p.media[0].default && p.media[0].autoselect && p.media[0].forced);
}

const IV_HEX: &str = "000102030405060708090a0b0c0d0e0f";

fn session_key(iv: &str) -> String {
    master(&format!(
        "#EXT-X-SESSION-KEY:METHOD=AES-128,URI=\"k.bin\",IV={iv}"
    ))
}

fn segment_key_playlist(iv: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV={iv}\n#EXTINF:4,\ns.ts\n#EXT-X-ENDLIST\n"
    )
}

#[test]
fn iv_requires_hex_prefix() {
    let r = MasterPlaylist::parse(&session_key(IV_HEX));
    assert!(
        r.is_err(),
        "session-key IV without 0x must be rejected, got {r:?}"
    );
    let r = MediaPlaylist::parse(&segment_key_playlist(IV_HEX))
        .and_then(|p| p.segment_keys().map(|_| ()));
    assert!(
        r.is_err(),
        "segment-key IV without 0x must be rejected, got {r:?}"
    );
    for prefix in ["0x", "0X"] {
        assert!(MasterPlaylist::parse(&session_key(&format!("{prefix}{IV_HEX}"))).is_ok());
        let p = MediaPlaylist::parse(&segment_key_playlist(&format!("{prefix}{IV_HEX}"))).unwrap();
        assert!(p.segment_keys().is_ok());
    }
}

#[test]
fn unterminated_quoted_string_rejected() {
    let r = MasterPlaylist::parse(&master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"n\",LANGUAGE=\"en",
    ));
    assert!(r.is_err(), "unterminated quote, got {r:?}");
    let r = MasterPlaylist::parse(&master("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"n"));
    assert!(r.is_err(), "unterminated quote at end, got {r:?}");
}

#[test]
fn duplicate_attribute_names_rejected() {
    let r = MasterPlaylist::parse(&master(
        "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"n\",NAME=\"m\"",
    ));
    assert!(r.is_err(), "duplicate NAME attribute, got {r:?}");
}

#[test]
fn rejections_carry_their_typed_error() {
    use broadcast_hls::{Error, MediaType, parse_attribute_list_strict};

    assert!(matches!(
        InstreamId::parse_closed_captions("CC01"),
        Err(Error::InvalidInstreamId { value }) if value == "CC01"
    ));
    assert!(matches!(
        render_with(InstreamId::Cc(9), "CLOSED-CAPTIONS"),
        Err(Error::InvalidInstreamId { value }) if value == "CC9"
    ));
    assert!(matches!(
        render_with(InstreamId::Cc(1), "AUDIO"),
        Err(Error::InstreamIdTypeMismatch {
            media_type: MediaType::Audio,
            ..
        })
    ));
    assert!(matches!(
        MasterPlaylist::parse(&master(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"1\",DEFAULT=YES,AUTOSELECT=YES\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"2\",DEFAULT=YES,AUTOSELECT=YES"
        )),
        Err(Error::DuplicateDefaultInGroup { media_type: MediaType::Audio, group_id }) if group_id == "a"
    ));
    assert!(matches!(
        MasterPlaylist::parse(&master(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"s\"\n\
             #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"s\""
        )),
        Err(Error::DuplicateNameInGroup { name, .. }) if name == "s"
    ));
    assert!(matches!(
        MasterPlaylist::parse(&master(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"s\",DEFAULT=yes"
        )),
        Err(Error::InvalidYesNo { attribute: "DEFAULT", value }) if value == "yes"
    ));
    assert!(matches!(
        MasterPlaylist::parse(&session_key(IV_HEX)),
        Err(Error::InvalidIv { .. })
    ));
    assert!(matches!(
        parse_attribute_list_strict("A=\"x"),
        Err(Error::UnterminatedQuotedString { .. })
    ));
    assert!(matches!(
        parse_attribute_list_strict("A=1,A=2"),
        Err(Error::DuplicateAttribute { name }) if name == "A"
    ));
}

#[test]
fn strict_tokenizer_matches_lenient_on_well_formed_input() {
    use broadcast_hls::{parse_attribute_list, parse_attribute_list_strict};
    let text = "ID=\"a,b\",DURATION=1.5,X-BARE=plain";
    assert_eq!(
        parse_attribute_list_strict(text).unwrap(),
        parse_attribute_list(text)
    );
}
