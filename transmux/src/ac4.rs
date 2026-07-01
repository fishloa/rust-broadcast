//! AC-4 in ISOBMFF — `ac-4` AudioSampleEntry + `dac4` config box.
//!
//! ETSI TS 103 190-2 Annex E (`transmux/docs/codec/ac4-dac4.md`): the
//! `AC4SpecificBox` (`dac4`) carries an `ac4_dsi_v1()` blob derived from the AC-4
//! TOC. `transmux` is samples-in, so — like `esds`/`avcC` — it takes the `dac4`
//! body as **caller-supplied opaque config** (the AC-4 TOC is not parsed here;
//! the sibling `rust-ac4` `ac4-si` crate is the TOC reference).

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the AC-4 config box.
pub const DAC4_FOURCC: [u8; 4] = *b"dac4";
/// FourCC of the AC-4 sample entry.
pub const AC4_FOURCC: [u8; 4] = *b"ac-4";

/// AC4SpecificBox (`dac4` box body) — ETSI TS 103 190-2 §E.5.
///
/// The body is the `ac4_dsi_v1()` byte string (Annex E.6), preserved verbatim so
/// the box round-trips byte-exact.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac4SpecificBox {
    /// The opaque `ac4_dsi_v1()` bytes.
    pub ac4_dsi: Vec<u8>,
}

impl Ac4SpecificBox {
    /// Build from a caller-supplied `ac4_dsi_v1()` blob.
    pub fn new(ac4_dsi: Vec<u8>) -> Self {
        Self { ac4_dsi }
    }

    /// RFC 6381 codec string — always the literal `"ac-4"`.
    pub fn rfc6381(&self) -> &'static str {
        "ac-4"
    }
}

impl<'a> Parse<'a> for Ac4SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            ac4_dsi: bytes.to_vec(),
        })
    }
}

impl Serialize for Ac4SpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.ac4_dsi.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..need].copy_from_slice(&self.ac4_dsi);
        Ok(need)
    }
}
