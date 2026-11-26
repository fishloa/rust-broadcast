//! The typed profile key: its text form, the codec it belongs to, and the
//! registry queries built on it.

use std::num::NonZeroU32;

use broadcast_codec::{
    CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProfile, CodecProvider, Decoder,
    Direction, Encoder, EncoderConfig, Preference, ProfileError, ProviderInfo, Registry, Timescale,
};

/// Sample rate the fake requests use.
const SAMPLE_RATE: u32 = 48_000;

/// Every named profile, one line per variant.
const EVERY_NAMED: &[CodecProfile] = &[
    CodecProfile::H264Baseline,
    CodecProfile::H264ConstrainedBaseline,
    CodecProfile::H264Main,
    CodecProfile::H264Extended,
    CodecProfile::H264High,
    CodecProfile::H264High10,
    CodecProfile::H264High422,
    CodecProfile::H264High444Predictive,
    CodecProfile::HevcMain,
    CodecProfile::HevcMain10,
    CodecProfile::HevcMainStillPicture,
    CodecProfile::HevcRext,
    CodecProfile::VvcMain10,
    CodecProfile::VvcMain10FourFourFour,
    CodecProfile::Av1Main,
    CodecProfile::Av1High,
    CodecProfile::Av1Professional,
    CodecProfile::Vp9Profile0,
    CodecProfile::Vp9Profile1,
    CodecProfile::Vp9Profile2,
    CodecProfile::Vp9Profile3,
    CodecProfile::Mpeg2Simple,
    CodecProfile::Mpeg2Main,
    CodecProfile::Mpeg2High,
    CodecProfile::Mpeg2FourTwoTwo,
    CodecProfile::AacLc,
    CodecProfile::AacHe,
    CodecProfile::AacHeV2,
    CodecProfile::AacLd,
    CodecProfile::AacEld,
    CodecProfile::AacXhe,
];

/// The profiles the codecs are grouped by, so a variant cannot be counted in
/// two codecs.
const GROUPS: &[(CodecId, &[CodecProfile])] = &[
    (
        CodecId::H264,
        &[
            CodecProfile::H264Baseline,
            CodecProfile::H264ConstrainedBaseline,
            CodecProfile::H264Main,
            CodecProfile::H264Extended,
            CodecProfile::H264High,
            CodecProfile::H264High10,
            CodecProfile::H264High422,
            CodecProfile::H264High444Predictive,
        ],
    ),
    (
        CodecId::Hevc,
        &[
            CodecProfile::HevcMain,
            CodecProfile::HevcMain10,
            CodecProfile::HevcMainStillPicture,
            CodecProfile::HevcRext,
        ],
    ),
    (
        CodecId::Vvc,
        &[CodecProfile::VvcMain10, CodecProfile::VvcMain10FourFourFour],
    ),
    (
        CodecId::Av1,
        &[
            CodecProfile::Av1Main,
            CodecProfile::Av1High,
            CodecProfile::Av1Professional,
        ],
    ),
    (
        CodecId::Vp9,
        &[
            CodecProfile::Vp9Profile0,
            CodecProfile::Vp9Profile1,
            CodecProfile::Vp9Profile2,
            CodecProfile::Vp9Profile3,
        ],
    ),
    (
        CodecId::Mpeg2Video,
        &[
            CodecProfile::Mpeg2Simple,
            CodecProfile::Mpeg2Main,
            CodecProfile::Mpeg2High,
            CodecProfile::Mpeg2FourTwoTwo,
        ],
    ),
    (
        CodecId::Aac,
        &[
            CodecProfile::AacLc,
            CodecProfile::AacHe,
            CodecProfile::AacHeV2,
            CodecProfile::AacLd,
            CodecProfile::AacEld,
            CodecProfile::AacXhe,
        ],
    ),
];

/// One implementation the fake provider offers.
type Offer = (&'static str, CodecId, Direction, &'static [CodecProfile]);

/// What the `alpha` provider offers: two H.264 decoders that overlap on Main,
/// an H.264 encoder, and an AAC decoder.
const ALPHA_OFFERS: &[Offer] = &[
    (
        "h264_a",
        CodecId::H264,
        Direction::Decode,
        &[CodecProfile::H264High, CodecProfile::H264Main],
    ),
    (
        "h264_b",
        CodecId::H264,
        Direction::Decode,
        &[CodecProfile::H264Main, CodecProfile::H264High10],
    ),
    (
        "h264_enc",
        CodecId::H264,
        Direction::Encode,
        &[CodecProfile::H264Baseline],
    ),
    (
        "aac",
        CodecId::Aac,
        Direction::Decode,
        &[CodecProfile::AacLc],
    ),
];

/// What the `beta` provider offers: one H.264 decoder, overlapping alpha on
/// Main.
const BETA_OFFERS: &[Offer] = &[
    (
        "h264",
        CodecId::H264,
        Direction::Decode,
        &[CodecProfile::H264Main, CodecProfile::H264High422],
    ),
    (
        "aac",
        CodecId::Aac,
        Direction::Decode,
        &[CodecProfile::AacLc, CodecProfile::AacHe],
    ),
];

struct Fake {
    name: &'static str,
    offers: &'static [Offer],
}

impl CodecProvider for Fake {
    fn name(&self) -> &str {
        self.name
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(self.name, "1.0.0")
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        self.offers
            .iter()
            .map(|(implementation, codec, direction, profiles)| {
                CodecDescriptor::new(self.name, *implementation, *codec, *direction)
                    .with_profiles(profiles.iter().copied())
                    .expect("a fake states only its own codec's profiles")
            })
            .collect()
    }

    fn make_decoder(
        &self,
        _implementation: &str,
        _parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        Err(CodecError::unsupported("this fake builds no decoder"))
    }

    fn make_encoder(
        &self,
        _implementation: &str,
        _config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported("this fake builds no encoder"))
    }
}

fn registry() -> Registry {
    Registry::new()
        .with(Box::new(Fake {
            name: "alpha",
            offers: ALPHA_OFFERS,
        }))
        .expect("alpha registers")
        .with(Box::new(Fake {
            name: "beta",
            offers: BETA_OFFERS,
        }))
        .expect("beta registers")
}

fn timescale() -> Timescale {
    Timescale::from_nonzero(NonZeroU32::new(SAMPLE_RATE).expect("non-zero sample rate"))
}

#[test]
fn every_named_profile_round_trips_its_token() {
    assert_eq!(EVERY_NAMED.len(), 31, "a variant was added or removed");
    let mut tokens = Vec::new();
    for profile in EVERY_NAMED {
        let text = profile.token();
        assert_eq!(CodecProfile::from_token(&text), Some(*profile), "{text}");

        let (codec, part) = text.split_once('/').expect("a codec and a profile token");
        assert_eq!(CodecId::from_name(codec), Some(profile.codec()), "{text}");
        assert_eq!(part, profile.profile_token(), "{text}");
        assert!(
            part.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{text}"
        );
        assert!(!tokens.contains(&text), "{text} is not unique");
        tokens.push(text);
    }
}

#[test]
fn every_variant_is_counted_in_exactly_one_codec_group() {
    let mut counted = Vec::new();
    for (codec, profiles) in GROUPS {
        assert!(!profiles.is_empty(), "{} has no profiles", codec.name());
        for profile in *profiles {
            assert_eq!(profile.codec(), *codec, "{}", profile.name());
            assert!(
                !counted.contains(profile),
                "{} is counted twice",
                profile.name()
            );
            counted.push(*profile);
        }
    }
    assert_eq!(counted.len(), EVERY_NAMED.len(), "a variant is in no group");
}

#[test]
fn a_named_profile_reads_case_and_punctuation_loosely() {
    assert_eq!(
        CodecProfile::from_token("h264/high 10"),
        Some(CodecProfile::H264High10)
    );
    assert_eq!(
        CodecProfile::from_token("H.264/Constrained Baseline"),
        Some(CodecProfile::H264ConstrainedBaseline)
    );
    assert_eq!(
        CodecProfile::from_token("mpeg2video/4:2:2"),
        Some(CodecProfile::Mpeg2FourTwoTwo)
    );
    assert_eq!(CodecProfile::from_token("h264"), None);
    assert_eq!(CodecProfile::from_token("h264/he"), None);
    assert_eq!(CodecProfile::from_token("nope/high"), None);
    assert_eq!(CodecProfile::from_token("h264/"), None);
}

#[test]
fn other_round_trips_its_codec_and_tag() {
    let profile = CodecProfile::Other {
        codec: CodecId::Other(7),
        tag: 0xDEAD_BEEF,
    };
    assert_eq!(profile.codec(), CodecId::Other(7));
    assert_eq!(profile.profile_token(), "deadbeef");
    assert_eq!(profile.token(), "other:00000007/deadbeef");
    assert_eq!(
        CodecProfile::from_token("other:00000007/deadbeef"),
        Some(profile)
    );
    for text in [
        "other:00000007/beef",
        "other:00000007/0xdeadbeef",
        "other:00000007/",
    ] {
        assert_eq!(CodecProfile::from_token(text), None, "{text}");
    }
}

#[test]
fn every_builder_refuses_another_codecs_profile() {
    let expected: Result<(), ProfileError> = Err(ProfileError::Mismatch {
        profile: CodecProfile::H264High,
        codec: CodecId::Aac,
    });
    assert_eq!(
        CodecDescriptor::new("alpha", "aac", CodecId::Aac, Direction::Decode)
            .with_profiles([CodecProfile::H264High])
            .map(|_| ()),
        expected
    );
    assert_eq!(
        CodecParameters::new(CodecId::Aac, timescale())
            .with_profile(CodecProfile::H264High)
            .map(|_| ()),
        expected
    );
    assert_eq!(
        EncoderConfig::new(CodecId::Aac, timescale())
            .with_profile(CodecProfile::H264High)
            .map(|_| ()),
        expected
    );
    assert_eq!(
        Preference::new(CodecId::Aac, "alpha")
            .with_profile(CodecProfile::H264High)
            .map(|_| ()),
        expected
    );
}

#[test]
fn the_profile_error_names_the_profile_and_the_codec() {
    let error = CodecParameters::new(CodecId::Aac, timescale())
        .with_profile(CodecProfile::H264High)
        .expect_err("an H.264 profile is not an AAC one");
    assert_eq!(
        error.to_string(),
        "profile `h264/high` is not a profile of AAC"
    );
}

#[test]
fn stated_profiles_read_back_in_order_and_default_to_unstated() {
    let descriptor = CodecDescriptor::new("alpha", "h264", CodecId::H264, Direction::Decode)
        .with_profiles([CodecProfile::H264Main, CodecProfile::H264High])
        .expect("an H.264 descriptor states H.264 profiles");
    assert_eq!(
        descriptor.profiles(),
        [CodecProfile::H264Main, CodecProfile::H264High]
    );

    let parameters = CodecParameters::new(CodecId::H264, timescale());
    assert_eq!(parameters.profile(), None);
    let parameters = parameters
        .with_profile(CodecProfile::H264Main)
        .expect("an H.264 profile");
    assert_eq!(parameters.profile(), Some(CodecProfile::H264Main));

    let config = EncoderConfig::new(CodecId::H264, timescale());
    assert_eq!(config.profile(), None);
    let config = config
        .with_profile(CodecProfile::H264High)
        .expect("an H.264 profile");
    assert_eq!(config.profile(), Some(CodecProfile::H264High));
}

#[test]
fn a_providers_profiles_are_the_first_seen_union_without_duplicates() {
    let provider = Fake {
        name: "alpha",
        offers: ALPHA_OFFERS,
    };
    assert_eq!(
        provider.profiles(CodecId::H264, Direction::Decode),
        [
            CodecProfile::H264High,
            CodecProfile::H264Main,
            CodecProfile::H264High10,
        ]
    );
    assert_eq!(
        provider.profiles(CodecId::H264, Direction::Encode),
        [CodecProfile::H264Baseline]
    );
    assert_eq!(
        provider.profiles(CodecId::Aac, Direction::Decode),
        [CodecProfile::AacLc]
    );
    assert!(
        provider
            .profiles(CodecId::Vp9, Direction::Decode)
            .is_empty()
    );
}

#[test]
fn registry_profiles_attribute_each_profile_to_its_providers() {
    let profiles = registry().profiles(CodecId::H264, Direction::Decode);
    assert_eq!(
        profiles,
        [
            (CodecProfile::H264High, vec!["alpha".to_string()],),
            (
                CodecProfile::H264Main,
                vec!["alpha".to_string(), "beta".to_string()],
            ),
            (CodecProfile::H264High10, vec!["alpha".to_string()],),
            (CodecProfile::H264High422, vec!["beta".to_string()],),
        ]
    );
    assert_eq!(
        registry().profiles(CodecId::Aac, Direction::Decode),
        [
            (
                CodecProfile::AacLc,
                vec!["alpha".to_string(), "beta".to_string()]
            ),
            (CodecProfile::AacHe, vec!["beta".to_string()]),
        ]
    );
}
