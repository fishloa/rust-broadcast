//! Serializers must reject a value wider than its wire bit-field instead of
//! masking it into a different, valid-looking section/descriptor (the #1129
//! class). The widest legal value must still round-trip, so rejecting
//! everything cannot satisfy the test.

use base64::{Engine, engine::general_purpose::STANDARD};
use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::descriptors::audio::{AudioComponent, AudioDescriptor};

/// The canonical SCTE 35 `splice_insert()` example (CRC-valid).
const SECTION: &str = "/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=";

#[test]
fn splice_info_section_sap_type_and_encryption_algorithm_widths() {
    let bytes = STANDARD.decode(SECTION).unwrap();
    let ok = SpliceInfoSection::parse(&bytes).unwrap();
    assert_eq!(ok.try_to_bytes().unwrap(), bytes);

    // sap_type is 2 bits.
    let mut max = ok.clone();
    max.sap_type = 0x03;
    assert!(max.try_to_bytes().is_ok());
    let mut over = ok.clone();
    over.sap_type = 0x04;
    assert!(
        over.try_to_bytes().is_err(),
        "sap_type 4 must not wrap to 0"
    );

    // encryption_algorithm is 6 bits.
    let mut max = ok.clone();
    max.encryption_algorithm = 0x3F;
    assert!(max.try_to_bytes().is_ok());
    let mut over = ok;
    over.encryption_algorithm = 0x40;
    assert!(
        over.try_to_bytes().is_err(),
        "encryption_algorithm 64 must not wrap to 0"
    );
}

#[test]
fn audio_descriptor_component_field_widths() {
    let component = |mode, ch| AudioComponent {
        component_tag: 1,
        iso_code: *b"eng",
        bit_stream_mode: mode,
        num_channels: ch,
        full_srvc_audio: true,
    };
    let desc = |c| AudioDescriptor {
        identifier: 0x4355_4549,
        components: vec![c],
    };
    // bit_stream_mode is 3 bits, num_channels 4 bits.
    let ok = desc(component(7, 15));
    let bytes = ok.try_to_bytes().unwrap();
    assert_eq!(AudioDescriptor::parse(&bytes).unwrap(), ok);
    assert!(desc(component(8, 0)).try_to_bytes().is_err());
    assert!(desc(component(0, 16)).try_to_bytes().is_err());
}
