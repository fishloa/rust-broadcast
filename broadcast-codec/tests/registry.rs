//! The selection rule, one test per clause, over two fake providers.

use std::cell::RefCell;
use std::num::NonZeroU32;
use std::rc::Rc;

use broadcast_codec::{
    CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProfile, CodecProvider, Decoder,
    Direction, Encoder, EncoderConfig, Fallback, Frame, Packet, Preference, ProviderInfo, Registry,
    RegistryError, SampleFormat, Timescale,
};

/// Sample rate the fake requests use.
const SAMPLE_RATE: u32 = 48_000;

/// The profiles `alpha` states for its H.264 implementations.
const H264_STATED: &[CodecProfile] = &[CodecProfile::H264High, CodecProfile::H264Main];
/// The profiles `beta`'s hardware H.264 implementation states.
const H264_HIGH10: &[CodecProfile] = &[CodecProfile::H264High10];

/// One implementation a fake provider offers.
struct Offer {
    implementation: &'static str,
    codec: CodecId,
    direction: Direction,
    profiles: &'static [CodecProfile],
}

const fn offer(
    implementation: &'static str,
    codec: CodecId,
    direction: Direction,
    profiles: &'static [CodecProfile],
) -> Offer {
    Offer {
        implementation,
        codec,
        direction,
        profiles,
    }
}

/// What the `alpha` provider offers: H.264 both ways and AAC decode, stating a
/// profile list for H.264 and none for AAC.
const ALPHA_OFFERS: &[Offer] = &[
    offer("h264", CodecId::H264, Direction::Decode, H264_STATED),
    offer("h264", CodecId::H264, Direction::Encode, H264_STATED),
    offer("aac", CodecId::Aac, Direction::Decode, &[]),
];

/// What the `beta` provider offers: H.264 both ways, two implementations, the
/// hardware one stating High 10 and the software one stating nothing.
const BETA_OFFERS: &[Offer] = &[
    offer("h264_hw", CodecId::H264, Direction::Decode, H264_HIGH10),
    offer("h264_sw", CodecId::H264, Direction::Decode, &[]),
    offer("h264_hw", CodecId::H264, Direction::Encode, H264_HIGH10),
    offer("h264_sw", CodecId::H264, Direction::Encode, &[]),
];

/// What a fake provider's builder answers.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Serve,
    Refuse,
    Unsupported,
    Fail,
}

struct Fake {
    name: &'static str,
    catalogue: &'static [Offer],
    answer: Answer,
    served: Rc<RefCell<Vec<String>>>,
}

impl Fake {
    fn willing(
        &self,
        implementation: &str,
        codec: CodecId,
        direction: Direction,
    ) -> Result<(), CodecError> {
        let offered = self.catalogue.iter().any(|offer| {
            offer.implementation == implementation
                && offer.codec == codec
                && offer.direction == direction
        });
        if !offered {
            return Err(CodecError::unsupported(format!(
                "{} does not offer {implementation} for {codec} {direction}",
                self.name
            )));
        }
        match self.answer {
            Answer::Serve => {
                self.served
                    .borrow_mut()
                    .push(format!("{}:{implementation}", self.name));
                Ok(())
            }
            Answer::Refuse => Err(CodecError::refused("fake refusal")),
            Answer::Unsupported => Err(CodecError::unsupported("fake gap")),
            Answer::Fail => Err(CodecError::failed("fake failure")),
        }
    }
}

impl CodecProvider for Fake {
    fn name(&self) -> &str {
        self.name
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(self.name, "1.0.0")
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        self.catalogue
            .iter()
            .map(|offer| {
                CodecDescriptor::new(
                    self.name,
                    offer.implementation,
                    offer.codec,
                    offer.direction,
                )
                .with_hardware(offer.implementation.ends_with("_hw"))
                .with_sample_formats(vec![SampleFormat::S16])
                .with_profiles(offer.profiles.iter().copied())
                .expect("a fake states only its own codec's profiles")
            })
            .collect()
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        self.willing(implementation, parameters.codec(), Direction::Decode)?;
        Ok(Box::new(FakeDecoder {
            parameters: parameters.clone(),
        }))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        self.willing(implementation, config.codec(), Direction::Encode)?;
        Ok(Box::new(FakeEncoder))
    }
}

struct FakeDecoder {
    parameters: CodecParameters,
}

impl Decoder for FakeDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, _packet: Packet<'_>) -> Result<(), CodecError> {
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        Ok(None)
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn reset(&mut self) {}
}

struct FakeEncoder;

impl Encoder for FakeEncoder {
    fn send(&mut self, _frame: Frame) -> Result<(), CodecError> {
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        Ok(None)
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        None
    }
}

type Served = Rc<RefCell<Vec<String>>>;

fn provider(
    name: &'static str,
    catalogue: &'static [Offer],
    answer: Answer,
    served: &Served,
) -> Box<dyn CodecProvider> {
    Box::new(Fake {
        name,
        catalogue,
        answer,
        served: Rc::clone(served),
    })
}

fn names(descriptors: &[CodecDescriptor]) -> Vec<String> {
    descriptors
        .iter()
        .map(|descriptor| format!("{}:{}", descriptor.provider(), descriptor.implementation()))
        .collect()
}

fn timescale() -> Timescale {
    Timescale::from_nonzero(NonZeroU32::new(SAMPLE_RATE).expect("non-zero sample rate"))
}

fn parameter(codec: CodecId) -> CodecParameters {
    CodecParameters::new(codec, timescale())
}

fn profiled(profile: CodecProfile) -> CodecParameters {
    CodecParameters::new(CodecId::H264, timescale())
        .with_profile(profile)
        .expect("an H.264 profile")
}

fn config(codec: CodecId) -> EncoderConfig {
    EncoderConfig::new(codec, timescale())
}

fn unsupported_message(error: &CodecError) -> String {
    match error {
        CodecError::Unsupported { what } => what.to_string(),
        other => panic!("expected Unsupported, got {other}"),
    }
}

/// Two fake providers registrable in either order of answers, with a log of
/// who served what.
struct Bench {
    served: Served,
    registry: Registry,
}

impl Bench {
    fn new(alpha: Answer, beta: Answer) -> Self {
        let served: Served = Rc::new(RefCell::new(Vec::new()));
        let registry = Registry::new()
            .with(provider("alpha", ALPHA_OFFERS, alpha, &served))
            .expect("alpha registers")
            .with(provider("beta", BETA_OFFERS, beta, &served))
            .expect("beta registers");
        Self { served, registry }
    }

    fn prefer(mut self, preference: Preference) -> Self {
        self.registry = self.registry.prefer(preference);
        self
    }

    fn fallback(mut self, fallback: Fallback) -> Self {
        self.registry = self.registry.fallback(fallback);
        self
    }

    fn select(mut self, selector: impl Fn(&[CodecDescriptor]) -> Option<usize> + 'static) -> Self {
        self.registry = self.registry.select_with(Box::new(selector));
        self
    }

    fn candidates(&self, codec: CodecId, direction: Direction) -> Vec<String> {
        names(&self.registry.candidates(codec, direction))
    }

    fn decoder(&self) -> Result<Box<dyn Decoder>, CodecError> {
        self.registry.decoder(&parameter(CodecId::H264))
    }

    fn decoder_in(&self, profile: CodecProfile) -> Result<Box<dyn Decoder>, CodecError> {
        self.registry.decoder(
            &parameter(CodecId::H264)
                .with_profile(profile)
                .expect("an H.264 profile"),
        )
    }

    fn encoder(&self) -> Result<Box<dyn Encoder>, CodecError> {
        self.registry.encoder(&config(CodecId::H264))
    }

    fn encoder_in(&self, profile: CodecProfile) -> Result<Box<dyn Encoder>, CodecError> {
        self.registry.encoder(
            &config(CodecId::H264)
                .with_profile(profile)
                .expect("an H.264 profile"),
        )
    }

    fn candidates_for(&self, profile: CodecProfile) -> Vec<String> {
        names(
            &self.registry.candidates_for(
                &parameter(CodecId::H264)
                    .with_profile(profile)
                    .expect("an H.264 profile"),
            ),
        )
    }

    fn served(&self) -> Vec<String> {
        self.served.borrow().clone()
    }
}

#[test]
fn rule_1_candidates_are_every_match_in_registration_order() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );
    assert_eq!(
        bench.candidates(CodecId::Aac, Direction::Decode),
        ["alpha:aac"]
    );
    assert_eq!(
        names(&bench.registry.list()),
        [
            "alpha:h264",
            "alpha:h264",
            "alpha:aac",
            "beta:h264_hw",
            "beta:h264_sw",
            "beta:h264_hw",
            "beta:h264_sw",
        ]
    );
}

#[test]
fn rule_2_selector_answer_is_the_first_candidate() {
    let bench = Bench::new(Answer::Serve, Answer::Serve).select(|candidates| {
        candidates
            .iter()
            .position(|descriptor| descriptor.implementation() == "h264_sw")
    });
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_sw", "alpha:h264", "beta:h264_hw"]
    );
}

#[test]
fn rule_2_selector_none_falls_through_to_the_preference() {
    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "beta"))
        .select(|_| None);
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_hw", "beta:h264_sw", "alpha:h264"]
    );
}

#[test]
fn rule_2_selector_out_of_range_falls_through_to_registration_order() {
    let bench = Bench::new(Answer::Serve, Answer::Serve).select(|_| Some(99));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );
}

#[test]
fn rule_3_preference_by_provider_moves_all_its_candidates_to_the_front() {
    let bench =
        Bench::new(Answer::Serve, Answer::Serve).prefer(Preference::new(CodecId::H264, "beta"));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_hw", "beta:h264_sw", "alpha:h264"]
    );
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Encode),
        ["beta:h264_hw", "beta:h264_sw", "alpha:h264"]
    );
}

#[test]
fn rule_3_preference_by_provider_and_implementation_moves_that_one() {
    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "beta").with_implementation("h264_sw"));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_sw", "alpha:h264", "beta:h264_hw"]
    );
}

#[test]
fn rule_3_later_preference_replaces_an_earlier_one() {
    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "beta").with_implementation("h264_sw"))
        .prefer(Preference::new(CodecId::H264, "alpha"));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );

    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "alpha"))
        .prefer(Preference::new(CodecId::H264, "beta").with_implementation("h264_sw"));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_sw", "alpha:h264", "beta:h264_hw"]
    );
}

#[test]
fn rule_3_direction_scoped_preference_outranks_one_for_both() {
    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "beta"))
        .prefer(Preference::new(CodecId::H264, "alpha").with_direction(Direction::Encode));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["beta:h264_hw", "beta:h264_sw", "alpha:h264"]
    );
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Encode),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );
}

#[test]
fn rule_3_preference_for_another_codec_does_not_apply() {
    let bench =
        Bench::new(Answer::Serve, Answer::Serve).prefer(Preference::new(CodecId::Aac, "beta"));
    assert_eq!(
        bench.candidates(CodecId::H264, Direction::Decode),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );
}

#[test]
fn rule_3_unregistered_provider_is_an_error() {
    let bench =
        Bench::new(Answer::Serve, Answer::Serve).prefer(Preference::new(CodecId::H264, "oxideav"));
    let error = bench.decoder().error();
    assert!(unsupported_message(&error).contains("oxideav"), "{error}");
    let error = bench.encoder().error();
    assert!(unsupported_message(&error).contains("oxideav"), "{error}");
    assert!(bench.served().is_empty(), "no provider may be asked");
}

#[test]
fn rule_3_unregistered_implementation_is_an_error() {
    let bench = Bench::new(Answer::Serve, Answer::Serve)
        .prefer(Preference::new(CodecId::H264, "beta").with_implementation("h264_avx"));
    let error = bench.decoder().error();
    assert!(unsupported_message(&error).contains("h264_avx"), "{error}");
    assert!(bench.served().is_empty());
}

#[test]
fn rule_3_registered_provider_offering_nothing_is_an_error() {
    let bench =
        Bench::new(Answer::Serve, Answer::Serve).prefer(Preference::new(CodecId::Aac, "beta"));
    let error = bench.registry.decoder(&parameter(CodecId::Aac)).error();
    assert!(unsupported_message(&error).contains("beta"), "{error}");
}

#[test]
fn rule_5_registration_order_decides_when_nothing_is_set() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    bench.decoder().expect("alpha serves the decoder");
    bench.encoder().expect("alpha serves the encoder");
    assert_eq!(bench.served(), ["alpha:h264", "alpha:h264"]);
}

#[test]
fn rule_6_strict_returns_the_first_candidates_error() {
    let bench = Bench::new(Answer::Refuse, Answer::Serve);
    let error = bench.decoder().error();
    assert_eq!(error, CodecError::refused("fake refusal"));
    assert!(bench.served().is_empty(), "beta must not be asked");
}

#[test]
fn rule_6_next_candidate_skips_unsupported_and_refused() {
    let bench = Bench::new(Answer::Unsupported, Answer::Serve).fallback(Fallback::NextCandidate);
    bench.decoder().expect("beta serves after an Unsupported");
    assert_eq!(bench.served(), ["beta:h264_hw"]);

    let bench = Bench::new(Answer::Refuse, Answer::Serve).fallback(Fallback::NextCandidate);
    bench.decoder().expect("beta serves after a Refused");
    assert_eq!(bench.served(), ["beta:h264_hw"]);
}

#[test]
fn rule_6_next_candidate_stops_on_failed() {
    let bench = Bench::new(Answer::Fail, Answer::Serve).fallback(Fallback::NextCandidate);
    let error = bench.decoder().error();
    assert_eq!(error, CodecError::failed("fake failure"));
    assert!(bench.served().is_empty());
}

#[test]
fn rule_6_all_skipped_lists_what_was_tried() {
    let bench = Bench::new(Answer::Unsupported, Answer::Refuse).fallback(Fallback::NextCandidate);
    let error = bench.decoder().error();
    let message = unsupported_message(&error);
    for tried in ["alpha:h264", "beta:h264_hw", "beta:h264_sw"] {
        assert!(message.contains(tried), "{tried} missing from {message}");
    }
}

#[test]
fn rule_7_no_candidate_names_the_codec_and_direction() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    assert!(bench.candidates(CodecId::Vp9, Direction::Decode).is_empty());
    let error = bench.registry.decoder(&parameter(CodecId::Vp9)).error();
    let message = unsupported_message(&error);
    assert!(
        message.contains("decode") && message.contains("VP9"),
        "{message}"
    );
    let error = bench.registry.encoder(&config(CodecId::Opus)).error();
    let message = unsupported_message(&error);
    assert!(
        message.contains("encode") && message.contains("Opus"),
        "{message}"
    );
}

#[test]
fn duplicate_provider_name_is_refused() {
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let registry = Registry::new()
        .with(provider("alpha", ALPHA_OFFERS, Answer::Serve, &served))
        .expect("the first alpha registers")
        .with(provider("alpha", BETA_OFFERS, Answer::Serve, &served));
    assert_eq!(
        registry.error(),
        RegistryError::DuplicateName {
            name: "alpha".into()
        }
    );
}

#[test]
fn invalid_provider_name_is_refused() {
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let registry = Registry::new().with(provider("Alpha", ALPHA_OFFERS, Answer::Serve, &served));
    assert_eq!(
        registry.error(),
        RegistryError::InvalidName {
            name: "Alpha".into(),
            character: Some('A'),
        }
    );
    let registry = Registry::new().with(provider("", ALPHA_OFFERS, Answer::Serve, &served));
    assert_eq!(
        registry.error(),
        RegistryError::InvalidName {
            name: String::new(),
            character: None,
        }
    );
}

#[test]
fn rule_4_a_stated_profile_list_that_omits_the_streams_profile_is_removed() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    assert_eq!(
        bench.candidates_for(CodecProfile::H264High10),
        ["beta:h264_hw", "beta:h264_sw"]
    );
    bench
        .decoder_in(CodecProfile::H264High10)
        .expect("beta's hardware decoder states High 10");
    assert_eq!(bench.served(), ["beta:h264_hw"]);
}

#[test]
fn rule_4_a_candidate_that_states_no_profiles_is_kept() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    assert_eq!(
        bench.candidates_for(CodecProfile::H264High),
        ["alpha:h264", "beta:h264_sw"]
    );
    let rejected: Vec<String> = bench
        .registry
        .rejected_for(&profiled(CodecProfile::H264High))
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(rejected, ["beta:h264_hw (states h264/high10)"]);
}

#[test]
fn rule_4_an_unstated_stream_profile_removes_nothing() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    assert_eq!(
        names(&bench.registry.candidates_for(&parameter(CodecId::H264))),
        ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
    );
    assert!(
        bench
            .registry
            .rejected_for(&parameter(CodecId::H264))
            .is_empty()
    );
}

#[test]
fn rule_4_a_mismatch_is_removed_under_strict() {
    let bench = Bench::new(Answer::Serve, Answer::Serve).fallback(Fallback::Strict);
    bench
        .decoder_in(CodecProfile::H264High10)
        .expect("the removed first candidate is not tried");
    assert_eq!(bench.served(), ["beta:h264_hw"]);
}

#[test]
fn rule_4_all_removed_names_the_codec_the_profile_and_each_candidate() {
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let registry = Registry::new()
        .with(provider("alpha", ALPHA_OFFERS, Answer::Serve, &served))
        .expect("alpha registers");
    let error = registry
        .decoder(&profiled(CodecProfile::H264High10))
        .error();
    assert_eq!(
        unsupported_message(&error),
        "decode of H.264 at profile h264/high10: no candidate handles it; \
         removed alpha:h264 (states h264/high, h264/main)"
    );
    assert!(served.borrow().is_empty(), "no candidate may be asked");
}

#[test]
fn rule_4_a_preference_naming_a_removed_candidate_is_an_error() {
    let bench =
        Bench::new(Answer::Serve, Answer::Serve).prefer(Preference::new(CodecId::H264, "alpha"));
    let error = bench.decoder_in(CodecProfile::H264High10).error();
    let message = unsupported_message(&error);
    for expected in ["preference `alpha`", "alpha:h264", "h264/high10"] {
        assert!(
            message.contains(expected),
            "{expected} missing from {message}"
        );
    }
    assert!(bench.served().is_empty(), "no engine may be switched to");
}

#[test]
fn rule_4_a_selector_index_at_a_removed_candidate_is_an_error() {
    let bench = Bench::new(Answer::Serve, Answer::Serve).select(|candidates| {
        candidates
            .iter()
            .position(|descriptor| descriptor.codec() == CodecId::H264)
    });
    let error = bench.decoder_in(CodecProfile::H264High10).error();
    let message = unsupported_message(&error);
    assert!(message.contains("alpha:h264"), "{message}");
    assert!(message.contains("selector chose"), "{message}");
    assert!(bench.served().is_empty(), "no engine may be switched to");
}

#[test]
fn rule_4_an_encoder_config_profile_removes_mismatched_encoders() {
    let bench = Bench::new(Answer::Serve, Answer::Serve);
    let requested = config(CodecId::H264)
        .with_profile(CodecProfile::H264High10)
        .expect("an H.264 profile");
    assert_eq!(
        names(&bench.registry.encoder_candidates_for(&requested)),
        ["beta:h264_hw", "beta:h264_sw"]
    );
    let rejected: Vec<String> = bench
        .registry
        .encoder_rejected_for(&requested)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(rejected, ["alpha:h264 (states h264/high, h264/main)"]);
    bench
        .encoder_in(CodecProfile::H264High10)
        .expect("beta's hardware encoder states High 10");
    assert_eq!(bench.served(), ["beta:h264_hw"]);
}

#[test]
fn a_profile_scoped_preference_beats_a_bare_one_for_that_profile_only() {
    let preferences = |bench: Bench| {
        bench
            .prefer(Preference::new(CodecId::H264, "alpha"))
            .prefer(
                Preference::new(CodecId::H264, "beta")
                    .with_profile(CodecProfile::H264High10)
                    .expect("an H.264 profile"),
            )
    };

    let bench = preferences(Bench::new(Answer::Serve, Answer::Serve));
    bench
        .decoder_in(CodecProfile::H264High10)
        .expect("the profile-scoped preference picks beta");
    assert_eq!(bench.served(), ["beta:h264_hw"]);

    let bench = preferences(Bench::new(Answer::Serve, Answer::Serve));
    bench
        .decoder_in(CodecProfile::H264High)
        .expect("the profile-scoped preference does not apply");
    assert_eq!(bench.served(), ["alpha:h264"]);
}

/// The error of a `Result` whose success type is a trait object or a registry,
/// neither of which is `Debug`.
trait ErrorOf {
    type Error;

    fn error(self) -> Self::Error;
}

impl ErrorOf for Result<Box<dyn Decoder>, CodecError> {
    type Error = CodecError;

    fn error(self) -> CodecError {
        match self {
            Ok(_) => panic!("expected the decoder to be refused"),
            Err(error) => error,
        }
    }
}

impl ErrorOf for Result<Box<dyn Encoder>, CodecError> {
    type Error = CodecError;

    fn error(self) -> CodecError {
        match self {
            Ok(_) => panic!("expected the encoder to be refused"),
            Err(error) => error,
        }
    }
}

impl ErrorOf for Result<Registry, RegistryError> {
    type Error = RegistryError;

    fn error(self) -> RegistryError {
        match self {
            Ok(_) => panic!("expected the provider to be refused"),
            Err(error) => error,
        }
    }
}

#[test]
fn rule_1_registration_order_is_the_order_providers_were_added() {
    let served: Served = Rc::new(RefCell::new(Vec::new()));
    let registry = Registry::new()
        .with(provider("beta", BETA_OFFERS, Answer::Serve, &served))
        .expect("beta registers")
        .with(provider("alpha", ALPHA_OFFERS, Answer::Serve, &served))
        .expect("alpha registers");
    assert_eq!(
        names(&registry.candidates(CodecId::H264, Direction::Decode)),
        ["beta:h264_hw", "beta:h264_sw", "alpha:h264"]
    );
}
