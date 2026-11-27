//! Common Encryption boxes — ISO/IEC 23001-7 (CENC) + ISO/IEC 14496-12:2026 §13.4.
//!
//! # Types
//!
//! | Box     | Four-CC | Spec                        | Description                          |
//! |---------|---------|-----------------------------|--------------------------------------|
//! | `tenc`  | tenc    | 23001-7                     | Track Encryption Box (KID, IV size)  |
//! | `senc`  | senc    | 23001-7                     | Sample Encryption Box (per-sample IV)|
//! | `pssh`  | pssh    | 23001-7                     | Protection System Specific Header    |
//! | `saiz`  | saiz    | 14496-12:2026 §8.7.8        | Sample Aux Info Sizes                |
//! | `saio`  | saio    | 14496-12:2026 §8.7.9        | Sample Aux Info Offsets              |
//! | `sinf`  | sinf    | 14496-12:2026 §13.4.2       | Protection Scheme Information Box    |
//! | `frma`  | frma    | 14496-12:2026 §13.4.3       | Original Format Box                  |
//! | `schm`  | schm    | 14496-12:2026 §13.4.6       | Scheme Type Box                      |
//! | `schi`  | schi    | 14496-12:2026 §13.4.7       | Scheme Information Box               |
//!
//! # Spec citations
//!
//! - **tenc**: ISO/IEC 23001-7 §12.2 — TrackEncryptionBox.
//! - **senc**: ISO/IEC 23001-7 §12.3 — SampleEncryptionBox.
//! - **pssh**: ISO/IEC 23001-7 §12.1 — ProtectionSystemSpecificHeaderBox.
//! - **sinf/frma/schm/schi**: ISO/IEC 14496-12:2026 §13.4.
//! - **saiz**: ISO/IEC 14496-12:2026 §8.7.8 — version 0: 8-bit sizes; version 1: 16-bit; version 2: 32-bit.
//! - **saio**: ISO/IEC 14496-12:2026 §8.7.9.

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET, FULLBOX_VERSION_OFFSET,
};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_core::error::BufferTooShort;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{u16_at, u32_at, u64_at};
use broadcast_core::{Parse, Serialize};

/// Width of an AES-128 content key in bytes (ISO/IEC 23001-7 AES-128 content
/// encryption), the same width as a key id ([`crate::box_types::KEY_ID_LEN`]).
pub const KEY_LEN: usize = 16;

/// One-byte fields before `default_KID` in a `tenc` body (§12.2.2).
const TENC_BODY_HEADER_LEN: usize = 4;
/// Shift of `default_crypt_byte_block` in the shared crypt/skip byte.
const CRYPT_BLOCK_SHIFT: u32 = 4;
/// Mask of a nibble (crypt or skip block count).
const NIBBLE_MASK: u8 = 0x0F;
/// `aux_info_type` + `aux_info_type_parameter` width in `saiz`/`saio`.
const AUX_INFO_LEN: usize = 8;
/// Flag bit announcing `aux_info_type` in `saiz`/`saio`.
const AUX_INFO_TYPE_FLAG: u32 = 0x01;
/// Width of a `u32` count/length field (`sample_count`, `entry_count`, `DataSize`).
const COUNT_FIELD_LEN: usize = 4;
/// Clear-bytes field width in a `senc` subsample (§12.3.2).
const SENC_SUBSAMPLE_CLEAR_LEN: usize = 2;
/// Protected-bytes field width in a `senc` subsample (§12.3.2).
const SENC_SUBSAMPLE_PROTECTED_LEN: usize = 4;
/// Whole `senc` subsample width (clear + protected).
const SENC_SUBSAMPLE_LEN: usize = SENC_SUBSAMPLE_CLEAR_LEN + SENC_SUBSAMPLE_PROTECTED_LEN;
/// `schm` flag bit announcing a `scheme_uri` (§12.1.4).
/// `schm`/`csch` flag: `scheme_uri` is present (§13.4.6.2, §13.5.5.2).
pub const SCHM_FLAG_HAS_SCHEME_URI: u32 = 0x000001;

/// A FullBox's 24-bit `flags`, from a buffer starting at its `version` byte.
fn fullbox_flags(full: &[u8]) -> u32 {
    let mut word = [0u8; core::mem::size_of::<u32>()];
    word[FULLBOX_FLAGS_OFFSET..].copy_from_slice(&full[FULLBOX_FLAGS_OFFSET..FULLBOX_EXTRA_SIZE]);
    u32::from_be_bytes(word)
}

// ---------------------------------------------------------------------------
// CencScheme — protection scheme identity (ISO/IEC 23001-7 §4)
// ---------------------------------------------------------------------------

/// A CENC protection scheme (`schm.scheme_type`) — ISO/IEC 23001-7 §4.
///
/// Re-exported from [`broadcast_core::cenc`], which is where the single
/// definition lives. The decrypt path, the encrypt path, and the IR crypto
/// carrier share that one definition rather than three (issue #564) — and so
/// does `broadcast-hls`, which renders the HLS `#EXT-X-KEY` tag from it (issue
/// #878). The scheme
/// identity is container-independent, so it belongs below both this crate and
/// the playlist crate; only the *boxes* that carry it (`tenc`/`senc`/`schm`/…,
/// below) are ISOBMFF-specific and stay here.
pub use broadcast_core::cenc::CencScheme;

// ---------------------------------------------------------------------------
// tenc — TrackEncryptionBox (ISO/IEC 23001-7 §12.2)
// ---------------------------------------------------------------------------

/// Four-CC of the Track Encryption Box (`tenc`, ISO/IEC 23001-7 §12.2).
pub const TENC: [u8; 4] = *b"tenc";

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
    pub default_kid: [u8; crate::box_types::KEY_ID_LEN],
    pub default_constant_iv: Option<Vec<u8>>,
}

impl TrackEncryptionBox {
    /// Parse a `tenc` box from FullBox body bytes (after 12-byte header).
    pub fn parse_body(bytes: &[u8], version: u8) -> Result<Self> {
        // reserved(8) + reserved/crypt-skip(8) + is_protected(8) + iv_size(8)
        // + default_KID(128) — ISO/IEC 23001-7:2016 §12.2.2 Syntax. Four 1-byte
        // fields ahead of the 16-byte KID, not three: the guard undercounted by
        // one byte, so a 19-byte body passed it and then panicked on the KID
        // slice at offset 4..20 (audit finding #1).
        let min = TENC_BODY_HEADER_LEN + crate::box_types::KEY_ID_LEN;
        need_in(bytes, min, "tenc body")?;
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
            (v >> CRYPT_BLOCK_SHIFT, v & NIBBLE_MASK)
        };
        let is_protected = bytes[offset];
        offset += 1;
        let iv_size = bytes[offset];
        offset += 1;
        let mut kid = [0u8; crate::box_types::KEY_ID_LEN];
        kid.copy_from_slice(&bytes[offset..offset + crate::box_types::KEY_ID_LEN]);
        offset += crate::box_types::KEY_ID_LEN;

        let constant_iv = if is_protected == 1 && iv_size == 0 {
            need_in(bytes, offset + 1, "tenc constant_IV_size")?;
            let iv_len = bytes[offset] as usize;
            offset += 1;
            need_in(bytes, offset + iv_len, "tenc constant_IV")?;
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
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE,
            "tenc header",
        )?;
        let version = bytes[FULLBOX_VERSION_OFFSET];
        let _flags = fullbox_flags(&bytes[FULLBOX_VERSION_OFFSET..]);
        Self::parse_body(&bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..], version)
    }
}

impl Serialize for TrackEncryptionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE
            + FULLBOX_EXTRA_SIZE
            + TENC_BODY_HEADER_LEN
            + crate::box_types::KEY_ID_LEN;
        if let Some(ref iv) = self.default_constant_iv {
            n += 1 + iv.len();
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(buf, b"tenc", need, self.version, 0, true)?;
        // reserved (8)
        buf[c] = 0;
        c += 1;
        if self.version == 0 {
            buf[c] = 0;
            c += 1;
        } else {
            buf[c] = (self.default_crypt_byte_block << CRYPT_BLOCK_SHIFT)
                | (self.default_skip_byte_block & NIBBLE_MASK);
            c += 1;
        }
        buf[c] = self.default_is_protected;
        c += 1;
        buf[c] = self.default_per_sample_iv_size;
        c += 1;
        buf[c..c + crate::box_types::KEY_ID_LEN].copy_from_slice(&self.default_kid);
        c += crate::box_types::KEY_ID_LEN;
        if let Some(ref iv) = self.default_constant_iv {
            buf[c] = broadcast_core::len::fit_u8(iv.len(), "default_constant_IV_size")?;
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

/// Flag `0x000001` of the `senc` box: **override** the `TrackEncryptionBox`
/// parameters. When set, an `AlgorithmID(24) IV_size(8) KID(128)` triple
/// (20 bytes) precedes `sample_count` — the PIFF 1.1 / ISO/IEC 23001-7:2012 v0
/// form, still emitted by Smooth-derived and older PIFF tooling.
///
/// This crate does not implement the override (it decrypts with the track-wide
/// `tenc`), and reading such a box as if the flag were clear consumes the 20
/// override bytes as `sample_count` and the first entries, yielding garbage IVs
/// or a spurious length error. [`SampleEncryptionBox::parse_body`] rejects the
/// flag explicitly so the failure is a clear `Err` rather than wrong plaintext
/// with `Ok`.
pub const SENC_FLAG_OVERRIDE_TRACK_ENCRYPTION: u32 = 0x000001;

/// The fixed size of the `senc` override triple
/// ([`SENC_FLAG_OVERRIDE_TRACK_ENCRYPTION`]): `AlgorithmID(24) + IV_size(8) +
/// KID(128)` = 20 bytes.
#[cfg(test)]
pub const SENC_OVERRIDE_FIELDS_LEN: usize = 20;

/// Four-CC of the Sample Encryption Box (`senc`, ISO/IEC 23001-7 §12.3).
pub const SENC: [u8; 4] = *b"senc";

/// Sample Encryption Box (`senc`) — ISO/IEC 23001-7 §12.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleEncryptionBox {
    pub version: u8,
    pub flags: u32,
    pub per_sample_iv_size: u8,
    pub entries: Vec<SampleEncryptionEntry>,
}

/// Bytes an entry's `sample_count` field occupies in a `senc` body — ISO/IEC
/// 23001-7 §12.3.
const SENC_SAMPLE_COUNT_LEN: usize = 4;
/// Bytes a `senc` entry's `subsample_count` field occupies when the
/// [`SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION`] flag is set — ISO/IEC 23001-7 §12.3.2.
const SENC_SUBSAMPLE_COUNT_LEN: usize = 2;

impl SampleEncryptionBox {
    /// Parse a `senc` box from its full box bytes (8-byte header + FullBox +
    /// body), given the track's `tenc.default_per_sample_iv_size`.
    pub fn parse_box(bytes: &[u8], per_sample_iv_size: u8) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE,
            "senc header",
        )?;
        let version = bytes[FULLBOX_VERSION_OFFSET];
        let flags = fullbox_flags(&bytes[FULLBOX_VERSION_OFFSET..]);
        Self::parse_body(
            &bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..],
            version,
            flags,
            per_sample_iv_size,
        )
    }

    /// Parse a `senc` box from FullBox body bytes, given the per-sample IV size.
    ///
    /// `sample_count` is an untrusted 32-bit field straight off the wire, so it
    /// is bounded against the box body's own length **before** anything is
    /// allocated: each entry occupies at least its per-sample IV plus (when
    /// `UseSubSampleEncryption` is set) its 2-byte `subsample_count`, so a body
    /// too short to hold `sample_count` such minima is rejected outright.
    /// Without that bound a 20-byte box declaring `sample_count == 0xFFFFFFFF`
    /// asks for ~4.3 billion entries up front (hundreds of GB) and aborts the
    /// process — a remote denial of service, since this parser reads
    /// third-party media.
    ///
    /// A `senc` whose entries would carry *no* bytes at all (no per-sample IV
    /// **and** no subsample map) is rejected for the same reason: it declares a
    /// sample count the wire cannot corroborate at any length, while carrying
    /// no per-sample information for a decryptor to use. ISO/IEC 23001-7 §12.3
    /// gives the box exactly two jobs — per-sample IVs and subsample maps — so
    /// a track with a constant IV and whole-sample protection has no `senc` to
    /// write in the first place.
    pub fn parse_body(
        bytes: &[u8],
        version: u8,
        flags: u32,
        per_sample_iv_size: u8,
    ) -> Result<Self> {
        // `senc` flag 0x000001 (`override TrackEncryptionBox parameters`, the
        // PIFF 1.1 / ISO 23001-7:2012 v0 form) inserts a 20-byte
        // AlgorithmID/IV_size/KID triple before `sample_count`. This crate
        // decrypts with the track-wide `tenc` only, so it cannot honour the
        // override; reading the box as if the flag were clear would take those
        // 20 bytes as `sample_count` and the first entries and produce garbage
        // IVs — or a spurious length error. Reject explicitly (audit r05-W9).
        if (flags & SENC_FLAG_OVERRIDE_TRACK_ENCRYPTION) != 0 {
            return Err(Error::UnsupportedFeature(concat!(
                "senc flag 0x000001 (override TrackEncryptionBox parameters, PIFF 1.1 / ",
                "ISO/IEC 23001-7:2012 v0) is not supported: the track-wide tenc is the only ",
                "encryption metadata this crate decrypts with",
            )));
        }
        need_in(bytes, SENC_SAMPLE_COUNT_LEN, "senc sample_count")?;
        let sample_count = u32_at(bytes, 0) as usize;
        let mut offset = SENC_SAMPLE_COUNT_LEN;
        let iv_sz = per_sample_iv_size as usize;
        let use_subs = (flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0;

        // Bound the untrusted `sample_count` against what the body can hold
        // before allocating for it (see the doc comment above).
        let min_entry_len = iv_sz
            + if use_subs {
                SENC_SUBSAMPLE_COUNT_LEN
            } else {
                0
            };
        if min_entry_len == 0 {
            if sample_count != 0 {
                return Err(Error::InvalidInput(
                    "senc declares samples but carries no per-sample data (per_sample_iv_size == 0 \
                     with UseSubSampleEncryption clear), so sample_count cannot be verified against \
                     the box length",
                ));
            }
        } else {
            let need = sample_count
                .checked_mul(min_entry_len)
                .and_then(|n| n.checked_add(offset))
                .ok_or(Error::InvalidInput("senc sample_count overflows the body"))?;
            need_in(
                bytes,
                need,
                "senc entries (sample_count exceeds what the box can hold)",
            )?;
        }

        let mut entries = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            need_in(bytes, offset + iv_sz, "senc IV")?;
            let iv = bytes[offset..offset + iv_sz].to_vec();
            offset += iv_sz;
            let mut subsamples = Vec::new();
            if use_subs {
                need_in(
                    bytes,
                    offset + SENC_SUBSAMPLE_COUNT_LEN,
                    "senc subsample_count",
                )?;
                let sub_count = u16_at(bytes, offset) as usize;
                offset += SENC_SUBSAMPLE_COUNT_LEN;
                for _ in 0..sub_count {
                    need_in(bytes, offset + SENC_SUBSAMPLE_LEN, "senc subsample")?;
                    let bytes_clear = u16_at(bytes, offset);
                    offset += SENC_SUBSAMPLE_CLEAR_LEN;
                    let bytes_protected = u32_at(bytes, offset);
                    offset += SENC_SUBSAMPLE_PROTECTED_LEN;
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
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + SENC_SAMPLE_COUNT_LEN;
        for e in &self.entries {
            n += e.initialization_vector.len();
            if (self.flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0 {
                n += SENC_SUBSAMPLE_COUNT_LEN;
                n += e.subsamples.len() * SENC_SUBSAMPLE_LEN;
            }
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"senc",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let sample_count = broadcast_core::len::fit_u32(self.entries.len(), "sample_count")?;
        buf[c..c + SENC_SAMPLE_COUNT_LEN].copy_from_slice(&sample_count.to_be_bytes());
        c += SENC_SAMPLE_COUNT_LEN;
        for e in &self.entries {
            buf[c..c + e.initialization_vector.len()].copy_from_slice(&e.initialization_vector);
            c += e.initialization_vector.len();
            if (self.flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION) != 0 {
                let subsample_count =
                    broadcast_core::len::fit_u16(e.subsamples.len(), "subsample_count")?;
                buf[c..c + SENC_SUBSAMPLE_COUNT_LEN]
                    .copy_from_slice(&subsample_count.to_be_bytes());
                c += SENC_SUBSAMPLE_COUNT_LEN;
                for s in &e.subsamples {
                    buf[c..c + SENC_SUBSAMPLE_CLEAR_LEN]
                        .copy_from_slice(&s.bytes_of_clear_data.to_be_bytes());
                    c += SENC_SUBSAMPLE_CLEAR_LEN;
                    buf[c..c + SENC_SUBSAMPLE_PROTECTED_LEN]
                        .copy_from_slice(&s.bytes_of_protected_data.to_be_bytes());
                    c += SENC_SUBSAMPLE_PROTECTED_LEN;
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
    pub system_id: [u8; crate::box_types::KEY_ID_LEN],
    pub kids: Vec<[u8; crate::box_types::KEY_ID_LEN]>,
    pub data: Vec<u8>,
}

impl ProtectionSystemSpecificHeaderBox {
    /// Parse a `pssh` box from full box bytes (8-byte box header + FullBox
    /// version/flags + body).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE,
            "pssh header",
        )?;
        let version = bytes[FULLBOX_VERSION_OFFSET];
        Self::parse_body(&bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..], version)
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
        need_in(
            bytes,
            COUNT_FIELD_LEN + crate::box_types::KEY_ID_LEN,
            "pssh SystemID+DataSize",
        )?;
        let mut system_id = [0u8; crate::box_types::KEY_ID_LEN];
        system_id.copy_from_slice(&bytes[0..crate::box_types::KEY_ID_LEN]);
        let mut offset = crate::box_types::KEY_ID_LEN;
        let mut kids = Vec::new();
        if version > 0 {
            need_in(bytes, offset + COUNT_FIELD_LEN, "pssh KID_count")?;
            let kid_count = u32_at(bytes, offset) as usize;
            offset += COUNT_FIELD_LEN;
            // KID_count is wire-controlled: checked arithmetic before the
            // length check so no wrap past it and no huge allocation (r05-C3).
            let kid_needed = kid_count
                .checked_mul(crate::box_types::KEY_ID_LEN)
                .ok_or(Error::InvalidInput("pssh KID_count overflows the body"))?;
            let kid_end = offset
                .checked_add(kid_needed)
                .ok_or(Error::InvalidInput("pssh KID_count overflows the body"))?;
            need_in(bytes, kid_end, "pssh KIDs")?;
            for i in 0..kid_count {
                let mut kid = [0u8; crate::box_types::KEY_ID_LEN];
                let at = offset + i * crate::box_types::KEY_ID_LEN;
                kid.copy_from_slice(&bytes[at..at + crate::box_types::KEY_ID_LEN]);
                kids.push(kid);
            }
            offset = kid_end;
        }
        // A v1 body can end right after the KID table, leaving no DataSize
        // field to read (r05-C3).
        need_in(bytes, offset + COUNT_FIELD_LEN, "pssh DataSize")?;
        let data_size = u32_at(bytes, offset) as usize;
        offset += COUNT_FIELD_LEN;
        let data_end = offset
            .checked_add(data_size)
            .ok_or(Error::InvalidInput("pssh DataSize overflows the body"))?;
        need_in(bytes, data_end, "pssh Data")?;
        let data = bytes[offset..data_end].to_vec();
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
        let mut n = BOX_HEADER_MIN_SIZE
            + FULLBOX_EXTRA_SIZE
            + crate::box_types::KEY_ID_LEN
            + COUNT_FIELD_LEN
            + self.data.len();
        if self.version > 0 {
            n += COUNT_FIELD_LEN + self.kids.len() * crate::box_types::KEY_ID_LEN;
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(buf, b"pssh", need, self.version, 0, false)?;
        buf[c..c + crate::box_types::KEY_ID_LEN].copy_from_slice(&self.system_id);
        c += crate::box_types::KEY_ID_LEN;
        if self.version > 0 {
            let kid_count = broadcast_core::len::fit_u32(self.kids.len(), "KID_count")?;
            buf[c..c + COUNT_FIELD_LEN].copy_from_slice(&kid_count.to_be_bytes());
            c += COUNT_FIELD_LEN;
            for kid in &self.kids {
                buf[c..c + crate::box_types::KEY_ID_LEN].copy_from_slice(kid);
                c += crate::box_types::KEY_ID_LEN;
            }
        }
        let data_size = broadcast_core::len::fit_u32(self.data.len(), "DataSize")?;
        buf[c..c + COUNT_FIELD_LEN].copy_from_slice(&data_size.to_be_bytes());
        c += COUNT_FIELD_LEN;
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        c += self.data.len();
        Ok(c)
    }
}

/// `saiz` version 0, 1, 2: 8-, 16-, 32-bit sizes — ISO/IEC 14496-12:2026 §8.7.8.2.
const SAIZ_VERSION_MAX: u8 = 2;

fn saiz_size_width(version: u8) -> Result<usize> {
    if version > SAIZ_VERSION_MAX {
        return Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "saiz version",
            value: u64::from(version),
            reason: "ISO/IEC 14496-12:2026 §8.7.8.2 defines versions 0 to 2",
        }));
    }
    Ok(1usize << version)
}

fn read_be(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, b| (acc << u8::BITS) | u32::from(*b))
}

fn write_be(
    buf: &mut [u8],
    c: &mut usize,
    value: u32,
    width: usize,
    field: &'static str,
) -> Result<()> {
    broadcast_core::len::fit_bits(u64::from(value), (width as u32) * u8::BITS, field)?;
    buf[*c..*c + width]
        .copy_from_slice(&value.to_be_bytes()[core::mem::size_of::<u32>() - width..]);
    *c += width;
    Ok(())
}

// ---------------------------------------------------------------------------
// saiz — SampleAuxiliaryInformationSizesBox (ISO/IEC 14496-12:2026 §8.7.8)
// ---------------------------------------------------------------------------

/// Four-CC of the Sample Auxiliary Information Sizes Box (`saiz`, ISO/IEC 14496-12:2026 §8.7.8).
pub const SAIZ: [u8; 4] = *b"saiz";

/// Sample Auxiliary Information Sizes Box (`saiz`) — §8.7.8.
///
/// `sample_count` (wire `unsigned int(32)`) is carried explicitly: unlike
/// `sample_info_sizes.len()`, it does not collapse to 0 when
/// `default_sample_info_size != 0` (the uniform-size form, where the wire
/// carries a count but no per-sample table). Round-tripping a uniform-size
/// `saiz` through `sample_info_sizes.len()` alone silently declared every
/// such box "0 samples of aux info" (issue #1013).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleAuxInfoSizesBox {
    pub version: u8,
    pub flags: u32,
    pub aux_info_type: Option<u32>,
    pub aux_info_type_parameter: Option<u32>,
    /// Width on the wire follows `version`: 8, 16 or 32 bits (§8.7.8.2).
    pub default_sample_info_size: u32,
    pub sample_count: u32,
    /// Same width as `default_sample_info_size`.
    pub sample_info_sizes: Vec<u32>,
}

impl SampleAuxInfoSizesBox {
    /// Parse a `saiz` box from full box bytes (including 8-byte box header + FullBox).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE,
            "saiz header",
        )?;
        let version = bytes[FULLBOX_VERSION_OFFSET];
        let flags = fullbox_flags(&bytes[FULLBOX_VERSION_OFFSET..]);
        Self::parse_body(
            &bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..],
            version,
            flags,
        )
    }

    /// Parse a `saiz` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], version: u8, flags: u32) -> Result<Self> {
        let mut offset = 0usize;
        let (aux_info_type, aux_info_type_parameter) = if (flags & AUX_INFO_TYPE_FLAG) != 0 {
            need_in(bytes, AUX_INFO_LEN, "saiz aux_info_type")?;
            let ty = u32_at(bytes, 0);
            let param = u32_at(bytes, core::mem::size_of::<u32>());
            offset = AUX_INFO_LEN;
            (Some(ty), Some(param))
        } else {
            (None, None)
        };
        let width = saiz_size_width(version)?;
        need_in(
            bytes,
            offset + width + COUNT_FIELD_LEN,
            "saiz default_sample_info_size+sample_count",
        )?;
        let default_size = read_be(&bytes[offset..offset + width]);
        offset += width;
        let sample_count_wire = u32_at(bytes, offset);
        offset += COUNT_FIELD_LEN;
        let sample_info_sizes = if default_size == 0 {
            let sample_count = sample_count_wire as usize;
            let table = sample_count
                .checked_mul(width)
                .ok_or(Error::InvalidInput("saiz sample_count overflows the body"))?;
            need_in(bytes, offset + table, "saiz sample_info_size")?;
            bytes[offset..offset + table]
                .chunks_exact(width)
                .map(read_be)
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            version,
            flags,
            aux_info_type,
            aux_info_type_parameter,
            default_sample_info_size: default_size,
            sample_count: sample_count_wire,
            sample_info_sizes,
        })
    }
}

impl Serialize for SampleAuxInfoSizesBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
        if (self.flags & AUX_INFO_TYPE_FLAG) != 0 {
            n += AUX_INFO_LEN;
        }
        let width = saiz_size_width(self.version).unwrap_or(1);
        n += width + COUNT_FIELD_LEN;
        if self.default_sample_info_size == 0 {
            n += self.sample_info_sizes.len() * width;
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"saiz",
            need,
            self.version,
            self.flags,
            false,
        )?;
        if (self.flags & AUX_INFO_TYPE_FLAG) != 0 {
            let ty = self.aux_info_type.unwrap_or(0);
            let param = self.aux_info_type_parameter.unwrap_or(0);
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&ty.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&param.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let width = saiz_size_width(self.version)?;
        write_be(
            buf,
            &mut c,
            self.default_sample_info_size,
            width,
            "default_sample_info_size",
        )?;
        if self.default_sample_info_size == 0
            && self.sample_info_sizes.len() != self.sample_count as usize
        {
            return Err(Error::InvalidInput(
                "saiz: sample_info_sizes.len() must equal sample_count when default_sample_info_size == 0",
            ));
        }
        buf[c..c + COUNT_FIELD_LEN].copy_from_slice(&self.sample_count.to_be_bytes());
        c += COUNT_FIELD_LEN;
        if self.default_sample_info_size == 0 {
            for &v in &self.sample_info_sizes {
                write_be(buf, &mut c, v, width, "sample_info_size")?;
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// saio — SampleAuxiliaryInformationOffsetsBox (ISO/IEC 14496-12:2026 §8.7.9)
// ---------------------------------------------------------------------------

/// Four-CC of the Sample Auxiliary Information Offsets Box (`saio`, ISO/IEC 14496-12:2026 §8.7.9).
pub const SAIO: [u8; 4] = *b"saio";

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
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE,
            "saio header",
        )?;
        let version = bytes[FULLBOX_VERSION_OFFSET];
        let flags = fullbox_flags(&bytes[FULLBOX_VERSION_OFFSET..]);
        Self::parse_body(
            &bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..],
            version,
            flags,
        )
    }

    /// Parse a `saio` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], version: u8, flags: u32) -> Result<Self> {
        let mut offset = 0usize;
        let (aux_info_type, aux_info_type_parameter) = if (flags & AUX_INFO_TYPE_FLAG) != 0 {
            need_in(bytes, AUX_INFO_LEN, "saio aux_info_type")?;
            let ty = u32_at(bytes, 0);
            let param = u32_at(bytes, core::mem::size_of::<u32>());
            offset = AUX_INFO_LEN;
            (Some(ty), Some(param))
        } else {
            (None, None)
        };
        need_in(bytes, offset + COUNT_FIELD_LEN, "saio entry_count")?;
        let entry_count = u32_at(bytes, offset) as usize;
        offset += COUNT_FIELD_LEN;
        let offset_sz = if version == 0 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u64>()
        };
        // `entry_count` is an untrusted 32-bit field: on a 32-bit target
        // `entry_count * 8` wraps, which would defeat the length check below
        // and let `Vec::with_capacity` be asked for a hostile count.
        let offsets_needed = entry_count
            .checked_mul(offset_sz)
            .ok_or(Error::InvalidInput("saio entry_count overflows the body"))?;
        need_in(bytes, offset + offsets_needed, "saio offsets")?;
        let mut offsets = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let o = if version == 0 {
                u64::from(u32_at(bytes, offset + i * core::mem::size_of::<u32>()))
            } else {
                u64_at(bytes, offset + i * core::mem::size_of::<u64>())
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
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
        if (self.flags & AUX_INFO_TYPE_FLAG) != 0 {
            n += AUX_INFO_LEN;
        }
        n += COUNT_FIELD_LEN;
        n += self.offsets.len()
            * if self.version == 0 {
                core::mem::size_of::<u32>()
            } else {
                core::mem::size_of::<u64>()
            };
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"saio",
            need,
            self.version,
            self.flags,
            false,
        )?;
        if (self.flags & AUX_INFO_TYPE_FLAG) != 0 {
            let ty = self.aux_info_type.unwrap_or(0);
            let param = self.aux_info_type_parameter.unwrap_or(0);
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&ty.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&param.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let entry_count = broadcast_core::len::fit_u32(self.offsets.len(), "entry_count")?;
        buf[c..c + COUNT_FIELD_LEN].copy_from_slice(&entry_count.to_be_bytes());
        c += COUNT_FIELD_LEN;
        if self.version == 0 {
            for &off in &self.offsets {
                let off32 = broadcast_core::len::fit_bits(off, u32::BITS, "offset")? as u32;
                buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&off32.to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
        } else {
            for &off in &self.offsets {
                buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&off.to_be_bytes());
                c += core::mem::size_of::<u64>();
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// frma — Original Format Box (ISO/IEC 14496-12:2026 §13.4.3)
// ---------------------------------------------------------------------------

/// Original Format Box (`frma`) — §13.4.3.
///
/// Identifies the original (unprotected) codec Four-CC that the protection scheme
/// encrypts (e.g. `avc1`, `mp4a`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OriginalFormatBox {
    pub data_format: [u8; crate::box_types::BOX_TYPE_SIZE],
}

impl<'a> Parse<'a> for OriginalFormatBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE,
            "frma",
        )?;
        let mut df = [0u8; crate::box_types::BOX_TYPE_SIZE];
        df.copy_from_slice(
            &bytes[BOX_HEADER_MIN_SIZE..BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE],
        );
        Ok(Self { data_format: df })
    }
}

impl Serialize for OriginalFormatBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[..core::mem::size_of::<u32>()].copy_from_slice(&(need as u32).to_be_bytes());
        buf[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(b"frma");
        buf[BOX_HEADER_MIN_SIZE..BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE]
            .copy_from_slice(&self.data_format);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// schm — Scheme Type Box (ISO/IEC 14496-12:2026 §13.4.6)
// ---------------------------------------------------------------------------

/// Scheme Type Box (`schm`) — §13.4.6.
///
/// Identifies the protection scheme (e.g. `cenc`), version, and URI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SchemeTypeBox {
    pub version: u8,
    pub flags: u32,
    pub scheme_type: [u8; crate::box_types::BOX_TYPE_SIZE],
    pub scheme_version: u32,
    pub scheme_uri: Option<Vec<u8>>,
}

impl SchemeTypeBox {
    /// The registered restricted scheme type, or `Other` for a protection
    /// scheme ([`CencScheme`]) or a scheme from a document this crate does not
    /// hold.
    #[must_use]
    pub fn restricted_scheme(&self) -> crate::registered::RestrictedSchemeType {
        crate::registered::RestrictedSchemeType::from_fourcc(self.scheme_type)
    }

    /// Parse a `schm` box from FullBox body bytes.
    pub fn parse_body(bytes: &[u8], _version: u8, flags: u32) -> Result<Self> {
        need_in(bytes, AUX_INFO_LEN, "schm body")?;
        let mut scheme_type = [0u8; crate::box_types::BOX_TYPE_SIZE];
        scheme_type.copy_from_slice(&bytes[0..crate::box_types::BOX_TYPE_SIZE]);
        let scheme_version = u32_at(bytes, crate::box_types::BOX_TYPE_SIZE);
        let scheme_uri = if (flags & SCHM_FLAG_HAS_SCHEME_URI) != 0 {
            Some(bytes[AUX_INFO_LEN..].to_vec())
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
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + AUX_INFO_LEN;
        if let Some(ref uri) = self.scheme_uri {
            n += uri.len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"schm",
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + crate::box_types::BOX_TYPE_SIZE].copy_from_slice(&self.scheme_type);
        c += crate::box_types::BOX_TYPE_SIZE;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.scheme_version.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if let Some(ref uri) = self.scheme_uri {
            buf[c..c + uri.len()].copy_from_slice(uri);
            c += uri.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// schi — Scheme Information Box (ISO/IEC 14496-12:2026 §13.4.7)
// ---------------------------------------------------------------------------

/// Scheme Information Box (`schi`) — §13.4.7.
///
/// Container box for scheme-specific data (e.g. `tenc` inside `schi`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SchemeInformationBox {
    pub tenc: Option<TrackEncryptionBox>,
    /// The stereoscopic-video scheme box (§13.5.4.2), when `scheme_type` is
    /// `stvi`.
    pub stvi: Option<crate::track_desc::StereoVideoBox>,
    /// The sample-packing information box (§13.5.6.2), when `scheme_type` is
    /// `spkt`.
    pub spki: Option<crate::scheme::SamplePackingInformationBox>,
    pub extra_boxes: Vec<crate::box_support::OpaqueBox>,
}

impl<'a> Parse<'a> for SchemeInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "schi")?;
        let body = crate::box_types::parse_box(bytes)?.0.body;
        let mut tenc = None;
        let mut stvi = None;
        let mut spki = None;
        let mut extra_boxes = Vec::new();
        let mut off = 0usize;
        while off + crate::box_types::BOX_HEADER_MIN_SIZE <= body.len() {
            let sz = u32_at(body, off) as usize;
            if sz < crate::box_types::BOX_HEADER_MIN_SIZE {
                break;
            }
            let end = off.saturating_add(sz).min(body.len());
            let boxtype = u32_at(body, off + crate::box_types::BOX_TYPE_SIZE).to_be_bytes();
            if &boxtype == b"tenc" {
                tenc = Some(TrackEncryptionBox::parse_box(&body[off..end])?);
            } else if &boxtype == b"stvi" {
                stvi = Some(crate::track_desc::StereoVideoBox::parse(&body[off..end])?);
            } else if &boxtype == b"spki" {
                spki = Some(crate::scheme::SamplePackingInformationBox::parse(
                    &body[off..end],
                )?);
            } else {
                extra_boxes.push(crate::box_support::OpaqueBox {
                    box_type: boxtype,
                    data: body[off + crate::box_types::BOX_HEADER_MIN_SIZE..end].to_vec(),
                    to_end: false,
                    largesize: false,
                });
            }
            off = off.saturating_add(sz);
        }
        Ok(Self {
            tenc,
            stvi,
            spki,
            extra_boxes,
        })
    }
}

impl Serialize for SchemeInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        if let Some(ref t) = self.tenc {
            n += t.serialized_len();
        }
        if let Some(ref t) = self.stvi {
            n += t.serialized_len();
        }
        if let Some(ref t) = self.spki {
            n += t.serialized_len();
        }
        for b in &self.extra_boxes {
            n += b.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, b"schi", need)?;
        if let Some(ref t) = self.tenc {
            c += t.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref t) = self.stvi {
            c += t.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref t) = self.spki {
            c += t.serialize_into(&mut buf[c..])?;
        }
        for b in &self.extra_boxes {
            c += b.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// sinf — Protection Scheme Information Box (ISO/IEC 14496-12:2026 §13.4.2)
// ---------------------------------------------------------------------------

/// Four-CC of the Protection Scheme Information Box (`sinf`, ISO/IEC 14496-12:2026 §13.4.2).
pub const SINF: [u8; 4] = *b"sinf";

/// Bytes of a box's 32-bit `size` field (ISO/IEC 14496-12:2026 §4.2).
const BOX_SIZE_FIELD_LEN: usize = 4;

/// A protected `encv` entry's child boxes start after the `VisualSampleEntry`
/// fixed part: 8 bytes of box header plus the 78 bytes §12.1.3 defines.
const ENCV_CHILD_OFFSET: usize = crate::box_types::BOX_HEADER_MIN_SIZE + 78;

/// A protected `enca` entry's child boxes start after the `AudioSampleEntry`
/// fixed part: 8 bytes of box header plus the 28 bytes §12.2.3 defines.
const ENCA_CHILD_OFFSET: usize = crate::box_types::BOX_HEADER_MIN_SIZE + 28;

/// A protected sample entry's `sinf` child, from the entry's own box bytes.
///
/// A CENC-protected track renames its sample entry's four-CC to `encv`
/// (visual) or `enca` (audio) and keeps the codec's configuration as children
/// followed by `sinf` (§13.4.1). The fixed part's length is therefore still the
/// wrapped entry's, and this is the one place that knows where its children
/// start.
#[must_use]
pub fn sinf_of_sample_entry(entry: &[u8]) -> Option<&[u8]> {
    let start =
        match entry.get(crate::box_types::BOX_TYPE_SIZE..2 * crate::box_types::BOX_TYPE_SIZE)? {
            b"encv" => ENCV_CHILD_OFFSET,
            b"enca" => ENCA_CHILD_OFFSET,
            _ => return None,
        };
    let region = entry.get(start..)?;
    crate::box_types::BoxIter::new(region)
        .filter_map(|item| item.ok())
        .find(|item| item.header.box_type.is(&SINF))
        .map(|item| item.bytes)
}

/// Reconstruct the cleartext sample entry a protected `encv`/`enca` entry
/// wraps (ISO/IEC 14496-12:2026 §13.4.1): rename the four-CC back to the
/// `frma` original format and drop the `sinf` child.
///
/// Returns `None` for an entry that is not `encv`/`enca`, carries no `sinf`,
/// or whose `sinf` has no valid `frma`/box layout.
#[must_use]
pub fn clear_sample_entry(entry: &[u8]) -> Option<Vec<u8>> {
    let child_start =
        match entry.get(crate::box_types::BOX_TYPE_SIZE..2 * crate::box_types::BOX_TYPE_SIZE)? {
            b"encv" => ENCV_CHILD_OFFSET,
            b"enca" => ENCA_CHILD_OFFSET,
            _ => return None,
        };
    let sinf = sinf_of_sample_entry(entry)?;
    let parsed = ProtectionSchemeInfoBox::parse(sinf).ok()?;
    let original = parsed.original_format.data_format;
    let region = entry.get(child_start..)?;
    let item = crate::box_types::BoxIter::new(region)
        .filter_map(|item| item.ok())
        .find(|item| item.four_cc() == SINF)?;
    let start = child_start.checked_add(item.start)?;
    let end = child_start.checked_add(item.end)?;
    if start < crate::box_types::BOX_HEADER_MIN_SIZE || end > entry.len() || end < start {
        return None;
    }
    let mut out = Vec::with_capacity(entry.len().saturating_sub(end - start));
    out.extend_from_slice(&entry[..start]);
    out.extend_from_slice(&entry[end..]);
    out.get_mut(crate::box_types::BOX_TYPE_OFFSET..2 * crate::box_types::BOX_TYPE_SIZE)?
        .copy_from_slice(&original);
    let size = u32::try_from(out.len()).ok()?;
    out.get_mut(..BOX_SIZE_FIELD_LEN)?
        .copy_from_slice(&size.to_be_bytes());
    Some(out)
}

/// Protection Scheme Information Box (`sinf`) — §13.4.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProtectionSchemeInfoBox {
    pub original_format: OriginalFormatBox,
    pub scheme_type: Option<SchemeTypeBox>,
    pub scheme_info: Option<SchemeInformationBox>,
    pub extra_boxes: Vec<crate::box_support::OpaqueBox>,
}

impl<'a> Parse<'a> for ProtectionSchemeInfoBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "sinf")?;
        let body = crate::box_types::parse_box(bytes)?.0.body;
        if body.len() < crate::box_types::BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: crate::box_types::BOX_HEADER_MIN_SIZE + crate::box_types::BOX_TYPE_SIZE,
                have: bytes.len(),
                what: "sinf (frma)",
            }));
        }
        let frma_sz = u32_at(body, 0) as usize;
        // The declared frma size is wire-controlled; bound it before slicing
        // (r05-C2).
        let frma_bytes = body
            .get(..frma_sz)
            .ok_or(Error::BufferTooShort(BufferTooShort {
                need: frma_sz,
                have: body.len(),
                what: "sinf (frma)",
            }))?;
        let original_format = OriginalFormatBox::parse(frma_bytes)?;

        let mut off = frma_sz;
        let mut scheme_type = None;
        let mut scheme_info = None;
        let mut extra_boxes = Vec::new();

        while off + crate::box_types::BOX_HEADER_MIN_SIZE <= body.len() {
            let sz = u32_at(body, off) as usize;
            if sz < crate::box_types::BOX_HEADER_MIN_SIZE {
                break;
            }
            // A declared size past the buffer ends parsing here rather than
            // wrapping on a narrow target (r05-C2).
            let Some(box_end) = off.checked_add(sz) else {
                break;
            };
            let end = box_end.min(body.len());
            let boxtype = u32_at(body, off + crate::box_types::BOX_TYPE_SIZE).to_be_bytes();
            match &boxtype {
                b"schm" => {
                    // A schm needs its FullBox version/flags present before
                    // they are read (r05-C2).
                    let full_end = off + BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
                    if sz < BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE || end < full_end {
                        return Err(Error::BufferTooShort(BufferTooShort {
                            need: full_end,
                            have: body.len(),
                            what: "sinf (schm)",
                        }));
                    }
                    let full = &body[off + BOX_HEADER_MIN_SIZE..full_end];
                    scheme_type = Some(SchemeTypeBox::parse_body(
                        &body[full_end..end],
                        full[0],
                        fullbox_flags(full),
                    )?);
                }
                b"schi" => {
                    scheme_info = Some(SchemeInformationBox::parse(&body[off..end])?);
                }
                _ => {
                    extra_boxes.push(crate::box_support::OpaqueBox {
                        box_type: boxtype,
                        data: body[off + crate::box_types::BOX_HEADER_MIN_SIZE..end].to_vec(),
                        to_end: false,
                        largesize: false,
                    });
                }
            }
            off = box_end;
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
        let mut n = BOX_HEADER_MIN_SIZE;
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
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, b"sinf", need)?;
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
    use broadcast_core::error::BufferTooShort;

    use super::*;

    /// `saiz` versions 0, 1 and 2 (ISO/IEC 14496-12:2026 §8.7.8.2): 8-, 16-
    /// and 32-bit `default_sample_info_size` / `sample_info_size[]`.
    fn saiz_variable_box(version: u8, flags_bytes: &[u8], sizes: &[u32]) -> Vec<u8> {
        let width = match version {
            0 => 1,
            1 => 2,
            _ => 4,
        };
        let mut body = alloc::vec![version, 0, 0, flags_bytes.len().min(1) as u8];
        body.extend_from_slice(flags_bytes);
        body.extend(core::iter::repeat_n(0u8, width));
        body.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        for v in sizes {
            body.extend_from_slice(&v.to_be_bytes()[4 - width..]);
        }
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"saiz");
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn saiz_versions_read_their_own_width_and_round_trip() {
        for (version, sizes) in [
            (0u8, alloc::vec![0x28u32, 0x10]),
            (1, alloc::vec![0x0128, 0x0200]),
            (2, alloc::vec![0x0001_0000, 0x0200_0001]),
        ] {
            let bytes = saiz_variable_box(version, &[], &sizes);
            let saiz = SampleAuxInfoSizesBox::parse_box(&bytes).unwrap();
            assert_eq!(saiz.version, version);
            assert_eq!(saiz.sample_count, 2, "v{version} sample_count");
            assert_eq!(saiz.default_sample_info_size, 0);
            assert_eq!(saiz.sample_info_sizes, sizes, "v{version} sizes");
            assert_eq!(saiz.try_to_bytes().unwrap(), bytes, "v{version} round trip");
        }
    }

    #[test]
    fn saiz_wide_default_size_and_aux_info_type_round_trip() {
        for (version, default) in [(1u8, 0x0123u32), (2, 0x0001_0203)] {
            let width = if version == 1 { 2 } else { 4 };
            let mut body = alloc::vec![version, 0, 0, 1];
            body.extend_from_slice(b"cenc");
            body.extend_from_slice(&7u32.to_be_bytes());
            body.extend_from_slice(&default.to_be_bytes()[4 - width..]);
            body.extend_from_slice(&9u32.to_be_bytes());
            let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(b"saiz");
            bytes.extend_from_slice(&body);
            let saiz = SampleAuxInfoSizesBox::parse_box(&bytes).unwrap();
            assert_eq!(saiz.default_sample_info_size, default);
            assert_eq!(saiz.sample_count, 9);
            assert_eq!(saiz.aux_info_type_parameter, Some(7));
            assert!(saiz.sample_info_sizes.is_empty());
            assert_eq!(saiz.try_to_bytes().unwrap(), bytes, "v{version} round trip");
        }
    }

    #[test]
    fn saiz_size_wider_than_its_version_errors_and_unknown_version_is_rejected() {
        let mut saiz =
            SampleAuxInfoSizesBox::parse_box(&saiz_variable_box(1, &[], &[1, 2])).unwrap();
        saiz.sample_info_sizes[1] = 0x1_0000;
        assert!(saiz.try_to_bytes().is_err(), "0x10000 does not fit 16 bits");
        let bad = saiz_variable_box(3, &[], &[1]);
        assert!(SampleAuxInfoSizesBox::parse_box(&bad).is_err());
    }

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

    /// Audit finding #1: a 19-byte `tenc` body (one byte short of the true
    /// minimum — reserved(1) + reserved/crypt-skip(1) + is_protected(1) +
    /// iv_size(1) + default_KID(16) = 20, ISO/IEC 23001-7:2016 §12.2.2) used to
    /// pass the length guard (which undercounted by one byte) and then panic
    /// slicing `bytes[4..20]` for the KID. Must be rejected with `Err`, not
    /// panic.
    #[test]
    fn tenc_19_byte_body_is_rejected_not_panicking() {
        let body = [0u8; 19];
        let err = TrackEncryptionBox::parse_body(&body, 0)
            .expect_err("a 19-byte tenc body is one byte short of the true minimum");
        assert!(
            matches!(
                err,
                Error::BufferTooShort(BufferTooShort {
                    need: 20,
                    have: 19,
                    ..
                })
            ),
            "expected BufferTooShort {{ need: 20, have: 19, .. }}, got {err:?}"
        );
    }

    /// The corrected minimum (20 bytes) still accepts a well-formed body —
    /// the fix must not have over-tightened the guard.
    #[test]
    fn tenc_20_byte_body_parses() {
        let body = [0u8; 20];
        let parsed = TrackEncryptionBox::parse_body(&body, 0).expect("20-byte tenc body parses");
        assert_eq!(parsed.default_kid, [0u8; 16]);
    }

    /// A `tenc` v1 with the cbcs constant IV (ISO/IEC 23001-7 §12.2.2:
    /// `default_is_protected == 1`, `default_per_sample_iv_size == 0`, then a
    /// `default_constant_IV_size` byte and the IV).
    fn tenc_v1_with_constant_iv() -> TrackEncryptionBox {
        TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: 1,
            default_skip_byte_block: 9,
            default_is_protected: 1,
            default_per_sample_iv_size: 0,
            default_kid: [
                0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
                0x1F, 0x20,
            ],
            default_constant_iv: Some(alloc::vec![0x11; 16]),
        }
    }

    fn buffer_too_short(err: Error) -> BufferTooShort {
        match err {
            Error::BufferTooShort(b) => b,
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// Every proper prefix of a valid box must be rejected with
    /// `BufferTooShort`, never an index panic: each guard sits in front of a
    /// direct `bytes[offset]` read, so only truncated input exercises it.
    fn assert_every_prefix_is_buffer_too_short(full: &[u8], parse: impl Fn(&[u8]) -> Result<()>) {
        for len in 0..full.len() {
            match parse(&full[..len]) {
                Err(Error::BufferTooShort(_)) => {}
                Err(other) => {
                    panic!("{len}-byte prefix: expected BufferTooShort, got {other:?}")
                }
                Ok(()) => panic!("{len}-byte prefix of a {}-byte box parsed", full.len()),
            }
        }
        parse(full).expect("the whole box parses");
    }

    #[test]
    fn tenc_truncations_are_buffer_too_short_with_the_exact_shortfall() {
        let tenc = tenc_v1_with_constant_iv();
        assert_eq!(tenc.serialized_len(), 12 + 20 + 1 + 16);
        let mut buf = alloc::vec![0u8; tenc.serialized_len()];
        assert_eq!(
            tenc.serialize_into(&mut buf).unwrap(),
            tenc.serialized_len()
        );
        assert_every_prefix_is_buffer_too_short(&buf, |b| {
            TrackEncryptionBox::parse_box(b).map(|_| ())
        });

        let b = buffer_too_short(TrackEncryptionBox::parse_box(&buf[..8]).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (12, 8, "tenc header"));
        let b = buffer_too_short(TrackEncryptionBox::parse_box(&buf[..32]).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (21, 20, "tenc constant_IV_size"));
        let b = buffer_too_short(TrackEncryptionBox::parse_box(&buf[..42]).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (37, 30, "tenc constant_IV"));
    }

    /// A `senc` with two 8-byte IVs and one subsample map each
    /// (ISO/IEC 23001-7 §12.3), with every field byte distinct.
    fn senc_two_subsample_entries() -> SampleEncryptionBox {
        SampleEncryptionBox {
            version: 0,
            flags: SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
            per_sample_iv_size: 8,
            entries: alloc::vec![
                SampleEncryptionEntry {
                    initialization_vector: alloc::vec![
                        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08
                    ],
                    subsamples: alloc::vec![SubSampleEntry {
                        bytes_of_clear_data: 0x0102,
                        bytes_of_protected_data: 0x0304_0506,
                    }],
                },
                SampleEncryptionEntry {
                    initialization_vector: alloc::vec![
                        0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18
                    ],
                    subsamples: alloc::vec![SubSampleEntry {
                        bytes_of_clear_data: 0x2132,
                        bytes_of_protected_data: 0x3344_5566,
                    }],
                },
            ],
        }
    }

    #[test]
    fn senc_truncations_are_buffer_too_short_with_the_exact_shortfall() {
        let senc = senc_two_subsample_entries();
        let mut buf = alloc::vec![0u8; senc.serialized_len()];
        let n = senc.serialize_into(&mut buf).unwrap();
        let body = &buf[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..n];
        assert_eq!(body.len(), 4 + 2 * (8 + 2 + 6));
        assert_eq!(
            senc.serialized_len(),
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + body.len()
        );

        for len in 0..body.len() {
            match SampleEncryptionBox::parse_body(
                &body[..len],
                senc.version,
                senc.flags,
                senc.per_sample_iv_size,
            ) {
                Err(Error::BufferTooShort(_)) => {}
                Err(other) => {
                    panic!("{len}-byte senc body: expected BufferTooShort, got {other:?}")
                }
                Ok(_) => panic!("{len}-byte senc body of {} parsed", body.len()),
            }
        }
        let whole = SampleEncryptionBox::parse_body(
            body,
            senc.version,
            senc.flags,
            senc.per_sample_iv_size,
        )
        .unwrap();
        assert_eq!(whole.entries[0].subsamples[0].bytes_of_clear_data, 0x0102);
        assert_eq!(
            whole.entries[0].subsamples[0].bytes_of_protected_data,
            0x0304_0506
        );
        assert_eq!(
            whole.entries[1].subsamples[0].bytes_of_protected_data,
            0x3344_5566
        );

        let b = buffer_too_short(
            SampleEncryptionBox::parse_body(&body[..29], 0, senc.flags, 8).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (30, 29, "senc subsample_count"));
        let b = buffer_too_short(
            SampleEncryptionBox::parse_body(&body[..35], 0, senc.flags, 8).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (36, 35, "senc subsample"));
        let b = buffer_too_short(
            SampleEncryptionBox::parse_body(&body[..8], 0, senc.flags, 8).unwrap_err(),
        );
        assert_eq!(
            (b.need, b.have, b.what),
            (
                24,
                8,
                "senc entries (sample_count exceeds what the box can hold)"
            )
        );
    }

    #[test]
    fn pssh_truncations_are_buffer_too_short() {
        let pssh = ProtectionSystemSpecificHeaderBox {
            version: 1,
            system_id: [0x22; 16],
            kids: alloc::vec![[0x33; 16], [0x44; 16]],
            data: alloc::vec![0xA5; 300],
        };
        let bytes = pssh.try_to_bytes().unwrap();
        assert_every_prefix_is_buffer_too_short(&bytes, |b| {
            ProtectionSystemSpecificHeaderBox::parse_box(b).map(|_| ())
        });
        let b = buffer_too_short(
            ProtectionSystemSpecificHeaderBox::parse_box(&bytes[..11]).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (12, 11, "pssh header"));

        // A v1 body that ends right after a zero KID_count: the KID table is
        // empty, so the next missing field is the DataSize.
        let mut empty_kid_table = alloc::vec![0x22; 16];
        empty_kid_table.extend_from_slice(&0u32.to_be_bytes());
        let b = buffer_too_short(
            ProtectionSystemSpecificHeaderBox::parse_body(&empty_kid_table, 1).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (24, 20, "pssh DataSize"));

        // Cut right after the KID table of a two-KID box: the body is then
        // 16 SystemID + 4 KID_count + 32 KIDs = 52 bytes, and DataSize needs
        // four more.
        let b = buffer_too_short(
            ProtectionSystemSpecificHeaderBox::parse_box(&bytes[..64]).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (56, 52, "pssh DataSize"));

        // DataSize 0x0001_0002 — a value whose second byte differs from its
        // first, so a read from the wrong byte of the field changes the answer.
        let mut wide_data_size = alloc::vec![0x22; 16];
        wide_data_size.extend_from_slice(&0u32.to_be_bytes());
        wide_data_size.extend_from_slice(&0x0001_0002u32.to_be_bytes());
        let b = buffer_too_short(
            ProtectionSystemSpecificHeaderBox::parse_body(&wide_data_size, 1).unwrap_err(),
        );
        assert_eq!((b.need, b.have, b.what), (65_562, 24, "pssh Data"));
    }

    /// A hostile v1 KID count: the KID table's footprint (count x 16) is what
    /// the box must hold, and the shortfall names it exactly (r05-C3).
    #[test]
    fn pssh_v1_hostile_kid_count_reports_the_exact_need() {
        let mut body = alloc::vec![0x22; 16];
        body.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        let b =
            buffer_too_short(ProtectionSystemSpecificHeaderBox::parse_body(&body, 1).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (34_359_738_372, 20, "pssh KIDs"));
    }

    #[test]
    fn saiz_truncations_are_buffer_too_short_with_the_exact_shortfall() {
        let mut body = alloc::vec![0u8];
        body.extend_from_slice(&4u32.to_be_bytes());
        body.extend_from_slice(&[8, 16, 24]);
        assert_eq!(body.len(), 8);

        let b = buffer_too_short(SampleAuxInfoSizesBox::parse_body(&body, 0, 0).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (9, 8, "saiz sample_info_size"));

        let b = buffer_too_short(SampleAuxInfoSizesBox::parse_box(&[0u8; 8]).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (12, 8, "saiz header"));
    }

    /// A hostile `saiz` `sample_count`: the per-sample table's footprint is
    /// what the box must hold, and its shortfall is reported exactly. The
    /// count's high bytes are non-zero here, so a read from the wrong byte of
    /// the field cannot produce the same answer.
    #[test]
    fn saiz_hostile_sample_count_reports_the_exact_need() {
        let mut body = alloc::vec![0u8];
        body.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        let b = buffer_too_short(SampleAuxInfoSizesBox::parse_body(&body, 0, 0).unwrap_err());
        assert_eq!(
            (b.need, b.have, b.what),
            (2_147_483_652, 5, "saiz sample_info_size")
        );
    }

    /// `serialize_into` must return the number of bytes it wrote, constant IV
    /// included — every cbcs stream's `tenc` carries one.
    #[test]
    fn tenc_constant_iv_serialize_returns_the_written_length() {
        let tenc = tenc_v1_with_constant_iv();
        let mut buf = alloc::vec![0u8; tenc.serialized_len()];
        let n = tenc.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 49);
        assert_eq!(n, tenc.serialized_len());
        assert_eq!(n, buf.len());
        assert_eq!(TrackEncryptionBox::parse_box(&buf[..n]).unwrap(), tenc);
    }

    /// A per-sample-size `saiz` table of four entries (§8.7.8.2), in each
    /// version's width. With zero or one entry the length guard's arithmetic
    /// is indistinguishable from a wrong bound.
    #[test]
    fn saiz_per_sample_table_of_four_entries_round_trips_in_every_width() {
        for (version, sizes) in [
            (0u8, alloc::vec![8u32, 16, 24, 16]),
            (1, alloc::vec![8, 16, 24, 16]),
            (2, alloc::vec![8, 16, 24, 16]),
        ] {
            let bytes = saiz_variable_box(version, &[], &sizes);
            let saiz = SampleAuxInfoSizesBox::parse_box(&bytes).unwrap();
            assert_eq!(saiz.version, version);
            assert_eq!(saiz.default_sample_info_size, 0, "v{version}");
            assert_eq!(saiz.sample_count, 4, "v{version}");
            assert_eq!(saiz.sample_info_sizes, sizes, "v{version}");
            assert_eq!(saiz.try_to_bytes().unwrap(), bytes, "v{version} bytes");
        }
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
        let parsed = ProtectionSystemSpecificHeaderBox::parse_body(
            &buf[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..],
            0,
        )
        .unwrap();
        assert_eq!(parsed.system_id, system_id);
        assert_eq!(parsed.data, pssh.data);
        assert!(parsed.kids.is_empty());
    }

    /// A `pssh` v1 carrying three KIDs and a 300-byte Data blob — the
    /// Widevine/PlayReady form (ISO/IEC 23001-7 §12.1, §8.1.2). Every length
    /// and KID offset on both sides is pinned, and DataSize is above 255 so a
    /// single mis-read byte cannot pass for it.
    #[test]
    fn pssh_v1_three_kids_and_long_data_round_trip() {
        let system_id = [
            0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE, 0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D,
            0x21, 0xED,
        ];
        let kids: [[u8; 16]; 3] = [
            [
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
                0x1E, 0x1F,
            ],
            [
                0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
                0x2E, 0x2F,
            ],
            [
                0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
                0x3E, 0x3F,
            ],
        ];
        let pssh = ProtectionSystemSpecificHeaderBox {
            version: 1,
            system_id,
            kids: kids.to_vec(),
            data: (0..300u32).map(|i| (i % 251) as u8).collect(),
        };

        assert_eq!(pssh.serialized_len(), 8 + 4 + 16 + 4 + 3 * 16 + 4 + 300);
        let mut buf = vec![0u8; pssh.serialized_len()];
        let n = pssh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 384, "serialize_into must return every byte it wrote");
        assert_eq!(&buf[28..32], &3u32.to_be_bytes(), "KID_count");
        for (i, kid) in kids.iter().enumerate() {
            assert_eq!(&buf[32 + 16 * i..48 + 16 * i], kid, "KID {i}");
        }
        assert_eq!(&buf[80..84], &300u32.to_be_bytes(), "DataSize");
        assert_eq!(&buf[84..], pssh.data, "Data");

        let parsed = ProtectionSystemSpecificHeaderBox::parse_box(&buf).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.system_id, system_id);
        assert_eq!(parsed.kids, kids.to_vec());
        assert_eq!(parsed.data.len(), 300);
        assert_eq!(parsed, pssh);
        assert_eq!(parsed.try_to_bytes().unwrap(), buf);
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

    /// **The alloc-DoS regression test (behaviour half).** A tiny `senc`
    /// declaring `sample_count == 0xFFFFFFFF` must be rejected from the box's
    /// own length. The old code sized `Vec::with_capacity(sample_count)` from
    /// this untrusted field first — ~4.3 billion entries, hundreds of GB — on a
    /// 20-byte input read straight off the network.
    ///
    /// This test pins the *error*; `tests/cenc_senc_alloc_bound.rs` pins the
    /// *allocation* (the part that actually matters, and the part this test
    /// cannot see: a hostile `with_capacity` still ends in `BufferTooShort`
    /// from the per-entry checks, just after asking the allocator for 206 GB).
    #[test]
    fn senc_hostile_sample_count_is_rejected() {
        // body = sample_count(4) + 16 bytes of would-be entry data = 20 bytes.
        let mut body = alloc::vec![0xFFu8; 4];
        body.extend_from_slice(&[0u8; 16]);
        assert_eq!(body.len(), 20);

        for (iv_size, flags) in [
            (8u8, 0u32),
            (8, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
            (16, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
            (0, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
        ] {
            let err = SampleEncryptionBox::parse_body(&body, 0, flags, iv_size)
                .expect_err("a hostile sample_count must be rejected");
            assert!(
                matches!(err, Error::BufferTooShort(..)),
                "iv_size={iv_size} flags={flags:#x}: expected BufferTooShort, got {err:?}"
            );
        }

        // The degenerate shape that carries no per-sample bytes at all: the
        // count is unverifiable against any body length, so it is rejected
        // rather than trusted.
        let err = SampleEncryptionBox::parse_body(&body, 0, 0, 0)
            .expect_err("an unverifiable sample_count must be rejected");
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
    }

    /// The bound must not reject *well-formed* boxes: a body exactly sized for
    /// its declared entries still parses.
    #[test]
    fn senc_exact_length_body_parses() {
        const SAMPLES: usize = 3;
        let mut body = (SAMPLES as u32).to_be_bytes().to_vec();
        for i in 0..SAMPLES {
            body.extend_from_slice(&[i as u8; 8]); // 8-byte IV
        }
        let parsed = SampleEncryptionBox::parse_body(&body, 0, 0, 8).expect("well-formed senc");
        assert_eq!(parsed.entries.len(), SAMPLES);
        assert_eq!(parsed.entries[2].initialization_vector, alloc::vec![2u8; 8]);
    }

    /// A `saio` `entry_count` that would overflow `entry_count * 8` on a 32-bit
    /// `usize` must be rejected rather than wrap past the length check.
    #[test]
    fn saio_hostile_entry_count_is_rejected() {
        // version 1 → 8-byte offsets; entry_count = 0xFFFFFFFF.
        let body = alloc::vec![0xFFu8; 4];
        let err = SampleAuxInfoOffsetsBox::parse_body(&body, 1, 0)
            .expect_err("a hostile entry_count must be rejected");
        assert!(
            matches!(err, Error::BufferTooShort(..) | Error::InvalidInput(_)),
            "{err:?}"
        );
    }

    // r05-C2: a `frma` child claiming size 0x1000 inside a 40-byte `sinf` used
    // to slice `&body[0..0x1000]`; must be Err, not panic.
    #[test]
    fn rejects_hostile_frma_size() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&40u32.to_be_bytes());
        bytes.extend_from_slice(b"sinf");
        bytes.extend_from_slice(&0x1000u32.to_be_bytes());
        bytes.extend_from_slice(b"frma");
        while bytes.len() < 40 {
            bytes.push(0);
        }
        let err = ProtectionSchemeInfoBox::parse(&bytes)
            .expect_err("a frma size beyond the sinf body must be rejected");
        assert!(
            matches!(err, Error::BufferTooShort(..)),
            "expected BufferTooShort, got {err:?}"
        );
    }

    // r05-C2: a header-only (8-byte) `schm` at the end of `sinf` has no
    // version/flags bytes; must be Err, not panic.
    #[test]
    fn rejects_header_only_schm() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&28u32.to_be_bytes());
        bytes.extend_from_slice(b"sinf");
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(b"frma");
        bytes.extend_from_slice(b"encv");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"schm");
        let err = ProtectionSchemeInfoBox::parse(&bytes)
            .expect_err("a schm without FullBox header must be rejected");
        assert!(
            matches!(err, Error::BufferTooShort(..)),
            "expected BufferTooShort, got {err:?}"
        );
    }

    // r05-C3: a 20-byte v1 `pssh` body (SystemID + KID_count = 0) carries no
    // DataSize field; must be Err, not panic.
    #[test]
    fn rejects_pssh_v1_missing_data_size() {
        let mut body = vec![0u8; 16]; // SystemID
        body.extend_from_slice(&0u32.to_be_bytes()); // KID_count = 0
        let err = ProtectionSystemSpecificHeaderBox::parse_body(&body, 1)
            .expect_err("a v1 pssh without DataSize must be rejected");
        assert!(
            matches!(err, Error::BufferTooShort(..)),
            "expected BufferTooShort, got {err:?}"
        );
    }

    // r05-C3: KID_count = 0xFFFFFFFF must be Err from the length check before
    // any allocation (kid_count * 16 is checked arithmetic).
    #[test]
    fn rejects_pssh_v1_hostile_kid_count() {
        let mut body = vec![0u8; 16]; // SystemID
        body.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        body.extend_from_slice(b"xxxx");
        let err = ProtectionSystemSpecificHeaderBox::parse_body(&body, 1)
            .expect_err("a hostile KID_count must be rejected");
        assert!(
            matches!(err, Error::BufferTooShort(..) | Error::InvalidInput(_)),
            "expected BufferTooShort/InvalidInput, got {err:?}"
        );
    }

    /// Audit r05-W9: `senc` flag `0x000001` (override TrackEncryptionBox
    /// parameters, PIFF 1.1 / ISO/IEC 23001-7:2012 v0) puts a 20-byte
    /// `AlgorithmID(24) IV_size(8) KID(128)` triple before `sample_count`.
    /// Reading such a box as if the flag were clear takes those bytes as the
    /// count and the first entries. It must be rejected with a clear error.
    #[test]
    fn senc_override_flag_is_rejected() {
        // A v0 PIFF `senc`: flags 0x1, a 20-byte override triple, then
        // sample_count = 1 and one 8-byte IV.
        let mut body = alloc::vec![0u8; SENC_OVERRIDE_FIELDS_LEN];
        // AlgorithmID 0x000001 (AES-CTR), IV_size 8, then a 16-byte KID.
        body[2] = 0x01;
        body[3] = 0x08;
        for (i, b) in body[4..20].iter_mut().enumerate() {
            *b = i as u8;
        }
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&[0xAAu8; 8]);

        let err = SampleEncryptionBox::parse_body(&body, 0, SENC_FLAG_OVERRIDE_TRACK_ENCRYPTION, 8)
            .expect_err("senc override flag must be rejected");
        assert!(
            matches!(err, Error::UnsupportedFeature(_)),
            "expected UnsupportedFeature, got {err:?}"
        );
    }

    /// Control: the *same* body with the override flag clear is a valid
    /// `senc` — the rejection is keyed on the flag, not the bytes.
    #[test]
    fn senc_without_override_flag_parses() {
        let mut body = 1u32.to_be_bytes().to_vec();
        body.extend_from_slice(&[0xAAu8; 8]);
        let senc = SampleEncryptionBox::parse_body(&body, 0, 0, 8).expect("parse");
        assert_eq!(senc.entries.len(), 1);
        assert_eq!(
            senc.entries[0].initialization_vector,
            alloc::vec![0xAAu8; 8]
        );
    }

    /// A truncated PIFF `senc` (flag set, fewer than 20 override bytes before
    /// the end) must still be a clean `Err`, never a panic — the flag is
    /// rejected before any length arithmetic, so this holds by construction.
    #[test]
    fn senc_override_flag_truncated_body_is_err() {
        for len in 0..SENC_OVERRIDE_FIELDS_LEN {
            let body = alloc::vec![0u8; len];
            let err =
                SampleEncryptionBox::parse_body(&body, 0, SENC_FLAG_OVERRIDE_TRACK_ENCRYPTION, 8)
                    .expect_err("truncated override senc must be rejected");
            assert!(matches!(err, Error::UnsupportedFeature(_)), "got {err:?}");
        }
    }

    /// A `senc` entry with more than 65 535 subsamples cannot fit the 16-bit
    /// `subsample_count` field (#1129): unfixed, `.len() as u16` wraps 65536
    /// down to 0 while all 65536 subsample pairs are still written.
    #[test]
    fn senc_oversized_subsample_count_errors() {
        let senc = SampleEncryptionBox {
            version: 0,
            flags: SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
            per_sample_iv_size: 8,
            entries: alloc::vec![SampleEncryptionEntry {
                initialization_vector: alloc::vec![0u8; 8],
                subsamples: (0..65536)
                    .map(|_| SubSampleEntry {
                        bytes_of_clear_data: 1,
                        bytes_of_protected_data: 1,
                    })
                    .collect(),
            }],
        };
        let err = senc.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "subsample_count",
                    ..
                })
            ),
            "expected FieldOverflow for subsample_count, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 subsamples still round-trips.
    #[test]
    fn senc_max_subsample_count_round_trips() {
        let senc = SampleEncryptionBox {
            version: 0,
            flags: SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
            per_sample_iv_size: 8,
            entries: alloc::vec![SampleEncryptionEntry {
                initialization_vector: alloc::vec![0u8; 8],
                subsamples: (0..65535)
                    .map(|_| SubSampleEntry {
                        bytes_of_clear_data: 1,
                        bytes_of_protected_data: 1,
                    })
                    .collect(),
            }],
        };
        let bytes = senc.try_to_bytes().unwrap();
        let parsed = SampleEncryptionBox::parse_body(
            &bytes[BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE..],
            senc.version,
            senc.flags,
            senc.per_sample_iv_size,
        )
        .unwrap();
        assert_eq!(parsed.entries[0].subsamples.len(), 65535);
    }

    /// A `saio` version-0 offset past `u32::MAX` cannot fit the 32-bit
    /// wire field (#1129): unfixed, `(off as u32)` silently truncated it.
    #[test]
    fn saio_v0_oversized_offset_errors() {
        let saio = SampleAuxInfoOffsetsBox {
            version: 0,
            flags: 0,
            aux_info_type: None,
            aux_info_type_parameter: None,
            offsets: alloc::vec![u64::from(u32::MAX) + 1],
        };
        let err = saio.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "offset",
                    ..
                })
            ),
            "expected FieldOverflow for offset, got {err:?}"
        );
    }

    /// The boundary: exactly `u32::MAX` still round-trips in version 0.
    #[test]
    fn saio_v0_max_offset_round_trips() {
        let saio = SampleAuxInfoOffsetsBox {
            version: 0,
            flags: 0,
            aux_info_type: None,
            aux_info_type_parameter: None,
            offsets: alloc::vec![u64::from(u32::MAX)],
        };
        let bytes = saio.try_to_bytes().unwrap();
        let parsed = SampleAuxInfoOffsetsBox::parse_box(&bytes).unwrap();
        assert_eq!(parsed.offsets, alloc::vec![u64::from(u32::MAX)]);
    }

    // -----------------------------------------------------------------------
    // Audit 23 findings 24-29 — saio/schm/schi/sinf boundaries and labels
    // -----------------------------------------------------------------------

    fn full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> alloc::vec::Vec<u8> {
        let mut out = ((body.len() + BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE) as u32)
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(fourcc);
        out.push(version);
        out.extend_from_slice(&flags.to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    fn container_box(fourcc: &[u8; 4], body: &[u8]) -> alloc::vec::Vec<u8> {
        let mut out = ((body.len() + BOX_HEADER_MIN_SIZE) as u32)
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(fourcc);
        out.extend_from_slice(body);
        out
    }

    fn saio_box_bytes(
        version: u8,
        flags: u32,
        aux: Option<(u32, u32)>,
        offsets: &[u64],
    ) -> alloc::vec::Vec<u8> {
        let mut body = alloc::vec::Vec::new();
        if let Some((ty, param)) = aux {
            body.extend_from_slice(&ty.to_be_bytes());
            body.extend_from_slice(&param.to_be_bytes());
        }
        body.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for &off in offsets {
            if version == 0 {
                body.extend_from_slice(&(off as u32).to_be_bytes());
            } else {
                body.extend_from_slice(&off.to_be_bytes());
            }
        }
        full_box(b"saio", version, flags, &body)
    }

    fn assert_box_round_trips(parsed: &SampleAuxInfoOffsetsBox, bytes: &[u8]) {
        assert_eq!(parsed.serialized_len(), bytes.len());
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(out, bytes);
    }

    /// `saio` version 0 with three 32-bit offsets. With zero or one offset the
    /// `i * 4` byte indices are indistinguishable from a wrong stride.
    #[test]
    fn saio_v0_three_offsets_round_trip() {
        let bytes = saio_box_bytes(0, 0, None, &[0x0102_0304, 0x0506_0708, 0x090A_0B0C]);
        let saio = SampleAuxInfoOffsetsBox::parse_box(&bytes).unwrap();
        assert_eq!(saio.version, 0);
        assert_eq!(saio.flags, 0);
        assert_eq!(saio.aux_info_type, None);
        assert_eq!(
            saio.offsets,
            alloc::vec![0x0102_0304u64, 0x0506_0708, 0x090A_0B0C]
        );
        assert_box_round_trips(&saio, &bytes);
    }

    /// `saio` version 1 with two 64-bit offsets, each byte distinct.
    #[test]
    fn saio_v1_two_offsets_round_trip() {
        let bytes = saio_box_bytes(1, 0, None, &[0x0102_0304_0506_0708, 0x1112_1314_1516_1718]);
        let saio = SampleAuxInfoOffsetsBox::parse_box(&bytes).unwrap();
        assert_eq!(saio.version, 1);
        assert_eq!(
            saio.offsets,
            alloc::vec![0x0102_0304_0506_0708u64, 0x1112_1314_1516_1718]
        );
        assert_box_round_trips(&saio, &bytes);
    }

    /// `saio` version 1 with the `aux_info_type`/`aux_info_type_parameter`
    /// pair before the table (flags bit 0).
    #[test]
    fn saio_v1_aux_info_type_and_parameter_round_trip() {
        let bytes = saio_box_bytes(
            1,
            1,
            Some((0x7365_6e63, 9)),
            &[0x0102_0304_0506_0708, 0x1112_1314_1516_1718],
        );
        let saio = SampleAuxInfoOffsetsBox::parse_box(&bytes).unwrap();
        assert_eq!(saio.flags, 1);
        assert_eq!(saio.aux_info_type, Some(0x7365_6e63));
        assert_eq!(saio.aux_info_type_parameter, Some(9));
        assert_eq!(
            saio.offsets,
            alloc::vec![0x0102_0304_0506_0708u64, 0x1112_1314_1516_1718]
        );
        assert_box_round_trips(&saio, &bytes);
    }

    /// A truncated `saio` header, body and a truncated `frma` must be Err, not
    /// index panics: each guard sits directly in front of a fixed-offset read.
    #[test]
    fn saio_and_frma_truncations_are_rejected() {
        let mut header = 18u32.to_be_bytes().to_vec();
        header.extend_from_slice(b"saio");
        header.extend_from_slice(&[0, 0]);
        assert_eq!(header.len(), 10);
        let err = SampleAuxInfoOffsetsBox::parse_box(&header)
            .expect_err("a 10-byte saio header must be rejected");
        assert!(matches!(err, Error::BufferTooShort(_)), "{err:?}");

        let err = SampleAuxInfoOffsetsBox::parse_body(&[0, 0, 0], 0, 0)
            .expect_err("a 3-byte saio body lacks entry_count");
        assert!(matches!(err, Error::BufferTooShort(_)), "{err:?}");

        let mut frma = 18u32.to_be_bytes().to_vec();
        frma.extend_from_slice(b"frma");
        frma.extend_from_slice(b"av");
        assert_eq!(frma.len(), 10);
        let err = OriginalFormatBox::parse(&frma).expect_err("a 10-byte frma lacks its four-CC");
        assert!(matches!(err, Error::BufferTooShort(_)), "{err:?}");
    }

    /// A `schm` with `flags` bit 0 set carries its `scheme_uri`; the serializer
    /// writes it at the right offset and reports the right length.
    #[test]
    fn schm_with_a_scheme_uri_round_trips() {
        let mut body = b"cenc".to_vec();
        body.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        body.extend_from_slice(b"urn:x\0");
        let schm = SchemeTypeBox::parse_body(&body, 0, 0x000001).unwrap();
        assert_eq!(&schm.scheme_type, b"cenc");
        assert_eq!(schm.scheme_version, 0x0001_0000);
        assert_eq!(schm.scheme_uri, Some(b"urn:x\0".to_vec()));

        let expected = full_box(b"schm", 0, 0x000001, &body);
        assert_eq!(schm.serialized_len(), expected.len());
        let mut out = alloc::vec![0u8; schm.serialized_len()];
        let n = schm.serialize_into(&mut out).unwrap();
        assert_eq!(n, expected.len());
        assert_eq!(out, expected);
    }

    fn schi_with_extra_boxes() -> alloc::vec::Vec<u8> {
        let tenc = tenc_v1_with_constant_iv();
        let mut tenc_bytes = alloc::vec![0u8; tenc.serialized_len()];
        tenc.serialize_into(&mut tenc_bytes).unwrap();
        let mut body = tenc_bytes;
        body.extend_from_slice(&container_box(b"free", &[]));
        body.extend_from_slice(&container_box(b"zzzz", &[0x7E; 292]));
        // 0x0001_0010 bytes: the size field's first two bytes are 0 and 1, so
        // a read of the wrong size byte changes the parse.
        body.extend_from_slice(&container_box(b"big!", &[0x55; 65_544]));
        container_box(b"schi", &body)
    }

    /// A `schi` carrying `tenc` plus three vendor boxes keeps them all, sizes
    /// the box and round-trips. The 300-byte `zzzz` pins the third size byte,
    /// and the 65 552-byte `big!` pins the second.
    #[test]
    fn schi_preserves_extra_boxes() {
        let bytes = schi_with_extra_boxes();
        let schi = SchemeInformationBox::parse(&bytes).unwrap();
        assert!(schi.tenc.is_some(), "tenc still parsed");
        assert_eq!(schi.extra_boxes.len(), 3);
        assert_eq!(&schi.extra_boxes[0].box_type, b"free");
        assert!(schi.extra_boxes[0].data.is_empty());
        assert_eq!(&schi.extra_boxes[1].box_type, b"zzzz");
        assert_eq!(schi.extra_boxes[1].data.len(), 292);
        assert_eq!(&schi.extra_boxes[2].box_type, b"big!");
        assert_eq!(schi.extra_boxes[2].data.len(), 65_544);

        assert_eq!(schi.serialized_len(), bytes.len());
        let mut out = alloc::vec![0u8; schi.serialized_len()];
        let n = schi.serialize_into(&mut out).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(out, bytes);
    }

    fn sinf_with_extra_boxes() -> alloc::vec::Vec<u8> {
        let mut body = container_box(b"frma", b"avc1");
        let schm = SchemeTypeBox {
            version: 0,
            flags: 0,
            scheme_type: *b"cenc",
            scheme_version: 0x0001_0000,
            scheme_uri: None,
        };
        let mut schm_bytes = alloc::vec![0u8; schm.serialized_len()];
        schm.serialize_into(&mut schm_bytes).unwrap();
        body.extend_from_slice(&schm_bytes);
        body.extend_from_slice(&schi_with_extra_boxes());
        body.extend_from_slice(&container_box(b"free", &[]));
        body.extend_from_slice(&container_box(b"zzzz", &[0x7E; 292]));
        body.extend_from_slice(&container_box(b"big!", &[0x55; 65_544]));
        container_box(b"sinf", &body)
    }

    /// A `sinf` with `frma`/`schm`/`schi` plus three vendor boxes keeps them
    /// all and round-trips byte-exactly.
    #[test]
    fn sinf_preserves_extra_boxes() {
        let bytes = sinf_with_extra_boxes();
        let sinf = ProtectionSchemeInfoBox::parse(&bytes).unwrap();
        assert_eq!(&sinf.original_format.data_format, b"avc1");
        assert_eq!(
            sinf.scheme_type.as_ref().map(|s| s.scheme_type),
            Some(*b"cenc")
        );
        assert!(sinf.scheme_info.is_some(), "schi still parsed");
        assert_eq!(sinf.extra_boxes.len(), 3);
        assert_eq!(&sinf.extra_boxes[1].box_type, b"zzzz");
        assert_eq!(sinf.extra_boxes[1].data.len(), 292);
        assert_eq!(&sinf.extra_boxes[2].box_type, b"big!");
        assert_eq!(sinf.extra_boxes[2].data.len(), 65_544);

        assert_eq!(sinf.serialized_len(), bytes.len());
        let mut out = alloc::vec![0u8; sinf.serialized_len()];
        let n = sinf.serialize_into(&mut out).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(out, bytes);
    }

    /// A `sinf` whose only child is the 12-byte `frma` is valid: `schm` and
    /// `schi` are optional.
    #[test]
    fn sinf_with_only_a_frma_child_parses() {
        let bytes = container_box(b"sinf", &container_box(b"frma", b"avc1"));
        let sinf = ProtectionSchemeInfoBox::parse(&bytes).unwrap();
        assert_eq!(&sinf.original_format.data_format, b"avc1");
        assert!(sinf.scheme_type.is_none());
        assert!(sinf.scheme_info.is_none());
        assert!(sinf.extra_boxes.is_empty());
        assert_eq!(sinf.serialized_len(), bytes.len());
        let mut out = alloc::vec![0u8; sinf.serialized_len()];
        let n = sinf.serialize_into(&mut out).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(out, bytes);
    }

    /// A `sinf` body shorter than `frma` (12 bytes) reports the exact
    /// shortfall. A loosened `8 + 4` guard would reach the frma slice and
    /// report a different `have`.
    #[test]
    fn sinf_with_a_short_body_reports_the_exact_shortfall() {
        let mut body = 12u32.to_be_bytes().to_vec();
        body.extend_from_slice(b"frma");
        let bytes = container_box(b"sinf", &body);
        assert_eq!(bytes.len(), 16);
        let b = buffer_too_short(ProtectionSchemeInfoBox::parse(&bytes).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (12, 16, "sinf (frma)"));
    }

    /// A `schm` declaring 20 bytes but ending after 10 must be rejected, not
    /// sliced past the end.
    #[test]
    fn sinf_with_a_truncated_schm_is_rejected() {
        let mut body = container_box(b"frma", b"avc1");
        body.extend_from_slice(&20u32.to_be_bytes());
        body.extend_from_slice(b"schm");
        body.extend_from_slice(&[0, 0]);
        let bytes = container_box(b"sinf", &body);
        let err =
            ProtectionSchemeInfoBox::parse(&bytes).expect_err("a truncated schm must be rejected");
        assert!(matches!(err, Error::BufferTooShort(_)), "{err:?}");
    }

    /// A `schm` of exactly 12 bytes carries its FullBox fields but no
    /// `scheme_type`/`scheme_version`; the shortfall comes from the `schm`
    /// body reader, not the `sinf` container guard.
    #[test]
    fn sinf_with_a_four_byte_schm_body_reports_the_schm_shortfall() {
        let mut body = container_box(b"frma", b"avc1");
        body.extend_from_slice(&container_box(b"schm", &[0, 0, 0, 0]));
        let bytes = container_box(b"sinf", &body);
        let b = buffer_too_short(ProtectionSchemeInfoBox::parse(&bytes).unwrap_err());
        assert_eq!((b.need, b.have, b.what), (8, 0, "schm body"));
    }
}
