//! LCT / NORM Header-Extension chain (RFC 5651 §5.2, RFC 5740 §4.1).
//!
//! LCT and NORM share the same dual-format header-extension scheme, keyed on
//! the 8-bit **HET** (Header Extension Type) value range:
//!
//! - **HET 0..=127** — *variable-length*: `HET(8) | HEL(8) | content…`, where
//!   `HEL` is the length of the whole extension in 32-bit words (so the content
//!   is `4*HEL − 2` bytes). `HEL` MUST be ≥ 1.
//! - **HET 128..=255** — *fixed-length*: exactly one 32-bit word, with three
//!   content bytes directly after the HET byte and **no HEL** field.
//!
//! Every extension is a whole number of 32-bit words. This module models the
//! generic shape — a [`HeaderExtension`] holds the HET plus the raw content
//! bytes; typed views (`EXT_FDT`, `EXT_TIME`, …) interpret that content in
//! their own modules. The fixed-form interpretation of the three content bytes
//! differs between specs (LCT treats all 24 bits as HEC; NORM's first content
//! byte is a `reserved` octet), so the generic model keeps them opaque and the
//! typed wrappers do the splitting.

use alloc::vec::Vec;

use crate::error::{Error, Result};

/// The HET boundary: values `< FIXED_HET_MIN` are variable-length (carry an
/// HEL); values `>= FIXED_HET_MIN` are fixed-length (one 32-bit word, no HEL).
pub const FIXED_HET_MIN: u8 = 128;
/// Bytes in one 32-bit word.
pub const WORD: usize = 4;

/// One LCT/NORM header extension (RFC 5651 §5.2 / RFC 5740 §4.1).
///
/// The `content` is everything in the extension after the leading HET byte and
/// (for variable-length extensions) the HEL byte:
///
/// - variable-length (`het < 128`): `content.len()` is `4*HEL − 2`, and may be
///   any length whose total (`+2`) is a multiple of 4.
/// - fixed-length (`het >= 128`): `content` is exactly the 3 bytes after HET.
///
/// `HEL` is **not** stored: it is recomputed from `content.len()` on serialize,
/// so the round-trip is driven entirely from the typed fields (no raw
/// passthrough).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HeaderExtension<'a> {
    /// Header Extension Type.
    pub het: u8,
    /// The extension content bytes (after HET, and after HEL when present).
    pub content: &'a [u8],
}

impl<'a> HeaderExtension<'a> {
    /// Construct a variable- or fixed-length extension from its HET and content.
    pub fn new(het: u8, content: &'a [u8]) -> Self {
        HeaderExtension { het, content }
    }

    /// `true` if this is a fixed-length extension (HET ≥ 128, one 32-bit word).
    pub fn is_fixed(&self) -> bool {
        self.het >= FIXED_HET_MIN
    }

    /// Total serialized size in bytes (always a multiple of 4).
    pub fn serialized_len(&self) -> usize {
        if self.is_fixed() {
            WORD
        } else {
            // HET + HEL + content.
            2 + self.content.len()
        }
    }

    /// The `HEL` value as it appears on the wire (whole extension length in
    /// 32-bit words). Only meaningful for variable-length extensions.
    pub fn hel(&self) -> usize {
        self.serialized_len() / WORD
    }

    /// Parse a single header extension from the start of `data`. Returns the
    /// extension and the number of bytes consumed.
    pub fn parse(data: &'a [u8]) -> Result<(Self, usize)> {
        if data.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "header extension HET",
            });
        }
        let het = data[0];
        if het >= FIXED_HET_MIN {
            // Fixed-length: one 32-bit word, three content bytes after HET.
            if data.len() < WORD {
                return Err(Error::BufferTooShort {
                    need: WORD,
                    have: data.len(),
                    what: "fixed-length header extension",
                });
            }
            Ok((
                HeaderExtension {
                    het,
                    content: &data[1..WORD],
                },
                WORD,
            ))
        } else {
            // Variable-length: HET, HEL, then 4*HEL-2 content bytes.
            if data.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: data.len(),
                    what: "variable-length header extension HEL",
                });
            }
            let hel = data[1] as usize;
            if hel == 0 {
                return Err(Error::InvalidExtension {
                    reason: "HEL must be >= 1 for a variable-length extension",
                });
            }
            let total = hel * WORD;
            if data.len() < total {
                return Err(Error::BufferTooShort {
                    need: total,
                    have: data.len(),
                    what: "variable-length header extension content",
                });
            }
            Ok((
                HeaderExtension {
                    het,
                    content: &data[2..total],
                },
                total,
            ))
        }
    }

    /// Serialize this extension into `out`, recomputing `HEL` from the content
    /// length. Returns the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        out[0] = self.het;
        if self.is_fixed() {
            if self.content.len() != WORD - 1 {
                return Err(Error::InvalidExtension {
                    reason: "fixed-length extension content must be exactly 3 bytes",
                });
            }
            out[1..WORD].copy_from_slice(self.content);
            Ok(WORD)
        } else {
            // Whole extension must be a multiple of 4 bytes.
            if total % WORD != 0 {
                return Err(Error::InvalidExtension {
                    reason: "variable-length extension total must be a multiple of 4 bytes",
                });
            }
            let hel = total / WORD;
            if hel > u8::MAX as usize {
                return Err(Error::FieldTooWide {
                    what: "HEL",
                    value: hel as u64,
                    bits: 8,
                });
            }
            out[1] = hel as u8;
            out[2..total].copy_from_slice(self.content);
            Ok(total)
        }
    }
}

/// Parse a chain of header extensions occupying exactly `data`. Every byte of
/// `data` must be consumed; a trailing partial extension is an error.
pub fn parse_chain(mut data: &[u8]) -> Result<Vec<HeaderExtension<'_>>> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let (ext, n) = HeaderExtension::parse(data)?;
        out.push(ext);
        data = &data[n..];
    }
    Ok(out)
}

/// Total serialized length (bytes) of a header-extension chain.
pub fn chain_len(exts: &[HeaderExtension<'_>]) -> usize {
    exts.iter().map(|e| e.serialized_len()).sum()
}

/// Serialize a header-extension chain into `out`. Returns bytes written.
pub fn serialize_chain(exts: &[HeaderExtension<'_>], out: &mut [u8]) -> Result<usize> {
    let mut off = 0;
    for e in exts {
        off += e.serialize_into(&mut out[off..])?;
    }
    Ok(off)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn variable_ext_round_trip() {
        // EXT_NOP-like: HET=0, HEL=2, content = 6 bytes (2 + 6 = 8 = 2 words).
        let content = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66];
        let ext = HeaderExtension::new(0, &content);
        assert!(!ext.is_fixed());
        assert_eq!(ext.serialized_len(), 8);
        assert_eq!(ext.hel(), 2);

        let mut out = vec![0u8; ext.serialized_len()];
        let n = ext.serialize_into(&mut out).unwrap();
        assert_eq!(n, 8);
        assert_eq!(&out, &[0x00, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);

        let (re, used) = HeaderExtension::parse(&out).unwrap();
        assert_eq!(used, 8);
        assert_eq!(re, ext);
    }

    #[test]
    fn fixed_ext_round_trip() {
        // HET=192 (fixed), 3 content bytes.
        let content = [0xAAu8, 0xBB, 0xCC];
        let ext = HeaderExtension::new(192, &content);
        assert!(ext.is_fixed());
        assert_eq!(ext.serialized_len(), 4);

        let mut out = vec![0u8; 4];
        ext.serialize_into(&mut out).unwrap();
        assert_eq!(&out, &[0xC0, 0xAA, 0xBB, 0xCC]);

        let (re, used) = HeaderExtension::parse(&out).unwrap();
        assert_eq!(used, 4);
        assert_eq!(re, ext);
    }

    #[test]
    fn rejects_zero_hel() {
        let data = [0x00u8, 0x00, 0x00, 0x00];
        assert!(matches!(
            HeaderExtension::parse(&data),
            Err(Error::InvalidExtension { .. })
        ));
    }

    #[test]
    fn multi_extension_chain_round_trips() {
        // A 2+ element chain: a variable EXT (HET 1, HEL 2) then a fixed EXT.
        let c1 = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        let c2 = [0x02u8, 0x00, 0x00];
        let exts = vec![HeaderExtension::new(1, &c1), HeaderExtension::new(128, &c2)];
        let total = chain_len(&exts);
        assert_eq!(total, 8 + 4);

        let mut out = vec![0u8; total];
        let n = serialize_chain(&exts, &mut out).unwrap();
        assert_eq!(n, total);

        let parsed = parse_chain(&out).unwrap();
        assert_eq!(parsed, exts);
    }
}
