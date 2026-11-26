//! Print the run-time inventory the symphonia adapter reports.

use std::num::NonZeroU32;

use broadcast_codec::{
    AudioParameters, Availability, ChannelLayout, CodecId, CodecParameters, CodecProfile, Registry,
    Timescale, render_inventory,
};
use codec_symphonia::SymphoniaProvider;

/// The sample rate the refusal test asks for.
const SAMPLE_RATE: u32 = 48_000;

#[test]
fn prints_the_symphonia_inventory() {
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let providers = registry.providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name(), "symphonia");
    assert_eq!(providers[0].library(), Some("symphonia 0.6.1"));
    assert_eq!(providers[0].licence(), Some("MPL-2.0"));

    let entries = registry.inventory(true);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].probe().availability(), Availability::Available);

    let table = render_inventory(&entries);
    println!("{table}");
    assert!(table.contains("symphonia"));
    assert!(table.contains("MPL-2.0"));
}

#[test]
fn the_reported_library_version_is_the_locked_one() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))
        .expect("the spike's Cargo.lock");
    let version = symphonia_version(&lock).expect("symphonia in Cargo.lock");
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let library = format!("symphonia {version}");
    assert_eq!(registry.providers()[0].library(), Some(library.as_str()));
}

fn symphonia_version(lock: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"symphonia\"" {
            let version = lines.next()?.trim();
            return version
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    None
}

#[test]
fn the_descriptor_states_low_complexity_only() {
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let entries = registry.inventory(false);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].descriptor().profiles(), [CodecProfile::AacLc]);
    assert_eq!(
        registry.profiles(CodecId::Aac, broadcast_codec::Direction::Decode),
        [(CodecProfile::AacLc, vec!["symphonia".to_string()])]
    );
    assert!(render_inventory(&entries).contains("lc"));
}

#[test]
fn aac_he_is_refused_before_the_decoder_is_built() {
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let rate = NonZeroU32::new(SAMPLE_RATE).expect("a non-zero sample rate");
    let parameters = CodecParameters::new(CodecId::Aac, Timescale::from_nonzero(rate))
        .with_audio(AudioParameters::new(rate, ChannelLayout::Mono))
        .with_profile(CodecProfile::AacHe)
        .expect("an AAC profile");
    let Err(error) = registry.decoder(&parameters) else {
        panic!("High Efficiency is not a profile symphonia states");
    };
    let message = error.to_string();
    assert!(message.contains("aac/he"), "{message}");
    assert!(message.contains("symphonia:aac"), "{message}");
}
