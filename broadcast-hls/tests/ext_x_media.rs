//! Typed `#EXT-X-MEDIA` (RFC 8216bis §4.4.6.1, `docs/playlist-tags.md`):
//! parse, render, round-trip, and the spec's MUST/MUST NOT rules.

use broadcast_hls::{ClosedCaptionsRef, InstreamId, MasterPlaylist, MediaType};

const MASTER: &str = "#EXTM3U\n\
#EXT-X-VERSION:7\n\
#EXT-X-MEDIA:TYPE=AUDIO,URI=\"en.m3u8\",GROUP-ID=\"aac\",LANGUAGE=\"en\",NAME=\"English\",DEFAULT=YES,AUTOSELECT=YES,SAMPLE-RATE=48000,BIT-DEPTH=16,CHANNELS=\"2\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,URI=\"fr.m3u8\",GROUP-ID=\"aac\",LANGUAGE=\"fr\",NAME=\"Francais\",DEFAULT=NO,AUTOSELECT=YES\n\
#EXT-X-MEDIA:TYPE=SUBTITLES,URI=\"s.m3u8\",GROUP-ID=\"subs\",LANGUAGE=\"nb\",ASSOC-LANGUAGE=\"no\",NAME=\"Bokmael\",DEFAULT=NO,AUTOSELECT=YES,FORCED=YES,CHARACTERISTICS=\"public.easy-to-read,public.machine-generated\"\n\
#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"cc\",NAME=\"CC1\",DEFAULT=YES,AUTOSELECT=YES,INSTREAM-ID=\"CC1\"\n\
#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"cc\",NAME=\"S9\",DEFAULT=NO,AUTOSELECT=NO,INSTREAM-ID=\"SERVICE9\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"aac\",SUBTITLES=\"subs\",CLOSED-CAPTIONS=\"cc\"\n\
v.m3u8\n";

fn parse_err(extra: &str) -> String {
    let text = format!("#EXTM3U\n{extra}\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n");
    match MasterPlaylist::parse(&text) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("must be rejected: {extra}"),
    }
}

#[test]
fn parses_every_attribute_typed() {
    let p = MasterPlaylist::parse(MASTER).unwrap();
    assert_eq!(p.media.len(), 5);
    assert!(p.extra_tags.is_empty());
    let en = &p.media[0];
    assert_eq!(en.media_type, MediaType::Audio);
    assert_eq!(en.uri.as_deref(), Some("en.m3u8"));
    assert_eq!(en.group_id, "aac");
    assert_eq!(en.language.as_deref(), Some("en"));
    assert_eq!(en.name, "English");
    assert!(en.default && en.autoselect && !en.forced);
    assert_eq!(en.sample_rate, Some(48000));
    assert_eq!(en.bit_depth, Some(16));
    assert_eq!(en.channels.as_deref(), Some("2"));
    let sub = &p.media[2];
    assert_eq!(sub.media_type, MediaType::Subtitles);
    assert_eq!(sub.assoc_language.as_deref(), Some("no"));
    assert!(sub.forced);
    assert_eq!(
        sub.characteristics,
        ["public.easy-to-read", "public.machine-generated"]
    );
    assert_eq!(p.media[3].media_type, MediaType::ClosedCaptions);
    assert_eq!(p.media[3].instream_id, Some(InstreamId::Cc(1)));
    assert_eq!(p.media[4].instream_id, Some(InstreamId::Service(9)));
    let v = &p.variants[0];
    assert_eq!(v.audio.as_deref(), Some("aac"));
    assert_eq!(v.subtitles.as_deref(), Some("subs"));
    assert_eq!(
        v.closed_captions,
        Some(ClosedCaptionsRef::Group("cc".into()))
    );
}

#[test]
fn round_trips_to_equal_document_and_fixed_point() {
    let p = MasterPlaylist::parse(MASTER).unwrap();
    let r1 = p.to_m3u8().unwrap();
    let p2 = MasterPlaylist::parse(&r1).unwrap();
    assert_eq!(p, p2);
    assert_eq!(r1, p2.to_m3u8().unwrap());
    // Rendered strings are quoted exactly where the spec says.
    assert!(r1.contains("TYPE=AUDIO,URI=\"en.m3u8\",GROUP-ID=\"aac\""));
    assert!(r1.contains("INSTREAM-ID=\"SERVICE9\""));
}

#[test]
fn rejects_spec_must_violations_on_parse() {
    assert!(parse_err("#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"g\"").contains("NAME"));
    assert!(parse_err("#EXT-X-MEDIA:TYPE=VIDEO,NAME=\"n\"").contains("GROUP-ID"));
    assert!(parse_err("#EXT-X-MEDIA:GROUP-ID=\"g\",NAME=\"n\"").contains("TYPE"));
    assert!(parse_err("#EXT-X-MEDIA:TYPE=BOGUS,GROUP-ID=\"g\",NAME=\"n\"").contains("TYPE"));
    // CLOSED-CAPTIONS: URI forbidden, INSTREAM-ID required and range-limited.
    assert!(
        parse_err("#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"g\",NAME=\"n\",URI=\"x\",INSTREAM-ID=\"CC1\"")
            .contains("URI")
    );
    assert!(
        parse_err("#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"g\",NAME=\"n\"")
            .contains("INSTREAM-ID")
    );
    for bad in ["CC0", "CC5", "SERVICE0", "SERVICE64", "cc1", "FOO"] {
        parse_err(&format!(
            "#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"g\",NAME=\"n\",INSTREAM-ID=\"{bad}\""
        ));
    }
    // FORCED / BIT-DEPTH / SAMPLE-RATE / CHANNELS only on their types.
    assert!(
        parse_err("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"n\",FORCED=YES")
            .contains("FORCED")
    );
    for attr in ["BIT-DEPTH=16", "SAMPLE-RATE=48000", "CHANNELS=\"2\""] {
        parse_err(&format!(
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"g\",NAME=\"n\",{attr}"
        ));
    }
    // DEFAULT=YES requires AUTOSELECT=YES when AUTOSELECT is present.
    parse_err("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"g\",NAME=\"n\",DEFAULT=YES,AUTOSELECT=NO");
}

#[test]
fn instream_id_boundaries_accepted() {
    for (v, want) in [
        ("CC4", InstreamId::Cc(4)),
        ("SERVICE1", InstreamId::Service(1)),
        ("SERVICE63", InstreamId::Service(63)),
    ] {
        let p = MasterPlaylist::parse(&format!(
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"g\",NAME=\"n\",INSTREAM-ID=\"{v}\"\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n"
        ))
        .unwrap();
        assert_eq!(p.media[0].instream_id, Some(want));
    }
}

#[test]
fn render_rejects_injection_and_invalid_models() {
    let mut p = MasterPlaylist::parse(MASTER).unwrap();
    p.media[0].name = "a\"b".into();
    assert!(p.to_m3u8().is_err());
    let mut p = MasterPlaylist::parse(MASTER).unwrap();
    p.media[0].uri = Some("x\ny".into());
    assert!(p.to_m3u8().is_err());
    let mut p = MasterPlaylist::parse(MASTER).unwrap();
    p.media[3].uri = Some("x.m3u8".into());
    assert!(p.to_m3u8().is_err());
    let mut p = MasterPlaylist::parse(MASTER).unwrap();
    p.media[1].forced = true; // AUDIO
    assert!(p.to_m3u8().is_err());
}
