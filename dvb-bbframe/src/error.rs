//! Error type for BBFrame parsing and serialization.

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error type for BBFrame parsing and serialization.
///
/// All variants carry spec-clause references in their display messages.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding.
    #[error("buffer too short: need {need} bytes, have {have} (while parsing {what})")]
    BufferTooShort {
        /// Bytes required to proceed.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// Human-readable name of the type or field being parsed.
        what: &'static str,
    },

    /// MODE field is neither 0 (NM) nor 1 (HEM).
    #[error("invalid MODE: {mode} (must be 0 or 1 per EN 302 755 §5.1.7)")]
    InvalidMode {
        /// MODE value that was rejected.
        mode: u8,
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

    /// DFL field is outside the valid range. The enforced ceiling `DFL_MAX_BITS`
    /// (64800) is the DVB-S2 normal-FECFRAME data-field bound (EN 302 307-1
    /// §5.1.4); DVB-T2 is tighter still (0..=53760, EN 302 755 Table 2).
    #[error(
        "DFL={dfl} bits exceeds maximum {max} bits (EN 302 307-1 §5.1.4 S2 normal frame; \
         DVB-T2 tighter per EN 302 755 Table 2)"
    )]
    DflOutOfRange {
        /// DFL value that was rejected.
        dfl: u16,
        /// Maximum allowed DFL.
        max: u16,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_too_short_message_contains_values() {
        let err = Error::BufferTooShort {
            need: 10,
            have: 5,
            what: "BBHEADER",
        };
        let msg = format!("{err}");
        assert!(msg.contains("10") && msg.contains("5") && msg.contains("BBHEADER"));
    }

    #[test]
    fn invalid_mode_message_contains_clause_ref() {
        let err = Error::InvalidMode { mode: 3 };
        let msg = format!("{err}");
        assert!(msg.contains("5.1.7"));
    }
}
