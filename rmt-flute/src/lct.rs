//! LCT — Layered Coding Transport header (RFC 5651 §5).
//!
//! Every packet of an LCT session carries a variable-size LCT header. The fixed
//! first 32 bits carry the version, the `C`/`PSI`/`S`/`O`/`H`/`A`/`B` flags,
//! `HDR_LEN`, and the Codepoint. After that come three variable-length fields
//! whose byte-sizes are driven **entirely** by the flags — this is the LCT
//! correctness point (RFC 5651 §5.1):
//!
//! - **CCI** (Congestion Control Information) = `32*(C+1)` bits → `4*(C+1)`
//!   bytes (4, 8, 12 or 16).
//! - **TSI** (Transport Session Identifier) = `32*S + 16*H` bits.
//! - **TOI** (Transport Object Identifier) = `32*O + 16*H` bits.
//!
//! The single `H` half-word flag feeds **both** the TSI and the TOI length
//! formulas independently (so the aggregate TSI+TOI length is always a whole
//! number of 32-bit words). After the variable fields, header extensions occupy
//! the space up to `HDR_LEN` words ([`crate::ext`]).

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::ext::{self, HeaderExtension, WORD};

/// LCT version number for RFC 5651.
pub const LCT_VERSION: u8 = 1;
/// Size in bytes of the fixed first word (V/C/PSI/S/O/H/Res/A/B + HDR_LEN + CP).
pub const FIXED_HEADER_LEN: usize = 4;

// First-word flag bits for the 16-bit packed field (RFC 5651 §5.1).
/// Bit mask for the `A` (Close Session) flag in the first header word.
const FLAG_A: u16 = 0x0002;
/// Bit mask for the `B` (Close Object) flag in the first header word.
const FLAG_B: u16 = 0x0001;

/// A decoded LCT header (RFC 5651 §5.1).
///
/// The flag-driven field widths are reconstructed from the typed fields on
/// serialize: `C` from `cci.len()`, `S`/`O`/`H` from `tsi`/`toi`, and `HDR_LEN`
/// from the total. None of the wire length/flag bytes are stored raw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LctHeader<'a> {
    /// LCT version (`V`). RFC 5651 = [`LCT_VERSION`] (1).
    pub version: u8,
    /// Protocol-Specific Indication (`PSI`, 2 bits). Meaning is per instantiation.
    pub psi: u8,
    /// Close Session flag (`A`).
    pub close_session: bool,
    /// Close Object flag (`B`).
    pub close_object: bool,
    /// Codepoint (`CP`, 8 bits) — opaque codec identifier.
    pub codepoint: u8,
    /// Congestion Control Information. Length is `4*(C+1)` bytes; `C` is derived
    /// as `cci.len()/4 − 1`, so it must be 4, 8, 12 or 16 bytes.
    pub cci: &'a [u8],
    /// Transport Session Identifier. Length `4*S + 2*H` bytes (0, 2, 4 or 6).
    pub tsi: &'a [u8],
    /// Transport Object Identifier. Length `4*O + 2*H` bytes (0, 2, 4, …, 14).
    pub toi: &'a [u8],
    /// Header-extension chain occupying the header space up to `HDR_LEN`.
    pub extensions: Vec<HeaderExtension<'a>>,
}

/// Decode `C`/`S`/`O`/`H` flags from the requested CCI/TSI/TOI byte lengths.
///
/// Returns `(c, s, o, h)`. The `H` half-word contributes 2 bytes to **both**
/// TSI and TOI, so the two lengths must agree on whether `H` is set.
fn flags_from_lengths(cci: usize, tsi: usize, toi: usize) -> Result<(u8, u8, u8, u8)> {
    // CCI: 4*(C+1) bytes → C in 0..=3.
    if cci == 0 || cci % WORD != 0 {
        return Err(Error::InvalidField {
            what: "CCI",
            reason: "CCI length must be a non-zero multiple of 4 bytes",
        });
    }
    let words = cci / WORD;
    if !(1..=4).contains(&words) {
        return Err(Error::InvalidField {
            what: "CCI",
            reason: "CCI length must be 4, 8, 12 or 16 bytes (C in 0..=3)",
        });
    }
    let c = (words - 1) as u8;

    // TSI = 4*S + 2*H bytes; TOI = 4*O + 2*H bytes. The half-word parity (odd
    // multiple of 2 bytes) determines H; it MUST match across TSI and TOI.
    let h_tsi = (tsi % WORD) != 0;
    let h_toi = (toi % WORD) != 0;
    if h_tsi != h_toi {
        return Err(Error::InvalidField {
            what: "H",
            reason: "TSI and TOI must agree on the shared half-word (H) bit",
        });
    }
    if tsi % 2 != 0 || toi % 2 != 0 {
        return Err(Error::InvalidField {
            what: "TSI/TOI",
            reason: "TSI and TOI lengths must be a whole number of 16-bit half-words",
        });
    }
    let h = u8::from(h_tsi);
    let s_bytes = tsi - (2 * h as usize);
    let o_bytes = toi - (2 * h as usize);
    let s = (s_bytes / WORD) as u8;
    let o = (o_bytes / WORD) as u8;
    if s > 1 {
        return Err(Error::InvalidField {
            what: "S",
            reason: "TSI 32-bit-word count (S) must be 0 or 1",
        });
    }
    if o > 7 {
        return Err(Error::InvalidField {
            what: "O",
            reason: "TOI 32-bit-word count (O) must be 0..=7",
        });
    }
    Ok((c, s, o, h))
}

impl<'a> LctHeader<'a> {
    /// CCI length in bytes = `4*(C+1)`.
    fn cci_len(c: u8) -> usize {
        WORD * (c as usize + 1)
    }
    /// TSI length in bytes = `4*S + 2*H`.
    fn tsi_len(s: u8, h: u8) -> usize {
        WORD * s as usize + 2 * h as usize
    }
    /// TOI length in bytes = `4*O + 2*H`.
    fn toi_len(o: u8, h: u8) -> usize {
        WORD * o as usize + 2 * h as usize
    }

    /// The `C` flag value (CCI words − 1), derived from the CCI length.
    pub fn c_flag(&self) -> u8 {
        (self.cci.len() / WORD).saturating_sub(1) as u8
    }
    /// The `H` half-word flag, derived from TSI/TOI parity.
    ///
    /// RFC 5651 §5.1: TSI and TOI **both** carry the half-word when `H` is set,
    /// so both must agree (the `flags_from_lengths` validator enforces this on
    /// serialize). A validly constructed header always has matching parity; we
    /// use `&&` to reflect the RFC constraint rather than the `||` that would
    /// mask a corrupted struct.
    pub fn h_flag(&self) -> u8 {
        u8::from((self.tsi.len() % WORD) != 0 && (self.toi.len() % WORD) != 0)
    }
    /// The `S` flag (full 32-bit words in TSI).
    pub fn s_flag(&self) -> u8 {
        (self.tsi.len() / WORD) as u8
    }
    /// The `O` flag (full 32-bit words in TOI).
    pub fn o_flag(&self) -> u8 {
        (self.toi.len() / WORD) as u8
    }

    /// Total bytes of the fixed + CCI + TSI + TOI portion (no extensions).
    fn base_len(&self) -> usize {
        FIXED_HEADER_LEN + self.cci.len() + self.tsi.len() + self.toi.len()
    }

    /// Total serialized length in bytes (fixed + CCI/TSI/TOI + extensions).
    pub fn serialized_len(&self) -> usize {
        self.base_len() + ext::chain_len(&self.extensions)
    }

    /// `HDR_LEN` as it appears on the wire — total header length in 32-bit words.
    pub fn hdr_len(&self) -> usize {
        self.serialized_len() / WORD
    }

    /// Parse an LCT header from the start of `data`. Reads `HDR_LEN` to find the
    /// end of the header (incl. extensions); trailing bytes (FEC Payload ID /
    /// payload) are left for the caller. Returns the header and bytes consumed.
    pub fn parse(data: &'a [u8]) -> Result<(Self, usize)> {
        if data.len() < FIXED_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_HEADER_LEN,
                have: data.len(),
                what: "LCT fixed header",
            });
        }
        // First 16 bits: V(4) C(2) PSI(2) S(1) O(2) H(1) Res(2) A(1) B(1).
        let w = u16::from_be_bytes([data[0], data[1]]);
        let version = (w >> 12) as u8 & 0x0F;
        let c = (w >> 10) as u8 & 0x03;
        let psi = (w >> 8) as u8 & 0x03;
        let s = (w >> 7) as u8 & 0x01;
        let o = (w >> 5) as u8 & 0x03;
        let h = (w >> 4) as u8 & 0x01;
        // Res = bits 2..3 (ignored). A = bit 1, B = bit 0.
        let close_session = (w & FLAG_A) != 0;
        let close_object = (w & FLAG_B) != 0;
        let hdr_len = data[2];
        let codepoint = data[3];

        let total = hdr_len as usize * WORD;
        if total < FIXED_HEADER_LEN {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "HDR_LEN smaller than the fixed header word",
            });
        }
        if data.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: data.len(),
                what: "LCT header (per HDR_LEN)",
            });
        }

        let cci_len = Self::cci_len(c);
        let tsi_len = Self::tsi_len(s, h);
        let toi_len = Self::toi_len(o, h);
        let base = FIXED_HEADER_LEN + cci_len + tsi_len + toi_len;
        if base > total {
            return Err(Error::InconsistentLength {
                length: hdr_len,
                reason: "HDR_LEN too small for the flag-derived CCI/TSI/TOI fields",
            });
        }

        let mut off = FIXED_HEADER_LEN;
        let cci = &data[off..off + cci_len];
        off += cci_len;
        let tsi = &data[off..off + tsi_len];
        off += tsi_len;
        let toi = &data[off..off + toi_len];
        off += toi_len;

        let extensions = ext::parse_chain(&data[off..total])?;

        Ok((
            LctHeader {
                version,
                psi,
                close_session,
                close_object,
                codepoint,
                cci,
                tsi,
                toi,
                extensions,
            },
            total,
        ))
    }

    /// Serialize the LCT header into `out`, recomputing the flag bits and
    /// `HDR_LEN` from the typed fields. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        if self.version > 0x0F {
            return Err(Error::FieldTooWide {
                what: "version",
                value: self.version as u64,
                bits: 4,
            });
        }
        if self.psi > 0x03 {
            return Err(Error::FieldTooWide {
                what: "PSI",
                value: self.psi as u64,
                bits: 2,
            });
        }
        // Validate the flag-derived widths (also yields C/S/O/H).
        let (c, s, o, h) = flags_from_lengths(self.cci.len(), self.tsi.len(), self.toi.len())?;

        let words = total / WORD;
        if total % WORD != 0 {
            return Err(Error::InvalidField {
                what: "HDR_LEN",
                reason: "total LCT header length is not a multiple of 4 bytes",
            });
        }
        if words > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "HDR_LEN",
                value: words as u64,
                bits: 8,
            });
        }

        // Pack the first 16-bit word MSB-first:
        // V(4) C(2) PSI(2) S(1) O(2) H(1) Res(2)=0 A(1) B(1).
        let mut w: u16 = 0;
        w |= (self.version as u16 & 0x0F) << 12;
        w |= (c as u16 & 0x03) << 10;
        w |= (self.psi as u16 & 0x03) << 8;
        w |= (s as u16 & 0x01) << 7;
        w |= (o as u16 & 0x03) << 5;
        w |= (h as u16 & 0x01) << 4;
        // Res (bits 2..3) = 0.
        if self.close_session {
            w |= FLAG_A;
        }
        if self.close_object {
            w |= FLAG_B;
        }
        out[0..2].copy_from_slice(&w.to_be_bytes());
        out[2] = words as u8;
        out[3] = self.codepoint;

        let mut off = FIXED_HEADER_LEN;
        out[off..off + self.cci.len()].copy_from_slice(self.cci);
        off += self.cci.len();
        out[off..off + self.tsi.len()].copy_from_slice(self.tsi);
        off += self.tsi.len();
        out[off..off + self.toi.len()].copy_from_slice(self.toi);
        off += self.toi.len();

        off += ext::serialize_chain(&self.extensions, &mut out[off..])?;
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // Minimal header: C=0 (CCI 4 bytes), S=0, O=0, H=0 (no TSI/TOI), no ext.
    #[test]
    fn minimal_header_exact_wire_bytes() {
        let cci = [0x00u8, 0x00, 0x00, 0x01];
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: false,
            close_object: false,
            codepoint: 0x00,
            cci: &cci,
            tsi: &[],
            toi: &[],
            extensions: vec![],
        };
        // HDR_LEN = (4 fixed + 4 cci) / 4 = 2.
        assert_eq!(hdr.hdr_len(), 2);
        let mut out = vec![0u8; hdr.serialized_len()];
        let n = hdr.serialize_into(&mut out).unwrap();
        assert_eq!(n, 8);
        // First word: V=1 (0x1000), all flags 0 → 0x1000. HDR_LEN=2, CP=0.
        assert_eq!(&out[0..4], &[0x10, 0x00, 0x02, 0x00]);
        assert_eq!(&out[4..8], &cci);
        let (re, used) = LctHeader::parse(&out).unwrap();
        assert_eq!(used, 8);
        assert_eq!(re, hdr);
    }

    // THE flag-dependent-size test: C=1 (CCI 8), S=1 + H=1 (TSI 6), O=1 + H=1
    // (TOI 6). All three widths differ from the minimal case.
    #[test]
    fn flag_dependent_widths_round_trip() {
        let cci = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11];
        let tsi = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06]; // 4*1 + 2*1 = 6 bytes
        let toi = [0x10u8, 0x20, 0x30, 0x40, 0x50, 0x60]; // 4*1 + 2*1 = 6 bytes
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0b10,
            close_session: true,
            close_object: false,
            codepoint: 0x42,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        assert_eq!(hdr.c_flag(), 1);
        assert_eq!(hdr.s_flag(), 1);
        assert_eq!(hdr.o_flag(), 1);
        assert_eq!(hdr.h_flag(), 1);

        // base = 4 + 8 + 6 + 6 = 24 → HDR_LEN 6.
        assert_eq!(hdr.serialized_len(), 24);
        assert_eq!(hdr.hdr_len(), 6);

        let mut out = vec![0u8; hdr.serialized_len()];
        let n = hdr.serialize_into(&mut out).unwrap();
        assert_eq!(n, 24);

        // Verify the packed first word bit-for-bit.
        // V=1 (1<<12=0x1000), C=1 (1<<10=0x0400), PSI=10 (2<<8=0x0200),
        // S=1 (1<<7=0x0080), O=1 (1<<5=0x0020), H=1 (1<<4=0x0010), A=1 (0x0002).
        let expect = 0x1000 | 0x0400 | 0x0200 | 0x0080 | 0x0020 | 0x0010 | 0x0002;
        assert_eq!(u16::from_be_bytes([out[0], out[1]]), expect);
        assert_eq!(out[2], 6); // HDR_LEN
        assert_eq!(out[3], 0x42); // CP

        let (re, used) = LctHeader::parse(&out).unwrap();
        assert_eq!(used, 24);
        assert_eq!(re, hdr);
        // The reparse must recover the exact widths.
        assert_eq!(re.cci.len(), 8);
        assert_eq!(re.tsi.len(), 6);
        assert_eq!(re.toi.len(), 6);
    }

    // H=0 vs H=1 must change BOTH TSI and TOI widths (the shared-half-word point).
    #[test]
    fn shared_h_bit_feeds_both_tsi_and_toi() {
        let cci = [0u8; 4];
        // H=1: TSI = 2 (S=0,H=1), TOI = 2 (O=0,H=1).
        let tsi = [0xABu8, 0xCD];
        let toi = [0x12u8, 0x34];
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        assert_eq!(hdr.h_flag(), 1);
        assert_eq!(hdr.s_flag(), 0);
        assert_eq!(hdr.o_flag(), 0);
        // base = 4 + 4 + 2 + 2 = 12 → 3 words.
        assert_eq!(hdr.hdr_len(), 3);
        let mut out = vec![0u8; hdr.serialized_len()];
        hdr.serialize_into(&mut out).unwrap();
        let (re, _) = LctHeader::parse(&out).unwrap();
        assert_eq!(re, hdr);
    }

    // Mutation bite: changing the codepoint changes the wire byte.
    #[test]
    fn mutating_codepoint_changes_wire() {
        let cci = [0u8; 4];
        let mk = |cp: u8| {
            let mut out = vec![0u8; 8];
            LctHeader {
                version: LCT_VERSION,
                psi: 0,
                close_session: false,
                close_object: false,
                codepoint: cp,
                cci: &cci,
                tsi: &[],
                toi: &[],
                extensions: vec![],
            }
            .serialize_into(&mut out)
            .unwrap();
            out
        };
        let a = mk(0x00);
        let b = mk(0x7F);
        assert_ne!(a, b);
        assert_eq!(a[3], 0x00);
        assert_eq!(b[3], 0x7F);
    }

    // Header WITH a 2-element extension chain round-trips and HDR_LEN grows.
    #[test]
    fn header_with_extension_chain() {
        let cci = [0u8; 4];
        let tsi = [0x00u8, 0x00, 0x00, 0x05]; // S=1, H=0
        let nop = [0u8; 2]; // EXT_NOP HEL=1: HET+HEL+2 = 4
        let ext_content = [0xAAu8, 0xBB, 0xCC]; // fixed ext (HET 200)
        let exts = vec![
            HeaderExtension::new(0, &nop),
            HeaderExtension::new(200, &ext_content),
        ];
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &[],
            extensions: exts,
        };
        // base = 4 + 4 + 4 + 0 = 12; ext = 4 + 4 = 8; total 20 → HDR_LEN 5.
        assert_eq!(hdr.serialized_len(), 20);
        assert_eq!(hdr.hdr_len(), 5);
        let mut out = vec![0u8; hdr.serialized_len()];
        hdr.serialize_into(&mut out).unwrap();
        let (re, used) = LctHeader::parse(&out).unwrap();
        assert_eq!(used, 20);
        assert_eq!(re, hdr);
        assert_eq!(re.extensions.len(), 2);
    }

    #[test]
    fn rejects_bad_cci_length() {
        let cci = [0u8; 3]; // not a multiple of 4
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &[],
            toi: &[],
            extensions: vec![],
        };
        let mut out = vec![0u8; 32];
        assert!(matches!(
            hdr.serialize_into(&mut out),
            Err(Error::InvalidField { .. })
        ));
    }

    #[test]
    fn rejects_mismatched_h() {
        // TSI has the half-word (odd 2-byte), TOI does not → H disagreement.
        let cci = [0u8; 4];
        let tsi = [0u8; 2]; // H=1
        let toi = [0u8; 4]; // H=0
        let hdr = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        let mut out = vec![0u8; 32];
        assert!(matches!(
            hdr.serialize_into(&mut out),
            Err(Error::InvalidField { what: "H", .. })
        ));
    }
}
