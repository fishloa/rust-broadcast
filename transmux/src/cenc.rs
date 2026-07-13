//! Common Encryption boxes — ISO/IEC 23001-7 (CENC) + ISO/IEC 14496-12:2015 §8.12.
//!
//! # Types
//!
//! | Box     | Four-CC | Spec                        | Description                          |
//! |---------|---------|-----------------------------|--------------------------------------|
//! | `tenc`  | tenc    | 23001-7                     | Track Encryption Box (KID, IV size)  |
//! | `senc`  | senc    | 23001-7                     | Sample Encryption Box (per-sample IV)|
//! | `pssh`  | pssh    | 23001-7                     | Protection System Specific Header    |
//! | `saiz`  | saiz    | 14496-12 §8.7.8             | Sample Aux Info Sizes                |
//! | `saio`  | saio    | 14496-12 §8.7.9             | Sample Aux Info Offsets              |
//! | `sinf`  | sinf    | 14496-12 §8.12.1            | Protection Scheme Information Box    |
//! | `frma`  | frma    | 14496-12 §8.12.2            | Original Format Box                  |
//! | `schm`  | schm    | 14496-12 §8.12.5            | Scheme Type Box                      |
//! | `schi`  | schi    | 14496-12 §8.12.6            | Scheme Information Box               |
//!
//! # Spec citations
//!
//! - **tenc**: ISO/IEC 23001-7 §12.2 — TrackEncryptionBox.
//! - **senc**: ISO/IEC 23001-7 §12.3 — SampleEncryptionBox.
//! - **pssh**: ISO/IEC 23001-7 §12.1 — ProtectionSystemSpecificHeaderBox.
//! - **sinf/frma/schm/schi**: ISO/IEC 14496-12:2015 §8.12.
//! - **saiz**: ISO/IEC 14496-12:2015 §8.7.8.
//! - **saio**: ISO/IEC 14496-12:2015 §8.7.9.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

const BOX_HDR: usize = 8;
const FULL_HDR: usize = 4;

// ---------------------------------------------------------------------------
// CencScheme — protection scheme identity (ISO/IEC 23001-7 §4)
// ---------------------------------------------------------------------------

/// Four-CC identifying the `cenc` (AES-128 full-block counter, CTR) protection
/// scheme (`schm.scheme_type`). Only [`CencScheme::from_four_cc`] (the
/// decrypt path's `schm` recovery, gated on the `cenc` feature) reads this.
#[cfg(feature = "cenc")]
const SCHEME_CENC: [u8; 4] = *b"cenc";
/// Four-CC identifying the `cbcs` (AES-128 pattern cipher-block-chaining)
/// protection scheme (`schm.scheme_type`). Only [`CencScheme::from_four_cc`]
/// reads this.
#[cfg(feature = "cenc")]
const SCHEME_CBCS: [u8; 4] = *b"cbcs";

/// A CENC protection scheme (`schm.scheme_type`) — ISO/IEC 23001-7 §4.
///
/// Lives here (not in `cenc_decrypt`/`cenc_encrypt`) so the decrypt path
/// ([`crate::cenc_decrypt::CencDecryptor`], which recovers it from a
/// protected file's `schm`), the encrypt path
/// ([`crate::cenc_encrypt::CencEncryptor`], which is given it by the caller),
/// and the IR carrier ([`crate::media::TrackEncryption`]) all share one
/// definition rather than three (issue #564) — and it is available whether or
/// not the `cenc` feature (which only gates the AES cipher work) is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CencScheme {
    /// `cenc` — AES-128 full-block counter (CTR) mode.
    Cenc,
    /// `cbcs` — AES-128 pattern cipher-block-chaining mode.
    Cbcs,
}

impl CencScheme {
    /// The scheme's four-CC token as it appears in `schm` (`"cenc"` / `"cbcs"`).
    pub fn name(&self) -> &'static str {
        match self {
            CencScheme::Cenc => "cenc",
            CencScheme::Cbcs => "cbcs",
        }
    }

    /// Map a `schm.scheme_type` four-CC to a known scheme, if recognised.
    ///
    /// Only used by the decrypt path's `schm` recovery
    /// ([`crate::cenc_decrypt`]), hence gated on the `cenc` feature alongside
    /// it (this type itself stays available regardless of the feature).
    #[cfg(feature = "cenc")]
    pub(crate) fn from_four_cc(four_cc: &[u8; 4]) -> Option<Self> {
        match *four_cc {
            SCHEME_CENC => Some(CencScheme::Cenc),
            SCHEME_CBCS => Some(CencScheme::Cbcs),
            _ => None,
        }
    }
}

broadcast_common::impl_spec_display!(CencScheme);

// ---------------------------------------------------------------------------
// tenc — TrackEncryptionBox (ISO/IEC 23001-7 §12.2)
// ---------------------------------------------------------------------------

/// Track Encryption Box (`tenc`) — ISO/IEC 23001-7 §12.2.
///
/// Carries the default KID, per-sample IV size, and cryptic block parameters
/// (used only by pattern-based schemes like `cbcs` / `cens`; absent/zero for `cenc`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackEncryptionBox {
    pub version: u8,
    pub default_crypt_byte_block: u8,
    pub default_skip_byte_block: u8,
    pub default_is_protected: u8,
    pub default_per_sample_iv_size: u8,
    pub default_kid: [u8; 16],
    pub default_constant_iv: Option<Vec<u8>>,
}

impl TrackEncryptionBox {
    /// Parse a `tenc` box from FullBox body bytes (after 12-byte header).
    pub fn parse_body(bytes: &[u8], version: u8) -> Result<Self> {
        let min = 1 + 1 + 1 + 16;
        if bytes.len() < min {
            return Err(Error::BufferTooShort {
                need: min,
                have: bytes.len(),
                what: "tenc body",
            });
        }
        let mut offset = 0usize;
        let _reserved = bytes[offset];
        offset += 1;
        let (crypt_byte_block, skip_byte_block) = if version == 0 {
            let _reserved2 = bytes[offset];
            offset += 1;
            (0u8, 0u8)
        } else {
            let v = bytes[offset];
            offset += 1;
            (v >> 4, v & 0x0F)
        };
        let is_protected = bytes[offset];
        offset += 1;
        let iv_size = bytes[offset];
        offset += 1;
        let mut kid = [0u8; 16];
        kid.copy_from_slice(&bytes[offset..offset + 16]);
        offset += 16;

        let constant_iv = if is_protected == 1 && iv_size == 0 {
            if bytes.len() < offset + 1 {
                return Err(Error::BufferTooShort {
                    need: offset + 1,
                    have: bytes.len(),
                    what: "tenc constant_IV_size",
                });
            }
            let iv_len = bytes[offset] as usize;
            offset += 1;
            if bytes.len() < offset + iv_len {
                return Err(Error::BufferTooShort {
                    need: offset + iv_len,
                    have: bytes.len(),
                    what: "tenc constant_IV",
                });
            }
            let iv = bytes[offset..offset + iv_len].to_vec();
            Some(iv)
        } else {
            None
        };
        Ok(Self {
            version,
            default_crypt_byte_block: crypt_byte_block,
            default_skip_byte_block: skip_byte_block,
            default_is_protected: is_protected,
            default_per_sample_iv_size: iv_size,
            default_kid: kid,
            default_constant_iv: constant_iv,
        })
    }

    /// Parse a `tenc` box from a full box buffer (includes 8-byte box header + FullBox).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR + FULL_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR + FULL_HDR,
                have: bytes.len(),
                what: "tenc header",
            });
        }
        let version = bytes[8];
        let _flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Self::parse_body(&bytes[BOX_HDR + FULL_HDR..], version)
    }
}

impl Serialize for TrackEncryptionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 2 + 1 + 1 + 16;
        if let Some(ref iv) = self.default_constant_iv {
            n += 1 + iv.len();
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"tenc");
        c += 4;
        buf[c] = self.version;
        c += 1;
        buf[c] = 0;
        c += 1;
        buf[c] = 0;
        c += 1;
        buf[c] = 0;
        c += 1;
        // reserved (8)
        buf[c] = 0;
        c += 1;
        if self.version == 0 {
            buf[c] = 0;
            c += 1;
        } else {
            buf[c] = (self.default_crypt_byte_block << 4) | (self.default_skip_byte_block & 0x0F);
            c += 1;
        }
        buf[c] = self.default_is_protected;
        c += 1;
        buf[c] = self.default_per_sample_iv_size;
        c += 1;
        buf[c..c + 16].copy_from_slice(&self.default_kid);
        c += 16;
        if let Some(ref iv) = self.default_constant_iv {
            buf[c] = iv.len() as u8;
            c += 1;
            buf[c..c + iv.len()].copy_from_slice(iv);
            c += iv.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// senc — SampleEncryptionBox (ISO/IEC 23001-7 §12.3)
// ---------------------------------------------------------------------------

/// Per-sample encryption metadata for one sample.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleEncryptionEntry {
    pub initialization_vector: Vec<u8>,
    pub subsamples: Vec<SubSampleEntry>,
}

/// Sub-sample range (clear + protected regions) — ISO/IEC 23001-7 §12.3.2.
///
/// Used when the `UseSubSampleEncryption` flag (bit 1) is set in the `senc` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSampleEntry {
    pub bytes_of_clear_data: u16,
    pub bytes_of_protected_data: u32,
}

/// Flag: UseSubSampleEncryption (bit 1 of `senc` flags field).
pub const SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION: u32 = 0x000002;

/// Sample Encryption Box (`senc`) — ISO/IEC 23001-7 §12.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleEncryptionBox {
    pub version: u8,
    pub flags: u32,
    pub per_sample_iv_size: u8,
    pub entries: Vec<SampleEncryptionEntry>,
}

impl SampleEncryptionBox {
    /// Parse a `senc` box from FullBox body bytes, given the per-sample IV size.
    pub fn parse_body(
        bytes: &[u8],
        version: u8,
        flags: u32,
        per_sample_iv_size: u8,
    ) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: bytes.len(),
                what: "senc sample_count",
            });
        }
        let sample_count = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let mut offset = 4usize;
        let iv_sz = per_sample_iv_size as usize;
        let use_subs = (flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0;
        let mut entries = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            if bytes.len() < offset + iv_sz {
                return Err(Error::BufferTooShort {
                    need: offset + iv_sz,
                    have: bytes.len(),
                    what: "senc IV",
                });
            }
            let iv = bytes[offset..offset + iv_sz].to_vec();
            offset += iv_sz;
            let mut subsamples = Vec::new();
            if use_subs {
                if bytes.len() < offset + 2 {
                    return Err(Error::BufferTooShort {
                        need: offset + 2,
                        have: bytes.len(),
                        what: "senc subsample_count",
                    });
                }
                let sub_count = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
                offset += 2;
                for _ in 0..sub_count {
                    if bytes.len() < offset + 6 {
                        return Err(Error::BufferTooShort {
                            need: offset + 6,
                            have: bytes.len(),
                            what: "senc subsample",
                        });
                    }
                    let bytes_clear = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
                    offset += 2;
                    let bytes_protected = u32::from_be_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]);
                    offset += 4;
                    subsamples.push(SubSampleEntry {
                        bytes_of_clear_data: bytes_clear,
                        bytes_of_protected_data: bytes_protected,
                    });
                }
            }
            entries.push(SampleEncryptionEntry {
                initialization_vector: iv,
                subsamples,
            });
        }
        Ok(Self {
            version,
            flags,
            per_sample_iv_size,
            entries,
        })
    }
}

impl Serialize for SampleEncryptionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 4;
        for e in &self.entries {
            n += e.initialization_vector.len();
            if (self.flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0 {
                n += 2;
                n += e.subsamples.len() * 6;
            }
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"senc");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for e in &self.entries {
            buf[c..c + e.initialization_vector.len()].copy_from_slice(&e.initialization_vector);
            c += e.initialization_vector.len();
            if (self.flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0 {
                buf[c..c + 2].copy_from_slice(&(e.subsamples.len() as u16).to_be_bytes());
                c += 2;
                for s in &e.subsamples {
                    buf[c..c + 2].copy_from_slice(&s.bytes_of_clear_data.to_be_bytes());
                    c += 2;
                    buf[c..c + 4].copy_from_slice(&s.bytes_of_protected_data.to_be_bytes());
                    c += 4;
                }
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// pssh — ProtectionSystemSpecificHeaderBox (ISO/IEC 23001-7 §12.1)
// ---------------------------------------------------------------------------

/// Protection System Specific Header Box (`pssh`) — ISO/IEC 23001-7 §12.1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProtectionSystemSpecificHeaderBox {
    pub version: u8,
    pub system_id: [u8; 16],
    pub kids: Vec<[u8; 16]>,
    pub data: Vec<u8>,
}

impl ProtectionSystemSpecificHeaderBox {
    /// Parse a `pssh` box from full box bytes (8-byte box header + FullBox
    /// version/flags + body).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR + FULL_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR + FULL_HDR,
                have: bytes.len(),
                what: "pssh header",
            });
        }
        let version = bytes[8];
        Self::parse_body(&bytes[BOX_HDR + FULL_HDR..], version)
    }

    /// Serialize the whole box (header + FullBox + body) into a fresh `Vec`,
    /// rebuilding every length from the typed fields (no raw echo).
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        let n = self.serialize_into(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Parse a `pssh` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], version: u8) -> Result<Self> {
        if bytes.len() < 16 + 4 {
            return Err(Error::BufferTooShort {
                need: 16 + 4,
                have: bytes.len(),
                what: "pssh SystemID+DataSize",
            });
        }
        let mut system_id = [0u8; 16];
        system_id.copy_from_slice(&bytes[0..16]);
        let mut offset = 16usize;
        let mut kids = Vec::new();
        if version > 0 {
            if bytes.len() < offset + 4 {
                return Err(Error::BufferTooShort {
                    need: offset + 4,
                    have: bytes.len(),
                    what: "pssh KID_count",
                });
            }
            let kid_count = u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;
            let kid_needed = kid_count * 16;
            if bytes.len() < offset + kid_needed {
                return Err(Error::BufferTooShort {
                    need: offset + kid_needed,
                    have: bytes.len(),
                    what: "pssh KIDs",
                });
            }
            for i in 0..kid_count {
                let mut kid = [0u8; 16];
                kid.copy_from_slice(&bytes[offset + i * 16..offset + (i + 1) * 16]);
                kids.push(kid);
            }
            offset += kid_needed;
        }
        let data_size = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;
        if bytes.len() < offset + data_size {
            return Err(Error::BufferTooShort {
                need: offset + data_size,
                have: bytes.len(),
                what: "pssh Data",
            });
        }
        let data = bytes[offset..offset + data_size].to_vec();
        Ok(Self {
            version,
            system_id,
            kids,
            data,
        })
    }
}

impl Serialize for ProtectionSystemSpecificHeaderBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 16 + 4 + self.data.len();
        if self.version > 0 {
            n += 4 + self.kids.len() * 16;
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"pssh");
        c += 4;
        buf[c] = self.version;
        c += 1;
        buf[c] = 0;
        c += 1;
        buf[c] = 0;
        c += 1;
        buf[c] = 0;
        c += 1;
        buf[c..c + 16].copy_from_slice(&self.system_id);
        c += 16;
        if self.version > 0 {
            buf[c..c + 4].copy_from_slice(&(self.kids.len() as u32).to_be_bytes());
            c += 4;
            for kid in &self.kids {
                buf[c..c + 16].copy_from_slice(kid);
                c += 16;
            }
        }
        buf[c..c + 4].copy_from_slice(&(self.data.len() as u32).to_be_bytes());
        c += 4;
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        c += self.data.len();
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// saiz — SampleAuxiliaryInformationSizesBox (ISO/IEC 14496-12:2015 §8.7.8)
// ---------------------------------------------------------------------------

/// Sample Auxiliary Information Sizes Box (`saiz`) — §8.7.8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleAuxInfoSizesBox {
    pub version: u8,
    pub flags: u32,
    pub aux_info_type: Option<u32>,
    pub aux_info_type_parameter: Option<u32>,
    pub default_sample_info_size: u8,
    pub sample_info_sizes: Vec<u8>,
}

impl SampleAuxInfoSizesBox {
    /// Parse a `saiz` box from full box bytes (including 8-byte box header + FullBox).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR + FULL_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR + FULL_HDR,
                have: bytes.len(),
                what: "saiz header",
            });
        }
        let version = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Self::parse_body(&bytes[BOX_HDR + FULL_HDR..], version, flags)
    }

    /// Parse a `saiz` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], version: u8, flags: u32) -> Result<Self> {
        let mut offset = 0usize;
        let (aux_info_type, aux_info_type_parameter) = if (flags & 0x01) != 0 {
            if bytes.len() < 8 {
                return Err(Error::BufferTooShort {
                    need: 8,
                    have: bytes.len(),
                    what: "saiz aux_info_type",
                });
            }
            let ty = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let param = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            offset = 8;
            (Some(ty), Some(param))
        } else {
            (None, None)
        };
        if bytes.len() < offset + 1 + 4 {
            return Err(Error::BufferTooShort {
                need: offset + 5,
                have: bytes.len(),
                what: "saiz default_sample_info_size+sample_count",
            });
        }
        let default_size = bytes[offset];
        offset += 1;
        let sample_count = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;
        let sample_info_sizes = if default_size == 0 {
            if bytes.len() < offset + sample_count {
                return Err(Error::BufferTooShort {
                    need: offset + sample_count,
                    have: bytes.len(),
                    what: "saiz sample_info_size",
                });
            }
            bytes[offset..offset + sample_count].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version,
            flags,
            aux_info_type,
            aux_info_type_parameter,
            default_sample_info_size: default_size,
            sample_info_sizes,
        })
    }
}

impl Serialize for SampleAuxInfoSizesBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR;
        if (self.flags & 0x01) != 0 {
            n += 8;
        }
        n += 1 + 4;
        if self.default_sample_info_size == 0 {
            n += self.sample_info_sizes.len();
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"saiz");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if (self.flags & 0x01) != 0 {
            let ty = self.aux_info_type.unwrap_or(0);
            let param = self.aux_info_type_parameter.unwrap_or(0);
            buf[c..c + 4].copy_from_slice(&ty.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&param.to_be_bytes());
            c += 4;
        }
        buf[c] = self.default_sample_info_size;
        c += 1;
        let sample_count = self.sample_info_sizes.len() as u32;
        buf[c..c + 4].copy_from_slice(&sample_count.to_be_bytes());
        c += 4;
        if self.default_sample_info_size == 0 {
            buf[c..c + self.sample_info_sizes.len()].copy_from_slice(&self.sample_info_sizes);
            c += self.sample_info_sizes.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// saio — SampleAuxiliaryInformationOffsetsBox (ISO/IEC 14496-12:2015 §8.7.9)
// ---------------------------------------------------------------------------

/// Sample Auxiliary Information Offsets Box (`saio`) — §8.7.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleAuxInfoOffsetsBox {
    pub version: u8,
    pub flags: u32,
    pub aux_info_type: Option<u32>,
    pub aux_info_type_parameter: Option<u32>,
    pub offsets: Vec<u64>,
}

impl SampleAuxInfoOffsetsBox {
    /// Parse a `saio` box from full box bytes (including 8-byte box header + FullBox).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR + FULL_HDR {
            return Err(Error::BufferTooShort {
                need: BOX_HDR + FULL_HDR,
                have: bytes.len(),
                what: "saio header",
            });
        }
        let version = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Self::parse_body(&bytes[BOX_HDR + FULL_HDR..], version, flags)
    }

    /// Parse a `saio` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], version: u8, flags: u32) -> Result<Self> {
        let mut offset = 0usize;
        let (aux_info_type, aux_info_type_parameter) = if (flags & 0x01) != 0 {
            if bytes.len() < 8 {
                return Err(Error::BufferTooShort {
                    need: 8,
                    have: bytes.len(),
                    what: "saio aux_info_type",
                });
            }
            let ty = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let param = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            offset = 8;
            (Some(ty), Some(param))
        } else {
            (None, None)
        };
        if bytes.len() < offset + 4 {
            return Err(Error::BufferTooShort {
                need: offset + 4,
                have: bytes.len(),
                what: "saio entry_count",
            });
        }
        let entry_count = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;
        let offset_sz = if version == 0 { 4 } else { 8 };
        let offsets_needed = entry_count * offset_sz;
        if bytes.len() < offset + offsets_needed {
            return Err(Error::BufferTooShort {
                need: offset + offsets_needed,
                have: bytes.len(),
                what: "saio offsets",
            });
        }
        let mut offsets = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let o = if version == 0 {
                u32::from_be_bytes([
                    bytes[offset + i * 4],
                    bytes[offset + i * 4 + 1],
                    bytes[offset + i * 4 + 2],
                    bytes[offset + i * 4 + 3],
                ]) as u64
            } else {
                u64::from_be_bytes([
                    bytes[offset + i * 8],
                    bytes[offset + i * 8 + 1],
                    bytes[offset + i * 8 + 2],
                    bytes[offset + i * 8 + 3],
                    bytes[offset + i * 8 + 4],
                    bytes[offset + i * 8 + 5],
                    bytes[offset + i * 8 + 6],
                    bytes[offset + i * 8 + 7],
                ])
            };
            offsets.push(o);
        }
        Ok(Self {
            version,
            flags,
            aux_info_type,
            aux_info_type_parameter,
            offsets,
        })
    }
}

impl Serialize for SampleAuxInfoOffsetsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR;
        if (self.flags & 0x01) != 0 {
            n += 8;
        }
        n += 4;
        n += self.offsets.len() * if self.version == 0 { 4 } else { 8 };
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"saio");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if (self.flags & 0x01) != 0 {
            let ty = self.aux_info_type.unwrap_or(0);
            let param = self.aux_info_type_parameter.unwrap_or(0);
            buf[c..c + 4].copy_from_slice(&ty.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&param.to_be_bytes());
            c += 4;
        }
        buf[c..c + 4].copy_from_slice(&(self.offsets.len() as u32).to_be_bytes());
        c += 4;
        if self.version == 0 {
            for &off in &self.offsets {
                buf[c..c + 4].copy_from_slice(&(off as u32).to_be_bytes());
                c += 4;
            }
        } else {
            for &off in &self.offsets {
                buf[c..c + 8].copy_from_slice(&off.to_be_bytes());
                c += 8;
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// frma — Original Format Box (ISO/IEC 14496-12:2015 §8.12.2)
// ---------------------------------------------------------------------------

/// Original Format Box (`frma`) — §8.12.2.
///
/// Identifies the original (unprotected) codec Four-CC that the protection scheme
/// encrypts (e.g. `avc1`, `mp4a`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OriginalFormatBox {
    pub data_format: [u8; 4],
}

impl<'a> Parse<'a> for OriginalFormatBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < BOX_HDR + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HDR + 4,
                have: bytes.len(),
                what: "frma",
            });
        }
        let mut df = [0u8; 4];
        df.copy_from_slice(&bytes[BOX_HDR..BOX_HDR + 4]);
        Ok(Self { data_format: df })
    }
}

impl Serialize for OriginalFormatBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..4].copy_from_slice(&(need as u32).to_be_bytes());
        buf[4..8].copy_from_slice(b"frma");
        buf[8..12].copy_from_slice(&self.data_format);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// schm — Scheme Type Box (ISO/IEC 14496-12:2015 §8.12.5)
// ---------------------------------------------------------------------------

/// Scheme Type Box (`schm`) — §8.12.5.
///
/// Identifies the protection scheme (e.g. `cenc`), version, and URI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SchemeTypeBox {
    pub version: u8,
    pub flags: u32,
    pub scheme_type: [u8; 4],
    pub scheme_version: u32,
    pub scheme_uri: Option<Vec<u8>>,
}

impl SchemeTypeBox {
    /// Parse a `schm` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], _version: u8, flags: u32) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "schm body",
            });
        }
        let mut scheme_type = [0u8; 4];
        scheme_type.copy_from_slice(&bytes[0..4]);
        let scheme_version = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let scheme_uri = if (flags & 0x000001) != 0 {
            Some(bytes[8..].to_vec())
        } else {
            None
        };
        Ok(Self {
            version: _version,
            flags,
            scheme_type,
            scheme_version,
            scheme_uri,
        })
    }
}

impl Serialize for SchemeTypeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 8;
        if let Some(ref uri) = self.scheme_uri {
            n += uri.len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"schm");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&self.scheme_type);
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.scheme_version.to_be_bytes());
        c += 4;
        if let Some(ref uri) = self.scheme_uri {
            buf[c..c + uri.len()].copy_from_slice(uri);
            c += uri.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// schi — Scheme Information Box (ISO/IEC 14496-12:2015 §8.12.6)
// ---------------------------------------------------------------------------

/// Scheme Information Box (`schi`) — §8.12.6.
///
/// Container box for scheme-specific data (e.g. `tenc` inside `schi`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SchemeInformationBox {
    pub tenc: Option<TrackEncryptionBox>,
    pub extra_boxes: Vec<crate::init_segment::OpaqueBox>,
}

impl<'a> Parse<'a> for SchemeInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = &bytes[BOX_HDR..];
        let mut tenc = None;
        let mut extra_boxes = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let sz = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(body.len());
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            if &boxtype == b"tenc" {
                tenc = Some(TrackEncryptionBox::parse_box(&body[off..end])?);
            } else {
                extra_boxes.push(crate::init_segment::OpaqueBox {
                    box_type: boxtype,
                    data: body[off + 8..end].to_vec(),
                });
            }
            off += sz;
        }
        Ok(Self { tenc, extra_boxes })
    }
}

impl Serialize for SchemeInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        if let Some(ref t) = self.tenc {
            n += t.serialized_len();
        }
        for b in &self.extra_boxes {
            n += b.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"schi");
        c += 4;
        if let Some(ref t) = self.tenc {
            c += t.serialize_into(&mut buf[c..])?;
        }
        for b in &self.extra_boxes {
            c += b.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// sinf — Protection Scheme Information Box (ISO/IEC 14496-12:2015 §8.12.1)
// ---------------------------------------------------------------------------

/// Protection Scheme Information Box (`sinf`) — §8.12.1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProtectionSchemeInfoBox {
    pub original_format: OriginalFormatBox,
    pub scheme_type: Option<SchemeTypeBox>,
    pub scheme_info: Option<SchemeInformationBox>,
    pub extra_boxes: Vec<crate::init_segment::OpaqueBox>,
}

impl<'a> Parse<'a> for ProtectionSchemeInfoBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = &bytes[BOX_HDR..];
        if body.len() < 8 + 4 {
            return Err(Error::BufferTooShort {
                need: 8 + 4,
                have: bytes.len(),
                what: "sinf (frma)",
            });
        }
        let frma_sz = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
        let original_format = OriginalFormatBox::parse(&body[0..frma_sz])?;

        let mut off = frma_sz;
        let mut scheme_type = None;
        let mut scheme_info = None;
        let mut extra_boxes = Vec::new();

        while off + 8 <= body.len() {
            let sz = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(body.len());
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            match &boxtype {
                b"schm" => {
                    scheme_type = Some(SchemeTypeBox::parse_body(
                        &body[off + BOX_HDR + FULL_HDR..end],
                        body[off + BOX_HDR],
                        u32::from_be_bytes([
                            0,
                            body[off + BOX_HDR + 1],
                            body[off + BOX_HDR + 2],
                            body[off + BOX_HDR + 3],
                        ]),
                    )?);
                }
                b"schi" => {
                    scheme_info = Some(SchemeInformationBox::parse(&body[off..end])?);
                }
                _ => {
                    extra_boxes.push(crate::init_segment::OpaqueBox {
                        box_type: boxtype,
                        data: body[off + 8..end].to_vec(),
                    });
                }
            }
            off += sz;
        }
        Ok(Self {
            original_format,
            scheme_type,
            scheme_info,
            extra_boxes,
        })
    }
}

impl Serialize for ProtectionSchemeInfoBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        n += self.original_format.serialized_len();
        if let Some(ref st) = self.scheme_type {
            n += st.serialized_len();
        }
        if let Some(ref si) = self.scheme_info {
            n += si.serialized_len();
        }
        for b in &self.extra_boxes {
            n += b.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"sinf");
        c += 4;
        c += self.original_format.serialize_into(&mut buf[c..])?;
        if let Some(ref st) = self.scheme_type {
            c += st.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref si) = self.scheme_info {
            c += si.serialize_into(&mut buf[c..])?;
        }
        for b in &self.extra_boxes {
            c += b.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenc_body_round_trip() {
        let kid = [
            0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b,
            0xf3, 0xb8,
        ];
        let tenc = TrackEncryptionBox {
            version: 0,
            default_crypt_byte_block: 0,
            default_skip_byte_block: 0,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: kid,
            default_constant_iv: None,
        };
        let mut buf = vec![0u8; tenc.serialized_len()];
        tenc.serialize_into(&mut buf).unwrap();
        let parsed = TrackEncryptionBox::parse_box(&buf).unwrap();
        assert_eq!(parsed.version, 0);
        assert_eq!(parsed.default_is_protected, 1);
        assert_eq!(parsed.default_per_sample_iv_size, 8);
        assert_eq!(parsed.default_kid, kid);
        assert_eq!(parsed.default_constant_iv, None);
    }

    #[test]
    fn pssh_v0_round_trip() {
        let system_id = [
            0x10, 0x77, 0xef, 0xec, 0xc0, 0xb2, 0x4d, 0x02, 0xac, 0xe3, 0x3c, 0x1e, 0x52, 0xe2,
            0xfb, 0x4b,
        ];
        let data = vec![0x08, 0x01, 0x12, 0x10];
        let pssh = ProtectionSystemSpecificHeaderBox {
            version: 0,
            system_id,
            kids: Vec::new(),
            data,
        };
        let mut buf = vec![0u8; pssh.serialized_len()];
        pssh.serialize_into(&mut buf).unwrap();
        let parsed =
            ProtectionSystemSpecificHeaderBox::parse_body(&buf[BOX_HDR + FULL_HDR..], 0).unwrap();
        assert_eq!(parsed.system_id, system_id);
        assert_eq!(parsed.data, pssh.data);
        assert!(parsed.kids.is_empty());
    }

    #[test]
    fn mutating_tenc_kid_changes_bytes() {
        let kid = [0u8; 16];
        let tenc = TrackEncryptionBox {
            version: 0,
            default_crypt_byte_block: 0,
            default_skip_byte_block: 0,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: kid,
            default_constant_iv: None,
        };
        let original = {
            let mut buf = vec![0u8; tenc.serialized_len()];
            tenc.serialize_into(&mut buf).unwrap();
            buf
        };
        let mut mutated = tenc.clone();
        mutated.default_kid[0] = 0xFF;
        let mutated_bytes = {
            let mut buf = vec![0u8; mutated.serialized_len()];
            mutated.serialize_into(&mut buf).unwrap();
            buf
        };
        assert_ne!(original, mutated_bytes);
    }
}
