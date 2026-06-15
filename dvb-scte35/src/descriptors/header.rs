//! Shared splice_descriptor() header handling — ANSI/SCTE 35 2023r1 §10.2,
//! Table 17.
//!
//! Every splice descriptor begins with the same six bytes:
//! `splice_descriptor_tag` (1) + `descriptor_length` (1) + `identifier` (4).

use crate::error::{Error, Result};

/// The SCTE-registered `identifier` for descriptors defined in this spec:
/// ASCII "CUEI" (§10.2.1).
pub const CUEI: u32 = 0x4355_4549;

/// Bytes of the fixed header: tag + length + 4-byte identifier.
pub const HEADER_LEN: usize = 6;

/// Validate a splice_descriptor header and return `(identifier, body)` where
/// `body` is the bytes after the identifier, bounded by `descriptor_length`.
///
/// `descriptor_length` counts the bytes *following* it (so it includes the
/// 4-byte identifier); the returned body is `descriptor_length - 4` bytes.
pub fn descriptor_body<'a>(
    bytes: &'a [u8],
    expected_tag: u8,
    what: &'static str,
) -> Result<(u32, &'a [u8])> {
    if bytes.len() < 2 {
        return Err(Error::BufferTooShort {
            need: 2,
            have: bytes.len(),
            what,
        });
    }
    if bytes[0] != expected_tag {
        return Err(Error::UnexpectedDescriptorTag {
            tag: bytes[0],
            what,
            expected: expected_tag,
        });
    }
    let declared = bytes[1] as usize; // bytes following descriptor_length
    let total = 2 + declared;
    if bytes.len() < total {
        return Err(Error::LengthOverflow {
            declared,
            available: bytes.len().saturating_sub(2),
            what,
        });
    }
    if declared < 4 {
        return Err(Error::BufferTooShort {
            need: 6,
            have: total,
            what,
        });
    }
    // bytes[0..2] are tag + descriptor_length; bytes[2..6] are the identifier.
    let (id_bytes, _) = bytes[2..total]
        .split_first_chunk::<4>()
        .ok_or(Error::BufferTooShort {
            need: 6,
            have: total,
            what,
        })?;
    let identifier = u32::from_be_bytes(*id_bytes);
    Ok((identifier, &bytes[HEADER_LEN..total]))
}

/// Write a splice_descriptor header into `buf`. `body_len` is the length of the
/// bytes following the identifier; `descriptor_length` is `4 + body_len`.
pub fn write_header(buf: &mut [u8], tag: u8, identifier: u32, body_len: usize) {
    buf[0] = tag;
    buf[1] = (4 + body_len) as u8;
    buf[2..6].copy_from_slice(&identifier.to_be_bytes());
}
