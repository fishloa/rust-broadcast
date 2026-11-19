//! The MPE datagram section's packed byte 5
//! (`reserved(2)` | `payload_scrambling_control(2)` | `address_scrambling_control(2)`
//! | `LLC_SNAP_flag` | `current_next_indicator`, ETSI EN 301 192 §7.1) compared
//! against hand-built wire bytes, so the mandatory `11` reserved fill cannot be
//! written shifted the wrong way.

use broadcast_common::Serialize;
use dvb_si::tables::mpe::{MacAddress, MpeDatagramSection};

#[test]
fn byte5_defaults_to_the_reserved_fill_and_current_next() {
    let section = MpeDatagramSection::new(MacAddress([1, 2, 3, 4, 5, 6]), 0, 0, &[0xAA]);
    let mut buf = vec![0u8; section.serialized_len()];
    section.serialize_into(&mut buf).unwrap();
    assert_eq!(buf[5], 0xC1);
}

#[test]
fn byte5_packs_scrambling_llc_and_current_next() {
    let mut section = MpeDatagramSection::new(MacAddress([1, 2, 3, 4, 5, 6]), 0, 0, &[0xAA]);
    section.payload_scrambling_control = 3;
    section.address_scrambling_control = 3;
    section.llc_snap_flag = true;
    section.current_next_indicator = false;
    let mut buf = vec![0u8; section.serialized_len()];
    section.serialize_into(&mut buf).unwrap();
    assert_eq!(buf[5], 0xFE);
}
