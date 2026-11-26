//! Two fake providers behind one registry.
//!
//! Usage: `cargo run -p broadcast-codec --example registry [PREFERENCE]`, where
//! `PREFERENCE` is a comma-separated preference list such as
//! `enc:h264=beta:h264_hw`. With no argument a built-in preference is used.
//!
//! Both providers are the same fake type; only their offers differ. Each
//! decoder and encoder records who built it, so the last thing printed is which
//! provider actually served each request.

use std::cell::RefCell;
use std::env;
use std::num::{NonZeroU32, NonZeroU64};
use std::rc::Rc;

use broadcast_codec::{
    CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProvider, Decoder, Direction,
    Encoder, EncoderConfig, Frame, Packet, ProviderInfo, Registry, Timescale, parse_preferences,
    render_inventory,
};

/// The preference used when no argument is given.
const DEFAULT_PREFERENCE: &str = "enc:h264=beta:h264_hw";
/// Sample rate both sample requests use.
const SAMPLE_RATE: u32 = 48_000;
/// Target bit rate the encoder request asks for.
const BIT_RATE: u64 = 2_000_000;

/// One implementation a provider offers: its own name, the codec and the
/// direction.
type Offer = (&'static str, CodecId, Direction);

/// What the `alpha` provider offers.
const ALPHA_OFFERS: &[Offer] = &[
    ("h264", CodecId::H264, Direction::Decode),
    ("h264", CodecId::H264, Direction::Encode),
    ("aac", CodecId::Aac, Direction::Decode),
];

/// What the `beta` provider offers, in the order it lists them.
const BETA_OFFERS: &[Offer] = &[
    ("h264_hw", CodecId::H264, Direction::Decode),
    ("h264_sw", CodecId::H264, Direction::Decode),
    ("h264_hw", CodecId::H264, Direction::Encode),
    ("h264_sw", CodecId::H264, Direction::Encode),
];

/// The codec and direction pairs the candidate table is printed for.
const TABLE: &[(CodecId, Direction)] = &[
    (CodecId::H264, Direction::Decode),
    (CodecId::H264, Direction::Encode),
    (CodecId::Aac, Direction::Decode),
];

type Log = Rc<RefCell<Vec<String>>>;

struct Fake {
    name: &'static str,
    catalogue: &'static [Offer],
    log: Log,
}

impl Fake {
    fn offers(&self, implementation: &str, codec: CodecId, direction: Direction) -> bool {
        self.catalogue.contains(&(implementation, codec, direction))
    }

    fn record(&self, implementation: &str, direction: Direction) {
        let line = format!("{}:{implementation} ({direction})", self.name);
        self.log.borrow_mut().push(line);
    }
}

impl CodecProvider for Fake {
    fn name(&self) -> &str {
        self.name
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(self.name, "0.1.0")
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        self.catalogue
            .iter()
            .map(|(implementation, codec, direction)| {
                CodecDescriptor::new(self.name, *implementation, *codec, *direction)
            })
            .collect()
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        if !self.offers(implementation, parameters.codec(), Direction::Decode) {
            return Err(CodecError::unsupported(implementation.to_owned()));
        }
        self.record(implementation, Direction::Decode);
        Ok(Box::new(FakeDecoder {
            parameters: parameters.clone(),
        }))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        if !self.offers(implementation, config.codec(), Direction::Encode) {
            return Err(CodecError::unsupported(implementation.to_owned()));
        }
        self.record(implementation, Direction::Encode);
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_PREFERENCE.to_owned());
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut registry = Registry::new()
        .with(Box::new(Fake {
            name: "alpha",
            catalogue: ALPHA_OFFERS,
            log: Rc::clone(&log),
        }))?
        .with(Box::new(Fake {
            name: "beta",
            catalogue: BETA_OFFERS,
            log: Rc::clone(&log),
        }))?;
    for preference in parse_preferences(&text)? {
        registry = registry.prefer(preference);
    }

    println!("preferences: {text}");
    for (codec, direction) in TABLE {
        println!("candidates for {codec} {direction}:");
        for descriptor in registry.candidates(*codec, *direction) {
            println!(
                "  {}:{}",
                descriptor.provider(),
                descriptor.implementation()
            );
        }
    }

    let rate = NonZeroU32::new(SAMPLE_RATE).ok_or("sample rate")?;
    let parameters = CodecParameters::new(CodecId::H264, Timescale::from_nonzero(rate));
    let _decoder = registry.decoder(&parameters)?;
    let config = EncoderConfig::new(CodecId::H264, Timescale::from_nonzero(rate))
        .with_bit_rate(NonZeroU64::new(BIT_RATE).ok_or("bit rate")?);
    let _encoder = registry.encoder(&config)?;

    println!("served:");
    for line in log.borrow().iter() {
        println!("  {line}");
    }

    println!("inventory:");
    print!("{}", render_inventory(&registry.inventory(false)));
    Ok(())
}
