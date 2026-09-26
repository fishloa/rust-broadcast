//! Error types for the DVB-CSA crate.
use crate::ts::KeyParity;
use mpeg_ts::ts::ScramblingControl;
use thiserror::Error;

/// Errors that can occur during DVB-CSA operations.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum Error {
    /// The provided buffer is too short for the requested operation.
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort {
        /// The number of bytes required.
        need: usize,
        /// The number of bytes available.
        have: usize,
    },

    /// The packet carries no payload at all — `adaptation_field_control`
    /// selects adaptation-field-only (ISO/IEC 13818-1 Table 2-5), or the
    /// declared `adaptation_field_length` consumes the whole packet. This is
    /// a structurally ordinary packet (e.g. a PCR-only stuffing packet), not
    /// a truncated buffer, so it is distinct from [`Self::BufferTooShort`].
    #[error("packet carries no payload to (de)scramble")]
    NoPayload,

    /// [`crate::ts::scramble_ts_packet`] was asked to scramble a packet
    /// whose `transport_scrambling_control` is not already `00` (not
    /// scrambled) — scrambling it again would corrupt whatever the existing
    /// value protects instead of producing a legally descramblable packet.
    #[error("packet is already scrambled ({found})")]
    AlreadyScrambled {
        /// The packet's current `transport_scrambling_control` value.
        found: ScramblingControl,
    },

    /// [`crate::ts::descramble_ts_packet`] was asked to descramble a packet
    /// under a [`KeyParity`] that does not match the packet's own
    /// `transport_scrambling_control` — using the wrong one of a head-end's
    /// even/odd control-word pair would silently produce garbage instead of
    /// the original payload.
    #[error("packet is scrambled with {found}, not the requested {expected} key")]
    ParityMismatch {
        /// The parity the caller supplied a control word for.
        expected: KeyParity,
        /// The packet's actual `transport_scrambling_control` value.
        found: ScramblingControl,
    },
}
