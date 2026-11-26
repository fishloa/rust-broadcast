//! The preference text form: the codec token, parse, display and the list.

use broadcast_codec::{CodecId, Direction, Preference, PreferenceError, parse_preferences};

/// Every codec that has a textual token.
const NAMED: &[CodecId] = &[
    CodecId::H264,
    CodecId::Hevc,
    CodecId::Vvc,
    CodecId::Av1,
    CodecId::Vp9,
    CodecId::Mpeg2Video,
    CodecId::Aac,
    CodecId::Ac3,
    CodecId::Eac3,
    CodecId::Ac4,
    CodecId::Dts,
    CodecId::Opus,
    CodecId::Flac,
    CodecId::MpegAudio,
    CodecId::MpegH3dAudio,
    CodecId::Pcm,
];

#[test]
fn from_name_reads_every_name_back() {
    for codec in NAMED {
        assert_eq!(
            CodecId::from_name(codec.name()),
            Some(*codec),
            "{}",
            codec.name()
        );
    }
    assert_eq!(CodecId::from_name("H.264"), Some(CodecId::H264));
    assert_eq!(CodecId::from_name("h264"), Some(CodecId::H264));
    assert_eq!(
        CodecId::from_name("MPEG-2 video"),
        Some(CodecId::Mpeg2Video)
    );
    assert_eq!(CodecId::from_name("mpeg2video"), Some(CodecId::Mpeg2Video));
    assert_eq!(CodecId::from_name(""), None);
    assert_eq!(CodecId::from_name("nope"), None);
    assert_eq!(CodecId::from_name("other"), None);
    assert_eq!(CodecId::from_name(CodecId::Other(7).name()), None);
}

#[test]
fn documented_examples_parse_to_their_preference() {
    assert_eq!(
        "aac=oxideav".parse::<Preference>().expect("aac=oxideav"),
        Preference::new(CodecId::Aac, "oxideav")
    );
    assert_eq!(
        "h264=libav:h264_videotoolbox"
            .parse::<Preference>()
            .expect("h264=libav:h264_videotoolbox"),
        Preference::new(CodecId::H264, "libav").with_implementation("h264_videotoolbox")
    );
    assert_eq!(
        "dec:aac=oxideav"
            .parse::<Preference>()
            .expect("dec:aac=oxideav"),
        Preference::new(CodecId::Aac, "oxideav").with_direction(Direction::Decode)
    );
    assert_eq!(
        "enc:aac=libav"
            .parse::<Preference>()
            .expect("enc:aac=libav"),
        Preference::new(CodecId::Aac, "libav").with_direction(Direction::Encode)
    );
}

#[test]
fn every_malformed_form_names_its_token() {
    let cases: &[(&str, PreferenceError)] = &[
        ("aac", PreferenceError::MissingEquals { text: "aac".into() }),
        (
            "zzz:aac=alpha",
            PreferenceError::UnknownDirection {
                prefix: "zzz".into(),
            },
        ),
        (
            "nope=alpha",
            PreferenceError::UnknownCodec {
                token: "nope".into(),
            },
        ),
        ("aac=", PreferenceError::EmptyProvider),
        ("aac=alpha:", PreferenceError::EmptyImplementation),
        (
            "aac=al pha",
            PreferenceError::InvalidCharacter {
                token: "al pha".into(),
                character: ' ',
            },
        ),
        (
            "aac!=alpha",
            PreferenceError::InvalidCharacter {
                token: "aac!".into(),
                character: '!',
            },
        ),
        (
            "aac=Alpha",
            PreferenceError::InvalidCharacter {
                token: "Alpha".into(),
                character: 'A',
            },
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(text.parse::<Preference>(), Err(expected.clone()), "{text}");
    }
}

#[test]
fn display_writes_the_documented_form() {
    assert_eq!(
        Preference::new(CodecId::Aac, "oxideav").to_string(),
        "aac=oxideav"
    );
    assert_eq!(
        Preference::new(CodecId::H264, "libav")
            .with_implementation("h264_videotoolbox")
            .to_string(),
        "h264=libav:h264_videotoolbox"
    );
    assert_eq!(
        Preference::new(CodecId::Hevc, "alpha")
            .with_direction(Direction::Encode)
            .to_string(),
        "enc:hevc=alpha"
    );
    assert_eq!(
        Preference::new(CodecId::Mpeg2Video, "alpha").to_string(),
        "mpeg2video=alpha"
    );
}

#[test]
fn display_round_trips_over_a_table() {
    let mut table = Vec::new();
    for codec in NAMED {
        table.push(Preference::new(*codec, "alpha"));
        table.push(Preference::new(*codec, "alpha").with_implementation("impl1"));
        table.push(Preference::new(*codec, "alpha").with_direction(Direction::Decode));
        table.push(
            Preference::new(*codec, "beta")
                .with_direction(Direction::Encode)
                .with_implementation("x264"),
        );
    }
    for preference in table {
        let text = preference.to_string();
        assert_eq!(
            text.parse::<Preference>().expect(&text),
            preference,
            "{text}"
        );
        assert_eq!(text, preference.to_string(), "{text}");
    }
}

#[test]
fn a_list_parses_in_order_and_ignores_surrounding_space() {
    let list = parse_preferences("aac=oxideav, enc:h264=libav:h264_videotoolbox")
        .expect("two preferences");
    assert_eq!(
        list,
        [
            Preference::new(CodecId::Aac, "oxideav"),
            Preference::new(CodecId::H264, "libav")
                .with_direction(Direction::Encode)
                .with_implementation("h264_videotoolbox"),
        ]
    );
    assert!(parse_preferences("").expect("an empty list").is_empty());
    assert!(parse_preferences("  ").expect("a blank list").is_empty());
}

#[test]
fn a_blank_entry_in_a_list_is_refused() {
    assert_eq!(
        parse_preferences("aac=oxideav,"),
        Err(PreferenceError::EmptyEntry { entry: 1 })
    );
    assert_eq!(
        parse_preferences("aac=oxideav,,enc:aac=libav"),
        Err(PreferenceError::EmptyEntry { entry: 1 })
    );
}

#[test]
fn the_first_bad_entry_in_a_list_is_the_one_reported() {
    assert_eq!(
        parse_preferences("aac=oxideav,enc:nope=libav"),
        Err(PreferenceError::UnknownCodec {
            token: "nope".into()
        })
    );
}

#[test]
fn other_carries_its_tag_in_the_text_form() {
    assert_eq!(
        CodecId::from_name("other:00000007"),
        Some(CodecId::Other(7))
    );
    assert_eq!(
        CodecId::from_name("other:DEADBEEF"),
        Some(CodecId::Other(0xDEAD_BEEF))
    );

    let preference = Preference::new(CodecId::Other(7), "alpha");
    assert_eq!(preference.to_string(), "other:00000007=alpha");
    assert_eq!(
        "other:00000007=alpha"
            .parse::<Preference>()
            .expect("a tagged preference"),
        preference
    );

    let preference = Preference::new(CodecId::Other(0xDEAD_BEEF), "beta")
        .with_direction(Direction::Encode)
        .with_implementation("impl1");
    let text = preference.to_string();
    assert_eq!(text, "enc:other:deadbeef=beta:impl1");
    assert_eq!(text.parse::<Preference>().expect(&text), preference);

    assert_eq!(
        parse_preferences("other:00000007=alpha,enc:h264=beta").expect("a tagged list"),
        [
            Preference::new(CodecId::Other(7), "alpha"),
            Preference::new(CodecId::H264, "beta").with_direction(Direction::Encode),
        ]
    );
}

#[test]
fn other_alone_stays_an_unknown_codec() {
    assert_eq!(CodecId::from_name("other"), None);
    assert_eq!(
        "other=alpha".parse::<Preference>(),
        Err(PreferenceError::UnknownCodec {
            token: "other".into()
        })
    );
}

#[test]
fn other_malformed_digits_are_an_unknown_codec() {
    for text in [
        "other:",
        "other:7",
        "other:0000007",
        "other:000000007",
        "other:0000000g",
        "other:0x000007",
    ] {
        assert_eq!(CodecId::from_name(text), None, "{text}");
        let preference = format!("{text}=alpha");
        assert_eq!(
            preference.parse::<Preference>(),
            Err(PreferenceError::UnknownCodec { token: text.into() }),
            "{preference}"
        );
    }
}
