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
//! `serialize_mask_guard.rs` does not see these because the mask lives in a
//! helper, not on `self.field` in a `serialize_into`.
use broadcast_common::Serialize;
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
        r.is_err(),
        "version_number 32 serialized as version {} instead of FieldOverflow",
        r.map(|b| (b[5] >> 1) & 0x1F).unwrap_or(0)
    );
}

#[test]
fn pat_entry_reserved_wider_than_3_bits_is_rejected() {
    let mut e = PatEntry::new(1, Pid::new(0x100));
    e.reserved = 0x0F; // 4 bits into a reserved(3)
    let r = ser(&PatSection::new(1, 0, true, 0, 0, vec![e]));
    assert!(r.is_err(), "reserved(3) = 0x0F silently truncated to 0x07");
}

#[test]
fn section_bits_reserved_wider_than_2_bits_is_rejected() {
    let mut p = PatSection::new(1, 0, true, 0, 0, vec![]);
    p.bits.reserved_b1 = 0x07; // 3 bits into a reserved(2)
    assert!(ser(&p).is_err(), "reserved_b1 = 7 silently truncated to 3");
}
