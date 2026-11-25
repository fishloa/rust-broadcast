//! Payload plug-ins (Phase 4 slice 8c): a data-track sample rendered as values.
//!
//! The fixture packets are MISB ST 0601 UAS Datalink Local Sets copied from
//! ST 0902.5 Annex C (see `private/fixtures/misb/PROVENANCE.md`); the expected
//! values are the ones ST 0601 §8 prints as its per-item examples. The typed
//! item decode is `motion-imagery-metadata`'s, so neither value side of these
//! assertions comes from this crate.
//!
//! Spec: MISB ST 0601 §5.5/§7.1 (checksum), §8 (per-item examples). See
//! `private/specs-verified/mpeg/transmux/klv/klv-misb0601.md`.

use bytes::Bytes;
use core::num::NonZeroU32;
use transmux::ir::{CodecConfig, DataIdentity, Sample, TrackId, TrackSpec};
use transmux::payload::{
    PayloadDecoder, PayloadRegistry, PayloadScalar, PayloadValue, St0601Decoder,
};

const WHO: &str = "payload_plugins";

fn data_spec() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        NonZeroU32::new(90_000).expect("90 kHz"),
        CodecConfig::Data {
            identity: DataIdentity::Mp4 {
                handler: *b"meta",
                sample_entry: Bytes::new(),
            },
        },
    )
}

fn sample(bytes: Vec<u8>) -> Sample {
    Sample::timed(Bytes::from(bytes), 0, 0, None)
}

fn value<'a>(values: &'a [PayloadValue], key: &str) -> &'a PayloadScalar {
    values
        .iter()
        .find(|v| v.key == key)
        .map(|v| &v.value)
        .unwrap_or_else(|| panic!("no `{key}` value in {values:?}"))
}

fn float(values: &[PayloadValue], key: &str) -> f64 {
    match value(values, key) {
        PayloadScalar::Float(v) => *v,
        other => panic!("`{key}`: expected a Float, got {other:?}"),
    }
}

#[test]
fn st0601_decoder_reads_the_published_example_values() {
    let Some(packet) = test_fixtures::require_bytes("misb/DynamicOnlyMISMMSPacketData.bin", WHO)
    else {
        return;
    };
    let values = St0601Decoder
        .decode(&sample(packet))
        .expect("decode a real ST 0601 packet");

    // ST 0601 §8's per-item examples, within the precision each field carries.
    assert!(
        (float(&values, "tag_5") - 159.974_365).abs() < 0.01,
        "tag 5"
    );
    assert!(
        (float(&values, "tag_6") - -0.431_531_724).abs() < 0.01,
        "tag 6"
    );
    assert!(
        (float(&values, "tag_13") - 60.176_822_966).abs() < 0.01,
        "tag 13"
    );
    assert!(
        (float(&values, "tag_15") - 14_190.719_5).abs() < 0.5,
        "tag 15"
    );
    assert_eq!(
        value(&values, "checksum_valid"),
        &PayloadScalar::Boolean(true)
    );
}

#[test]
fn st0601_decoder_reports_an_invalid_checksum_as_a_value() {
    // PROVENANCE records this packet's checksum as hand-edited and invalid;
    // the decoder must say so, not fail or drop the sample.
    let Some(packet) =
        test_fixtures::require_bytes("misb/DynamicConstantMISMMSPacketData.bin", WHO)
    else {
        return;
    };
    let values = St0601Decoder
        .decode(&sample(packet))
        .expect("decode a real ST 0601 packet");
    assert!(!values.is_empty(), "the sample's items still decode");
    assert_eq!(
        value(&values, "checksum_valid"),
        &PayloadScalar::Boolean(false)
    );
}

#[test]
fn a_registry_with_no_claiming_decoder_returns_none() {
    let registry = PayloadRegistry::new();
    let spec = data_spec();
    let sample = sample(vec![0u8; 16]);
    assert_eq!(registry.decode(&spec, &sample), None);

    let mut registry = PayloadRegistry::new();
    registry.register(Box::new(St0601Decoder));
    assert_eq!(
        registry.decode(&spec, &sample),
        None,
        "ST 0601 does not claim a sample without its Universal Label"
    );
}
