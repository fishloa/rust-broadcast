//! Error type for the `ts-fix` crate.

use thiserror::Error;

/// Errors returned by [`crate::TsFixBuilder::build`] and [`crate::TsFix::push`].
///
/// `#[non_exhaustive]` so future variants (e.g. from new repair operations) are
/// additive — callers must not match exhaustively.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The slice passed to [`crate::TsFix::push`] was not exactly 188 bytes.
    ///
    /// ISO/IEC 13818-1 §2.4.3.2 — TS packets are fixed at 188 bytes.
    #[error("short packet: expected 188 bytes, got {len}")]
    ShortPacket {
        /// Actual length of the slice supplied by the caller.
        len: usize,
    },

    /// The first byte of the packet was not the TS sync byte `0x47`.
    ///
    /// ISO/IEC 13818-1 §2.4.3.2 — `sync_byte == 0x47`.
    #[error("missing TS sync byte: expected 0x47, got {found:#04x}")]
    NoSyncByte {
        /// The byte actually found at position 0.
        found: u8,
    },

    /// The buffer given to [`crate::pes::reconstruct_access_units`] is not a
    /// whole number of 188-byte TS packets.
    ///
    /// ISO/IEC 13818-1 §2.4.3.2 — TS packets are fixed at 188 bytes.
    #[error("unaligned TS buffer: {len} bytes is not a multiple of 188")]
    UnalignedInput {
        /// Length of the buffer supplied by the caller.
        len: usize,
    },
}
