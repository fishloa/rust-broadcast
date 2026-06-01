//! Error type for BBFrame parsing and serialization.

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error type for BBFrame parsing and serialization.
///
/// All variants carry spec-clause references in their display messages.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding.
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort {
        /// Bytes required to proceed.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },

    /// MATYPE field contained an invalid value.
    #[error("invalid MATYPE field: {reason}")]
    InvalidMatypeField {
        /// Why the field is invalid.
        reason: &'static str,
    },

    /// MODE field is neither 0 (NM) nor 1 (HEM).
    #[error("invalid MODE: {mode} (must be 0 or 1 per EN 302 755 §5.1.7)")]
    InvalidMode {
        /// MODE value that was rejected.
        mode: u8,
    },

    /// CRC-8 validation failed (EN 302 755 Annex F / EN 302 307-1 §5.1.4).
    #[error("CRC-8 mismatch: computed=0x{computed:02X}, stored=0x{stored:02X}")]
    Crc8Mismatch {
        /// CRC we calculated over bytes 0-8.
        computed: u8,
        /// CRC carried at byte 9 (XOR'd with MODE).
        stored: u8,
    },

    /// TS/GS input stream type is not supported.
    #[error("unsupported TS/GS: 0x{ts_gs:02X}")]
    UnsupportedTsGs {
        /// The invalid TS/GS value.
        ts_gs: u8,
    },

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },

    /// DFL field is outside the valid range (0..=53760 for DVB-T2).
    #[error("DFL={dfl} bits exceeds maximum {max} bits (EN 302 755 Table 2)")]
    DflOutOfRange {
        /// DFL value that was rejected.
        dfl: u16,
        /// Maximum allowed DFL.
        max: u16,
    },

    /// HEM detected but expected NM fields are inconsistent.
    #[error("HEM inconsistency: {reason}")]
    InconsistentHem {
        /// Why the HEM state is inconsistent.
        reason: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_too_short_message_contains_values() {
        let err = Error::BufferTooShort { need: 10, have: 5 };
        let msg = format!("{err}");
        assert!(msg.contains("10") && msg.contains("5"));
    }

    #[test]
    fn crc8_mismatch_message_contains_values() {
        let err = Error::Crc8Mismatch {
            computed: 0x42,
            stored: 0x1F,
        };
        let msg = format!("{err}");
        assert!(msg.contains("42") && msg.contains("1F"));
    }

    #[test]
    fn invalid_mode_message_contains_clause_ref() {
        let err = Error::InvalidMode { mode: 3 };
        let msg = format!("{err}");
        assert!(msg.contains("5.1.7"));
    }
}
