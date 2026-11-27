//! The run-time inventory: provider information, profiles, probes and the
//! rendered table.

use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::rc::Rc;

use broadcast_codec::{
    Availability, CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProfile,
    CodecProvider, Decoder, DecoderDelay, DecoderTrim, Direction, Encoder, EncoderConfig, Frame,
    InventoryEntry, Packet, Probe, ProviderInfo, Registry, Timescale, render_inventory,
};

/// Sample rate the fake requests use.
const SAMPLE_RATE: u32 = 48_000;
/// Why the `_hw` implementations report themselves unavailable.
const UNAVAILABLE_REASON: &str = "no hardware codec on this host";

/// One implementation a fake provider offers.
type Offer = (&'static str, CodecId, Direction);

/// What the `alpha` provider offers.
const ALPHA_OFFERS: &[Offer] = &[
    ("h264", CodecId::H264, Direction::Decode),
    ("h264", CodecId::H264, Direction::Encode),
    ("aac", CodecId::Aac, Direction::Decode),
];

/// What the `beta` provider offers.
const BETA_OFFERS: &[Offer] = &[
    ("h264_hw", CodecId::H264, Direction::Decode),
    ("h264_sw", CodecId::H264, Direction::Decode),
];

struct Fake {
    name: &'static str,
    version: &'static str,
    library: Option<&'static str>,
    licence: Option<&'static str>,
    offers: &'static [Offer],
    profiles: bool,
    probes: Rc<Cell<usize>>,
    served: Rc<RefCell<Vec<String>>>,
}

impl CodecProvider for Fake {
    fn name(&self) -> &str {
        self.name
    }

    fn info(&self) -> ProviderInfo {
        let info = ProviderInfo::new(self.name, self.version);
        let info = match self.library {
            Some(library) => info.with_library(library),
            None => info,
        };
        match self.licence {
            Some(licence) => info.with_licence(licence),
            None => info,
        }
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        self.offers
            .iter()
            .map(|(implementation, codec, direction)| {
                let descriptor =
                    CodecDescriptor::new(self.name, *implementation, *codec, *direction)
                        .with_hardware(implementation.ends_with("_hw"));
                if self.profiles && implementation.starts_with("h264") {
                    descriptor
                        .with_profiles([CodecProfile::H264High, CodecProfile::H264Main])
                        .expect("an H.264 descriptor states H.264 profiles")
                } else {
                    descriptor
                }
            })
            .collect()
    }

    fn probe(&self, implementation: &str, _direction: Direction, _codec: CodecId) -> Probe {
        self.probes.set(self.probes.get() + 1);
        if implementation.ends_with("_hw") {
            Probe::unavailable(UNAVAILABLE_REASON)
        } else {
            Probe::available()
        }
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        if !self
            .offers
            .contains(&(implementation, parameters.codec(), Direction::Decode))
        {
            return Err(CodecError::unsupported(implementation.to_owned()));
        }
        self.served
            .borrow_mut()
            .push(format!("{}:{implementation}", self.name));
        Ok(Box::new(FakeDecoder {
            parameters: parameters.clone(),
        }))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        _config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported(implementation.to_owned()))
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

struct Bench {
    probes: Rc<Cell<usize>>,
    served: Rc<RefCell<Vec<String>>>,
    registry: Registry,
}

fn bench(profiles: bool) -> Bench {
    let probes = Rc::new(Cell::new(0));
    let served = Rc::new(RefCell::new(Vec::new()));
    let registry = Registry::new()
        .with(Box::new(Fake {
            name: "alpha",
            version: "0.3.0",
            library: None,
            licence: None,
            offers: ALPHA_OFFERS,
            profiles,
            probes: Rc::clone(&probes),
            served: Rc::clone(&served),
        }))
        .expect("alpha registers")
        .with(Box::new(Fake {
            name: "beta",
            version: "1.4.2",
            library: Some("libfake 2.0.1"),
            licence: Some("MIT"),
            offers: BETA_OFFERS,
            profiles,
            probes: Rc::clone(&probes),
            served: Rc::clone(&served),
        }))
        .expect("beta registers");
    Bench {
        probes,
        served,
        registry,
    }
}

fn parameter(codec: CodecId) -> CodecParameters {
    let rate = NonZeroU32::new(SAMPLE_RATE).expect("non-zero sample rate");
    CodecParameters::new(codec, Timescale::from_nonzero(rate))
}

fn names(descriptors: &[CodecDescriptor]) -> Vec<String> {
    descriptors
        .iter()
        .map(|descriptor| format!("{}:{}", descriptor.provider(), descriptor.implementation()))
        .collect()
}

#[test]
fn providers_are_reported_in_registration_order() {
    let bench = bench(false);
    let providers = bench.registry.providers();
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0].name(), "alpha");
    assert_eq!(providers[0].version(), "0.3.0");
    assert_eq!(providers[0].library(), None);
    assert_eq!(providers[0].licence(), None);
    assert_eq!(providers[0].notes(), None);
    assert_eq!(providers[1].name(), "beta");
    assert_eq!(providers[1].version(), "1.4.2");
    assert_eq!(providers[1].library(), Some("libfake 2.0.1"));
    assert_eq!(providers[1].licence(), Some("MIT"));
}

#[test]
fn inventory_without_probe_is_unknown_and_asks_nothing() {
    let bench = bench(true);
    let entries = bench.registry.inventory(false);
    assert_eq!(entries.len(), bench.registry.list().len());
    for (entry, descriptor) in entries.iter().zip(bench.registry.list()) {
        assert_eq!(entry.descriptor(), &descriptor);
        assert_eq!(entry.probe().availability(), Availability::Unknown);
        assert_eq!(entry.probe().reason(), None);
    }
    assert_eq!(bench.probes.get(), 0, "no provider may be probed");
}

#[test]
fn inventory_with_probe_asks_once_per_descriptor() {
    let bench = bench(true);
    let entries = bench.registry.inventory(true);
    assert_eq!(bench.probes.get(), entries.len());

    let hardware = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "h264_hw")
        .expect("the hardware implementation is listed");
    assert_eq!(hardware.probe().availability(), Availability::Unavailable);
    assert_eq!(hardware.probe().reason(), Some(UNAVAILABLE_REASON));
    assert_eq!(hardware.provider().name(), "beta");

    let software = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "h264_sw")
        .expect("the software implementation is listed");
    assert_eq!(software.probe().availability(), Availability::Available);
    assert_eq!(software.probe().reason(), None);
}

#[test]
fn profiles_reach_the_inventory() {
    let bench = bench(true);
    let entries = bench.registry.inventory(false);
    let h264 = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "h264")
        .expect("alpha's H.264 implementation is listed");
    assert_eq!(
        h264.descriptor().profiles(),
        [CodecProfile::H264High, CodecProfile::H264Main]
    );
    let aac = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "aac")
        .expect("alpha's AAC implementation is listed");
    assert!(aac.descriptor().profiles().is_empty());
}

#[test]
fn profiles_change_nothing_when_the_stream_states_no_profile() {
    for profiles in [false, true] {
        let bench = bench(profiles);
        assert_eq!(
            names(&bench.registry.candidates(CodecId::H264, Direction::Decode)),
            ["alpha:h264", "beta:h264_hw", "beta:h264_sw"]
        );
        bench
            .registry
            .decoder(&parameter(CodecId::H264))
            .expect("alpha serves the decoder");
        assert_eq!(*bench.served.borrow(), ["alpha:h264"]);
    }
}

#[test]
fn entry_display_is_the_documented_line() {
    let bench = bench(true);
    let entries = bench.registry.inventory(true);
    let hardware = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "h264_hw")
        .expect("the hardware implementation is listed");
    assert_eq!(
        hardware.to_string(),
        "h264 decode beta h264_hw hw high/main unavailable MIT"
    );

    let aac = entries
        .iter()
        .find(|entry| entry.descriptor().implementation() == "aac")
        .expect("alpha's AAC implementation is listed");
    assert_eq!(aac.to_string(), "aac decode alpha aac sw - available -");

    let lossless = InventoryEntry::new(
        CodecDescriptor::new("gamma", "flac", CodecId::Flac, Direction::Decode).with_lossless(true),
        ProviderInfo::new("gamma", "2.0.0").with_licence("MPL-2.0"),
        Probe::unknown(),
    );
    assert_eq!(
        lossless.to_string(),
        "flac decode gamma flac sw lossless - unknown MPL-2.0"
    );
}

#[test]
fn the_table_renders_the_documented_columns() {
    let entries = [
        InventoryEntry::new(
            CodecDescriptor::new("alpha", "h264", CodecId::H264, Direction::Decode)
                .with_profiles([
                    CodecProfile::H264Baseline,
                    CodecProfile::H264Main,
                    CodecProfile::H264High,
                ])
                .expect("an H.264 descriptor states H.264 profiles"),
            ProviderInfo::new("alpha", "1.0.0").with_licence("MIT"),
            Probe::available(),
        ),
        InventoryEntry::new(
            CodecDescriptor::new("beta", "aac_hw", CodecId::Aac, Direction::Encode)
                .with_hardware(true),
            ProviderInfo::new("beta", "1.0.0"),
            Probe::unavailable("no hardware"),
        ),
        InventoryEntry::new(
            CodecDescriptor::new("gamma", "flac", CodecId::Flac, Direction::Decode)
                .with_lossless(true),
            ProviderInfo::new("gamma", "1.0.0").with_licence("MPL-2.0"),
            Probe::unknown(),
        ),
    ];
    let expected = "\
codec  direction  provider  implementation  kind  lossless  delay    profiles            availability  licence
h264   decode     alpha     h264            sw    -         unknown  baseline/main/high  available     MIT
aac    encode     beta      aac_hw          hw    -         unknown  -                   unavailable   -
flac   decode     gamma     flac            sw    yes       unknown  -                   unknown       MPL-2.0
";
    assert_eq!(render_inventory(&entries), expected);
}

#[test]
fn the_table_shows_each_implementations_declared_delay() {
    let entries = [
        InventoryEntry::new(
            CodecDescriptor::new("alpha", "mp3", CodecId::MpegAudio, Direction::Decode)
                .with_trim(DecoderTrim::Caller(DecoderDelay::Declared(529))),
            ProviderInfo::new("alpha", "1.0.0"),
            Probe::available(),
        ),
        InventoryEntry::new(
            CodecDescriptor::new("beta", "aac", CodecId::Aac, Direction::Decode)
                .with_trim(DecoderTrim::Caller(DecoderDelay::PerStream)),
            ProviderInfo::new("beta", "1.0.0"),
            Probe::available(),
        ),
        InventoryEntry::new(
            CodecDescriptor::new("gamma", "flac", CodecId::Flac, Direction::Decode)
                .with_trim(DecoderTrim::Implementation),
            ProviderInfo::new("gamma", "1.0.0"),
            Probe::available(),
        ),
    ];
    let table = render_inventory(&entries);
    assert!(table.contains("529"), "{table}");
    assert!(table.contains("per stream"), "{table}");
    assert!(table.contains("applies trim"), "{table}");
}
