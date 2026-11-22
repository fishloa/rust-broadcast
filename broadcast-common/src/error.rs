//! The common failure kinds of the workspace, defined once.
//!
//! Every parser/serializer crate keeps its own `Error` enum for failures that
//! are specific to its format, and carries each kind below as a single
//! wrapping variant:
//!
//! ```ignore
//! #[error(transparent)]
//! BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
//! ```
//!
//! so the struct, its `Display` message and its field names exist once (the
//! `http` / `aws-smithy-types` pattern: a foundation crate owns the shared
//! types, family crates wrap them) and `?` converts without hand-written
//! `From` impls.
//!
//! # Field naming
//!
//! `what` names the structure or unit being parsed, written or compared
//! (`"ProgramStreamMap"`, `"SNDU"`); `field` names one spec field inside it
//! (`"section_length"`). A kind that can only concern a field uses `field`;
//! one that can concern a whole structure uses `what`.
//!
//! # Shape
//!
//! Every kind is `Debug + Clone + Copy + PartialEq + Eq`, holds only plain
//! data and implements `core::error::Error`, so it works in `no_std`. None is
//! `#[non_exhaustive]`: the fields are public so a parser constructs the value
//! with a struct literal, and a future field is a deliberate breaking change.
//!
//! Kinds whose meaning differs per crate (unknown tag, invalid length, UTF-8,
//! unexpected table id) are deliberately absent.

/// An input buffer held fewer bytes than the structure being parsed needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("buffer too short: need {need} bytes, have {have} (while parsing {what})")]
pub struct BufferTooShort {
    /// Bytes required to proceed.
    pub need: usize,
    /// Bytes actually available.
    pub have: usize,
    /// Name of the type or field being parsed.
    pub what: &'static str,
}

/// An output buffer was smaller than the serialized form requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("output buffer too small: need {need}, have {have}")]
pub struct OutputBufferTooSmall {
    /// Required size.
    pub need: usize,
    /// Actual size.
    pub have: usize,
}

/// A value did not fit the wire field it is written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{field} = {value} exceeds the field maximum {max}")]
pub struct FieldOverflow {
    /// Spec name of the field, e.g. "section_length".
    pub field: &'static str,
    /// The value that did not fit.
    pub value: u64,
    /// The largest value the field can hold.
    pub max: u64,
}

/// A CRC-32 carried on the wire disagrees with the one computed over the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{what} CRC-32 mismatch: computed {computed:#010x}, expected {expected:#010x}")]
pub struct CrcMismatch {
    /// The structure the CRC protects, e.g. "PSM", "SNDU", "section".
    pub what: &'static str,
    /// The CRC computed over the received bytes.
    pub computed: u32,
    /// The CRC carried in the stream.
    pub expected: u32,
}

/// A fixed marker (TS sync byte, burst sync word, start code) did not match.
///
/// `expected` and `found` are widened to `u32` so one kind covers an 8-bit
/// sync byte, a 16-bit sync word and a 24/32-bit start code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what}: expected {expected:#x}, got {found:#x}")]
pub struct InvalidSync {
    /// The marker's name, e.g. "TS sync byte", "Pa sync word".
    pub what: &'static str,
    /// The value the spec fixes.
    pub expected: u32,
    /// The value read.
    pub found: u32,
}

/// A reserved field did not hold the value the spec requires (all ones, or
/// zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("reserved field {field} must be {expected:#x}, found {found:#x}")]
pub struct ReservedBits {
    /// Spec name of the reserved field.
    pub field: &'static str,
    /// The value the spec requires.
    pub expected: u64,
    /// The value read.
    pub found: u64,
}

/// A field held a value the spec does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("field {field} value {value} invalid: {reason}")]
pub struct InvalidValue {
    /// Spec name of the field.
    pub field: &'static str,
    /// The value read, widened to `u64`.
    pub value: u64,
    /// Why the value is rejected.
    pub reason: &'static str,
}

/// A field is structurally invalid and has no single offending number (a bad
/// combination, a missing mandatory part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid field {field}: {reason}")]
pub struct InvalidField {
    /// Spec name of the field.
    pub field: &'static str,
    /// Why the field is rejected.
    pub reason: &'static str,
}

/// A format version number this crate does not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported {what} version {found}")]
pub struct UnsupportedVersion {
    /// The structure carrying the version, e.g. "RTP", "emsg".
    pub what: &'static str,
    /// The version read.
    pub found: u8,
}

/// A length field disagrees with the number of bytes actually present or
/// computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("length mismatch for {what}: declared {declared}, actual {actual}")]
pub struct LengthMismatch {
    /// The structure whose length is checked.
    pub what: &'static str,
    /// The length the header declares.
    pub declared: usize,
    /// The length actually found or computed.
    pub actual: usize,
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn display_messages() {
        assert_eq!(
            BufferTooShort {
                need: 4,
                have: 1,
                what: "PAT"
            }
            .to_string(),
            "buffer too short: need 4 bytes, have 1 (while parsing PAT)"
        );
        assert_eq!(
            OutputBufferTooSmall { need: 4, have: 1 }.to_string(),
            "output buffer too small: need 4, have 1"
        );
        assert_eq!(
            FieldOverflow {
                field: "section_length",
                value: 5000,
                max: 4095
            }
            .to_string(),
            "section_length = 5000 exceeds the field maximum 4095"
        );
        assert_eq!(
            CrcMismatch {
                what: "PSM",
                computed: 0xAB,
                expected: 0xCD
            }
            .to_string(),
            "PSM CRC-32 mismatch: computed 0x000000ab, expected 0x000000cd"
        );
        assert_eq!(
            InvalidSync {
                what: "TS sync byte",
                expected: 0x47,
                found: 0
            }
            .to_string(),
            "invalid TS sync byte: expected 0x47, got 0x0"
        );
        assert_eq!(
            ReservedBits {
                field: "reserved",
                expected: 0x7,
                found: 0x2
            }
            .to_string(),
            "reserved field reserved must be 0x7, found 0x2"
        );
        assert_eq!(
            InvalidValue {
                field: "version",
                value: 9,
                reason: "out of range"
            }
            .to_string(),
            "field version value 9 invalid: out of range"
        );
        assert_eq!(
            InvalidField {
                field: "pts",
                reason: "missing"
            }
            .to_string(),
            "invalid field pts: missing"
        );
        assert_eq!(
            UnsupportedVersion {
                what: "RTP",
                found: 1
            }
            .to_string(),
            "unsupported RTP version 1"
        );
        assert_eq!(
            LengthMismatch {
                what: "APDU",
                declared: 9,
                actual: 7
            }
            .to_string(),
            "length mismatch for APDU: declared 9, actual 7"
        );
    }

    #[test]
    fn equality_distinguishes_every_field() {
        let a = CrcMismatch {
            what: "PSM",
            computed: 1,
            expected: 2,
        };
        assert_eq!(a, a);
        assert_ne!(a, CrcMismatch { what: "SNDU", ..a });
        assert_ne!(a, CrcMismatch { computed: 3, ..a });
        assert_ne!(a, CrcMismatch { expected: 3, ..a });
        let b = LengthMismatch {
            what: "x",
            declared: 1,
            actual: 2,
        };
        assert_ne!(b, LengthMismatch { declared: 2, ..b });
        assert_ne!(b, LengthMismatch { actual: 1, ..b });
    }

    #[test]
    fn len_reexports_are_the_same_types() {
        let e: BufferTooShort = crate::len::need_in(&[], 1, "x").unwrap_err();
        assert_eq!(
            e,
            BufferTooShort {
                need: 1,
                have: 0,
                what: "x"
            }
        );
        let o: OutputBufferTooSmall = crate::len::need_out(&[], 1).unwrap_err();
        assert_eq!(o, OutputBufferTooSmall { need: 1, have: 0 });
        let _: FieldOverflow = crate::len::fit_bits(256, 8, "f").unwrap_err();
    }

    #[test]
    fn kinds_are_core_errors() {
        fn is_error<E: core::error::Error + Copy + Send + Sync + 'static>() {}
        is_error::<BufferTooShort>();
        is_error::<CrcMismatch>();
        is_error::<InvalidSync>();
        is_error::<ReservedBits>();
        is_error::<InvalidValue>();
        is_error::<InvalidField>();
        is_error::<UnsupportedVersion>();
        is_error::<LengthMismatch>();
    }
}
