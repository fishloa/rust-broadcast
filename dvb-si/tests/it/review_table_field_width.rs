//! Adversarial review (dvb-si tables/core batch): the #1129 rule ("never
//! write a field with `as u8` or a bitmask, which silently wraps an
//! over-range value") is enforced for descriptors
//! (`serialize_field_width_guard.rs`) but not for TABLE headers. The shared
//! helpers in `tables/mod.rs` mask instead of checking:
//!
//! * `b5_of`: `(version_number & B5_VERSION_MASK) << 1` — a 5-bit field;
//! * `b1_of`: `(reserved_b1 & RESERVED_2) << 4`, `b5_of`:
//!   `(reserved_b5 & RESERVED_2) << 6`;
//! * `write_pid_r`: `(reserved & RESERVED_3) << 5`.
//!
//! The reserved fields are new public, settable fields in this release
//! (`SectionBits`, `PatEntry::reserved`, ...), so an over-range value is now
//! reachable through the public API and is silently truncated. The lexical
//! `serialize_mask` guard (workspace-guards) does not see these because the mask lives in a
//! helper, not on `self.field` in a `serialize_into`.
use broadcast_core::Serialize;
use dvb_si::tables::pat::{PatEntry, PatSection};
use mpeg_ts::pid::Pid;

fn ser(p: &PatSection) -> Result<Vec<u8>, dvb_si::error::Error> {
    let mut b = vec![0u8; p.serialized_len()];
    p.serialize_into(&mut b)?;
    Ok(b)
}

#[test]
fn pat_version_number_32_is_rejected_not_wrapped_to_0() {
    // Widest legal value still serializes.
    assert!(ser(&PatSection::new(1, 31, true, 0, 0, vec![])).is_ok());
    let r = ser(&PatSection::new(1, 32, true, 0, 0, vec![]));
    assert!(
        matches!(&r, Err(dvb_si::error::Error::FieldOverflow(_))),
        "version_number 32 serialized as version {} instead of FieldOverflow",
        r.map(|b| (b[5] >> 1) & 0x1F).unwrap_or(0)
    );
}

#[test]
fn pat_entry_reserved_cannot_hold_more_than_3_bits() {
    let mut e = PatEntry::new(1, Pid::from_const(0x100));
    e.reserved = broadcast_core::reserved::ReservedOnes::<3>::from_bits(0x0F);
    assert_eq!(
        e.reserved.get(),
        0x07,
        "the 4th bit is not part of the field"
    );
    assert!(ser(&PatSection::new(1, 0, true, 0, 0, vec![e])).is_ok());
}

#[test]
fn section_bits_reserved_cannot_hold_more_than_2_bits() {
    let mut p = PatSection::new(1, 0, true, 0, 0, vec![]);
    p.bits.reserved_b1 = broadcast_core::reserved::ReservedOnes::<2>::from_bits(0x07);
    assert_eq!(
        p.bits.reserved_b1.get(),
        0x03,
        "the 3rd bit is not part of the field"
    );
    assert!(ser(&p).is_ok(), "a reserved(2) is always in range");
}
