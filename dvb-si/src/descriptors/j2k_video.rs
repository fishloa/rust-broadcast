//! J2K Video Descriptor — ISO/IEC 13818-1 §2.6.80, Table 2-101 (tag 0x32).
//!
//! Describes a JPEG 2000 (J2K) video elementary stream. The `extended_capability_flag`
//! controls a large nested block: when true, stripe/block/mdm flags appear,
//! followed by colour parameters and up to three conditional sub-blocks
//! (stripe, block, mastering display metadata). When false, `color_specification`
//! is present instead. `still_mode` and `interlaced_video` follow in both cases,
//! plus `private_data` tail bytes.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for J2K_video_descriptor.
pub const TAG: u8 = 0x32;
const HEADER_LEN: usize = 2;
/// Default of the 5-bit `reserved` in the extended-capability flags byte
/// (H.222.0 Table 2-101, 08/2018-08/2023, audit §4 row 14): the table says
/// "reserved (all bits to be set to '0')", an explicit exception to the
/// clause 2.1 "reserved = 1" rule.
pub const RESERVED_5_EXTENDED_DEFAULT: u8 = 0;
/// Default of the 6-bit `reserved` after `still_mode`/`interlaced_video`
/// (both branches) — plain `reserved`, clause 2.1 = `1` (all ones).
pub const RESERVED_6_DEFAULT: u8 = 0b11_1111;
/// Default of the 7-bit `reserved` after `video_full_range_flag` — plain
/// `reserved`, clause 2.1 = `1` (all ones).
pub const RESERVED_7_DEFAULT: u8 = 0x7F;
const RESERVED_5_MASK: u8 = 0x1F;
const RESERVED_6_MASK: u8 = 0x3F;
const RESERVED_7_MASK: u8 = 0x7F;

/// Stripe sub-block — present when extended_capability_flag AND stripe_flag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct J2kStripe {
    /// Maximum stripe index (1..=255, 0 is forbidden).
    pub strp_max_idx: u8,
    /// Default vertical size of a stripe.
    pub strp_height: u16,
}

/// Block sub-block — present when extended_capability_flag AND block_flag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct J2kBlock {
    /// Horizontal size of entire video frame.
    pub full_horizontal_size: u32,
    /// Vertical size of entire video frame.
    pub full_vertical_size: u32,
    /// Default width of a J2K block.
    pub blk_width: u16,
    /// Default height of a J2K block.
    pub blk_height: u16,
    /// Maximum block index in horizontal direction.
    pub max_blk_idx_h: u8,
    /// Maximum block index in vertical direction.
    pub max_blk_idx_v: u8,
    /// Block index in horizontal direction.
    pub blk_idx_h: u8,
    /// Block index in vertical direction.
    pub blk_idx_v: u8,
}

/// Mastering Display Metadata (MDM) sub-block — present when extended_capability_flag AND mdm_flag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct J2kMdm {
    /// X coordinate of primary 0 (× 16-bit).
    pub x_c0: u16,
    /// Y coordinate of primary 0 (× 16-bit).
    pub y_c0: u16,
    /// X coordinate of primary 1 (× 16-bit).
    pub x_c1: u16,
    /// Y coordinate of primary 1 (× 16-bit).
    pub y_c1: u16,
    /// X coordinate of primary 2 (× 16-bit).
    pub x_c2: u16,
    /// Y coordinate of primary 2 (× 16-bit).
    pub y_c2: u16,
    /// White point X.
    pub x_wp: u16,
    /// White point Y.
    pub y_wp: u16,
    /// Max luminance (cd/m² × 10000).
    pub l_max: u32,
    /// Min luminance (cd/m² × 10000).
    pub l_min: u32,
    /// Max Content Light Level.
    pub max_cll: u16,
    /// Max Frame Average Light Level.
    pub max_fall: u16,
}

/// Extended capability block — present when extended_capability_flag is true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct J2kExtendedCapability {
    /// Stripe mode enabled.
    pub stripe_flag: bool,
    /// Block mode enabled.
    pub block_flag: bool,
    /// Mastering display metadata present.
    pub mdm_flag: bool,
    /// Colour primaries (H.273 / ISO 23001-8).
    pub colour_primaries: u8,
    /// Transfer characteristics (H.273 / ISO 23001-8).
    pub transfer_characteristics: u8,
    /// Matrix coefficients (H.273 / ISO 23001-8).
    pub matrix_coefficients: u8,
    /// Video full range flag.
    pub video_full_range_flag: bool,
    /// Stripe sub-block — present when stripe_flag.
    pub stripe: Option<J2kStripe>,
    /// Block sub-block — present when block_flag.
    pub block: Option<J2kBlock>,
    /// Mastering display metadata sub-block — present when mdm_flag.
    pub mdm: Option<J2kMdm>,
    /// `reserved(5)` after the `mdm_flag` (5 bits, right-aligned; the spec
    /// says "all bits to be set to '0'"). Captured on parse, re-emitted
    /// verbatim; defaults to [`RESERVED_5_EXTENDED_DEFAULT`] (0).
    pub reserved_5: u8,
    /// `reserved(7)` after `video_full_range_flag` (7 bits, right-aligned).
    /// Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_7_DEFAULT`].
    pub reserved_7: u8,
}

impl J2kExtendedCapability {
    /// Build with the colour parameters; no sub-blocks, full-range flag clear
    /// and the spec-default reserved bits. Set the remaining fields directly
    /// (the three sub-block flags are derived from the sub-blocks on
    /// serialize).
    #[must_use]
    pub const fn new(
        colour_primaries: u8,
        transfer_characteristics: u8,
        matrix_coefficients: u8,
    ) -> Self {
        Self {
            stripe_flag: false,
            block_flag: false,
            mdm_flag: false,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            video_full_range_flag: false,
            stripe: None,
            block: None,
            mdm: None,
            reserved_5: RESERVED_5_EXTENDED_DEFAULT,
            reserved_7: RESERVED_7_DEFAULT,
        }
    }
}

/// J2K Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct J2kVideoDescriptor<'a> {
    /// Extended capability flag, as parsed from the wire.
    ///
    /// Informational only: [`Serialize`] derives the wire flag from
    /// `extended_capability.is_some()` (and likewise the stripe/block/mdm
    /// flags from their sub-blocks), so a stale value here can never
    /// misframe the descriptor (r03-W5).
    pub extended_capability_flag: bool,
    /// Profile and level (15-bit, from least significant 15 bits of Rsiz).
    pub profile_and_level: u16,
    /// Horizontal size of the frame/field.
    pub horizontal_size: u32,
    /// Vertical size of the frame/field.
    pub vertical_size: u32,
    /// Maximum bit rate.
    pub max_bit_rate: u32,
    /// Maximum buffer size.
    pub max_buffer_size: u32,
    /// Frame rate denominator.
    pub den_frame_rate: u16,
    /// Frame rate numerator.
    pub num_frame_rate: u16,
    /// Extended capability block (when flag is true).
    pub extended_capability: Option<J2kExtendedCapability>,
    /// Legacy color specification (when extended_capability_flag is false).
    pub color_specification: Option<u8>,
    /// Still picture mode.
    pub still_mode: bool,
    /// Interlaced video.
    pub interlaced_video: bool,
    /// Trailing private data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
    /// `reserved(6)` after `still_mode`/`interlaced_video` (6 bits,
    /// right-aligned; present in both the extended and the legacy branch).
    /// Captured on parse, re-emitted verbatim; defaults to
    /// [`RESERVED_6_DEFAULT`].
    pub reserved_6: u8,
}

impl<'a> J2kVideoDescriptor<'a> {
    /// Build a legacy-form descriptor (`extended_capability` absent,
    /// `color_specification` 0), zero rates/sizes beyond the given ones and
    /// the spec-default reserved bits. Set the remaining fields directly.
    #[must_use]
    pub const fn new(profile_and_level: u16, horizontal_size: u32, vertical_size: u32) -> Self {
        Self {
            extended_capability_flag: false,
            profile_and_level,
            horizontal_size,
            vertical_size,
            max_bit_rate: 0,
            max_buffer_size: 0,
            den_frame_rate: 0,
            num_frame_rate: 0,
            extended_capability: None,
            color_specification: Some(0),
            still_mode: false,
            interlaced_video: false,
            private_data: &[],
            reserved_6: RESERVED_6_DEFAULT,
        }
    }
}

impl Default for J2kVideoDescriptor<'_> {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

fn parse_extended_capability(
    body: &[u8],
    mut pos: usize,
) -> Result<(J2kExtendedCapability, bool, bool, u8, usize)> {
    // stripe_flag(1) + block_flag(1) + mdm_flag(1) + reserved(5) = 1 byte
    if body.len() < pos + 1 {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "J2K_video_descriptor too short for stripe/block/mdm flags",
        });
    }
    let sb = body[pos];
    let stripe_flag = (sb & 0x80) != 0;
    let block_flag = (sb & 0x40) != 0;
    let mdm_flag = (sb & 0x20) != 0;
    let reserved_5 = sb & RESERVED_5_MASK;
    pos += 1;

    // Table 2-101: still_mode(1) | interlaced_video(1) | reserved(6) comes
    // right after the stripe/block/mdm flags byte and BEFORE the colour
    // parameters and any sub-blocks — not after them.
    if body.len() < pos + 1 {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "J2K_video_descriptor too short for still_mode/interlaced byte",
        });
    }
    let sm_iv = body[pos];
    let still_mode = (sm_iv & 0x80) != 0;
    let interlaced_video = (sm_iv & 0x40) != 0;
    let reserved_6 = sm_iv & RESERVED_6_MASK;
    pos += 1;

    // colour_primaries(1) + transfer(1) + matrix(1) + video_full_range(1) + reserved(7) = 4 bytes
    if body.len() < pos + 4 {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "J2K_video_descriptor too short for colour parameters",
        });
    }
    let colour_primaries = body[pos];
    let transfer_characteristics = body[pos + 1];
    let matrix_coefficients = body[pos + 2];
    let vfr = body[pos + 3];
    let video_full_range_flag = (vfr & 0x80) != 0;
    let reserved_7 = vfr & RESERVED_7_MASK;
    pos += 4;

    // Stripe sub-block: strp_max_idx(1) + strp_height(2) = 3 bytes
    let stripe = if stripe_flag {
        if body.len() < pos + 3 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "J2K_video_descriptor too short for stripe sub-block",
            });
        }
        let strp_max_idx = body[pos];
        let strp_height = u16::from_be_bytes([body[pos + 1], body[pos + 2]]);
        pos += 3;
        Some(J2kStripe {
            strp_max_idx,
            strp_height,
        })
    } else {
        None
    };

    // Block sub-block: full_h(4) + full_v(4) + blk_w(2) + blk_h(2) + max_idx_h(1) + max_idx_v(1) + blk_idx_h(1) + blk_idx_v(1) = 16
    let block = if block_flag {
        if body.len() < pos + 16 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "J2K_video_descriptor too short for block sub-block",
            });
        }
        let full_horizontal_size =
            u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]);
        let full_vertical_size =
            u32::from_be_bytes([body[pos + 4], body[pos + 5], body[pos + 6], body[pos + 7]]);
        let blk_width = u16::from_be_bytes([body[pos + 8], body[pos + 9]]);
        let blk_height = u16::from_be_bytes([body[pos + 10], body[pos + 11]]);
        let max_blk_idx_h = body[pos + 12];
        let max_blk_idx_v = body[pos + 13];
        let blk_idx_h = body[pos + 14];
        let blk_idx_v = body[pos + 15];
        pos += 16;
        Some(J2kBlock {
            full_horizontal_size,
            full_vertical_size,
            blk_width,
            blk_height,
            max_blk_idx_h,
            max_blk_idx_v,
            blk_idx_h,
            blk_idx_v,
        })
    } else {
        None
    };

    // MDM sub-block: 6×u16(=12) + X_wp(2) + Y_wp(2) + L_max(4) + L_min(4) + MaxCLL(2) + MaxFALL(2) = 28
    let mdm = if mdm_flag {
        if body.len() < pos + 28 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "J2K_video_descriptor too short for MDM sub-block",
            });
        }
        let x_c0 = u16::from_be_bytes([body[pos], body[pos + 1]]);
        let y_c0 = u16::from_be_bytes([body[pos + 2], body[pos + 3]]);
        let x_c1 = u16::from_be_bytes([body[pos + 4], body[pos + 5]]);
        let y_c1 = u16::from_be_bytes([body[pos + 6], body[pos + 7]]);
        let x_c2 = u16::from_be_bytes([body[pos + 8], body[pos + 9]]);
        let y_c2 = u16::from_be_bytes([body[pos + 10], body[pos + 11]]);
        let x_wp = u16::from_be_bytes([body[pos + 12], body[pos + 13]]);
        let y_wp = u16::from_be_bytes([body[pos + 14], body[pos + 15]]);
        let l_max = u32::from_be_bytes([
            body[pos + 16],
            body[pos + 17],
            body[pos + 18],
            body[pos + 19],
        ]);
        let l_min = u32::from_be_bytes([
            body[pos + 20],
            body[pos + 21],
            body[pos + 22],
            body[pos + 23],
        ]);
        let max_cll = u16::from_be_bytes([body[pos + 24], body[pos + 25]]);
        let max_fall = u16::from_be_bytes([body[pos + 26], body[pos + 27]]);
        pos += 28;
        Some(J2kMdm {
            x_c0,
            y_c0,
            x_c1,
            y_c1,
            x_c2,
            y_c2,
            x_wp,
            y_wp,
            l_max,
            l_min,
            max_cll,
            max_fall,
        })
    } else {
        None
    };

    Ok((
        J2kExtendedCapability {
            stripe_flag,
            block_flag,
            mdm_flag,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            video_full_range_flag,
            stripe,
            block,
            mdm,
            reserved_5,
            reserved_7,
        },
        still_mode,
        interlaced_video,
        reserved_6,
        pos,
    ))
}

fn serialize_extended_capability(
    ext: &J2kExtendedCapability,
    still_mode: bool,
    interlaced_video: bool,
    reserved_6: u8,
    buf: &mut [u8],
    mut pos: usize,
) -> Result<usize> {
    // The three sub-block flags are derived from which sub-blocks exist, so a
    // stored flag can never promise bytes that are not written (r03-W5).
    let mut sb = broadcast_common::len::fit_mask(ext.reserved_5, RESERVED_5_MASK, "reserved_5")?;
    if ext.stripe.is_some() {
        sb |= 0x80;
    }
    if ext.block.is_some() {
        sb |= 0x40;
    }
    if ext.mdm.is_some() {
        sb |= 0x20;
    }
    buf[pos] = sb;
    pos += 1;

    // still_mode/interlaced_video precede the colour parameters here (Table 2-101).
    let mut sm_iv = broadcast_common::len::fit_mask(reserved_6, RESERVED_6_MASK, "reserved_6")?;
    if still_mode {
        sm_iv |= 0x80;
    }
    if interlaced_video {
        sm_iv |= 0x40;
    }
    buf[pos] = sm_iv;
    pos += 1;

    buf[pos] = ext.colour_primaries;
    buf[pos + 1] = ext.transfer_characteristics;
    buf[pos + 2] = ext.matrix_coefficients;
    buf[pos + 3] =
        if ext.video_full_range_flag {
            0x80
        } else {
            0x00
        } | broadcast_common::len::fit_mask(ext.reserved_7, RESERVED_7_MASK, "reserved_7")?;
    pos += 4;

    if let Some(ref s) = ext.stripe {
        buf[pos] = s.strp_max_idx;
        buf[pos + 1..pos + 3].copy_from_slice(&s.strp_height.to_be_bytes());
        pos += 3;
    }

    if let Some(ref b) = ext.block {
        buf[pos..pos + 4].copy_from_slice(&b.full_horizontal_size.to_be_bytes());
        buf[pos + 4..pos + 8].copy_from_slice(&b.full_vertical_size.to_be_bytes());
        buf[pos + 8..pos + 10].copy_from_slice(&b.blk_width.to_be_bytes());
        buf[pos + 10..pos + 12].copy_from_slice(&b.blk_height.to_be_bytes());
        buf[pos + 12] = b.max_blk_idx_h;
        buf[pos + 13] = b.max_blk_idx_v;
        buf[pos + 14] = b.blk_idx_h;
        buf[pos + 15] = b.blk_idx_v;
        pos += 16;
    }

    if let Some(ref m) = ext.mdm {
        buf[pos..pos + 2].copy_from_slice(&m.x_c0.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&m.y_c0.to_be_bytes());
        buf[pos + 4..pos + 6].copy_from_slice(&m.x_c1.to_be_bytes());
        buf[pos + 6..pos + 8].copy_from_slice(&m.y_c1.to_be_bytes());
        buf[pos + 8..pos + 10].copy_from_slice(&m.x_c2.to_be_bytes());
        buf[pos + 10..pos + 12].copy_from_slice(&m.y_c2.to_be_bytes());
        buf[pos + 12..pos + 14].copy_from_slice(&m.x_wp.to_be_bytes());
        buf[pos + 14..pos + 16].copy_from_slice(&m.y_wp.to_be_bytes());
        buf[pos + 16..pos + 20].copy_from_slice(&m.l_max.to_be_bytes());
        buf[pos + 20..pos + 24].copy_from_slice(&m.l_min.to_be_bytes());
        buf[pos + 24..pos + 26].copy_from_slice(&m.max_cll.to_be_bytes());
        buf[pos + 26..pos + 28].copy_from_slice(&m.max_fall.to_be_bytes());
        pos += 28;
    }

    Ok(pos)
}

fn extended_capability_serialized_len(ext: &J2kExtendedCapability) -> usize {
    let mut len: usize = 6; // flags(1) + still_mode/interlaced(1) + colour_primaries(1) + transfer(1) + matrix(1) + vfr(1)
    if ext.stripe.is_some() {
        len += 3;
    }
    if ext.block.is_some() {
        len += 16;
    }
    if ext.mdm.is_some() {
        len += 28;
    }
    len
}

impl<'a> Parse<'a> for J2kVideoDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "J2kVideoDescriptor",
            "unexpected tag for J2K_video_descriptor",
        )?;

        // Minimum before extended_capability decision:
        //   flag+profile_and_level(2) + h_size(4) + v_size(4) + max_bit_rate(4) + max_buf(4)
        //   + DEN(2) + NUM(2) = 22
        if body.len() < 22 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "J2K_video_descriptor too short (< 22 body bytes)",
            });
        }

        // extended_capability_flag(1) | profile_and_level(15)
        let b01 = u16::from_be_bytes([body[0], body[1]]);
        let extended_capability_flag = (b01 & 0x8000) != 0;
        let profile_and_level = b01 & 0x7FFF;

        let horizontal_size = u32::from_be_bytes([body[2], body[3], body[4], body[5]]);
        let vertical_size = u32::from_be_bytes([body[6], body[7], body[8], body[9]]);
        let max_bit_rate = u32::from_be_bytes([body[10], body[11], body[12], body[13]]);
        let max_buffer_size = u32::from_be_bytes([body[14], body[15], body[16], body[17]]);
        let den_frame_rate = u16::from_be_bytes([body[18], body[19]]);
        let num_frame_rate = u16::from_be_bytes([body[20], body[21]]);
        let pos = 22;

        let (
            extended_capability,
            color_specification,
            still_mode,
            interlaced_video,
            reserved_6,
            pos,
        ) = if extended_capability_flag {
            let (ext, still_mode, interlaced_video, reserved_6, new_pos) =
                parse_extended_capability(body, pos)?;
            (
                Some(ext),
                None,
                still_mode,
                interlaced_video,
                reserved_6,
                new_pos,
            )
        } else {
            // color_specification(1) then still_mode(1) | interlaced_video(1) | reserved(6)
            if body.len() < pos + 2 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "J2K_video_descriptor too short for color_specification/still_mode",
                });
            }
            let cs = body[pos];
            let sm_iv = body[pos + 1];
            let still_mode = (sm_iv & 0x80) != 0;
            let interlaced_video = (sm_iv & 0x40) != 0;
            (
                None,
                Some(cs),
                still_mode,
                interlaced_video,
                sm_iv & RESERVED_6_MASK,
                pos + 2,
            )
        };

        let private_data = &body[pos..];

        Ok(Self {
            extended_capability_flag,
            profile_and_level,
            horizontal_size,
            vertical_size,
            max_bit_rate,
            max_buffer_size,
            den_frame_rate,
            num_frame_rate,
            extended_capability,
            color_specification,
            still_mode,
            interlaced_video,
            private_data,
            reserved_6,
        })
    }
}

impl Serialize for J2kVideoDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + 22; // pre-flag fields
        if let Some(ref ext) = self.extended_capability {
            len += extended_capability_serialized_len(ext); // includes still_mode/interlaced byte
        } else {
            len += 2; // color_specification(1) + still_mode/interlaced byte(1)
        }
        len += self.private_data.len();
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;

        // extended_capability_flag(1) | profile_and_level(15)
        // Derived from the Option, never the stored flag (r03-W5).
        let mut b01 =
            broadcast_common::len::fit_mask(self.profile_and_level, 0x7FFF, "profile_and_level")?;
        if self.extended_capability.is_some() {
            b01 |= 0x8000;
        }
        buf[HEADER_LEN] = (b01 >> 8) as u8;
        buf[HEADER_LEN + 1] = b01 as u8;
        buf[HEADER_LEN + 2..HEADER_LEN + 6].copy_from_slice(&self.horizontal_size.to_be_bytes());
        buf[HEADER_LEN + 6..HEADER_LEN + 10].copy_from_slice(&self.vertical_size.to_be_bytes());
        buf[HEADER_LEN + 10..HEADER_LEN + 14].copy_from_slice(&self.max_bit_rate.to_be_bytes());
        buf[HEADER_LEN + 14..HEADER_LEN + 18].copy_from_slice(&self.max_buffer_size.to_be_bytes());
        buf[HEADER_LEN + 18..HEADER_LEN + 20].copy_from_slice(&self.den_frame_rate.to_be_bytes());
        buf[HEADER_LEN + 20..HEADER_LEN + 22].copy_from_slice(&self.num_frame_rate.to_be_bytes());
        let mut pos = HEADER_LEN + 22;

        if let Some(ref ext) = self.extended_capability {
            pos = serialize_extended_capability(
                ext,
                self.still_mode,
                self.interlaced_video,
                self.reserved_6,
                buf,
                pos,
            )?;
        } else {
            // serialized_len reserves color_specification + the still/interlaced
            // byte on this branch, so the byte must exist (r03-W5/W6).
            let cs = self.color_specification.ok_or(Error::ValueOutOfRange {
                field: "color_specification",
                reason: "required when extended_capability is absent",
            })?;
            buf[pos] = cs;
            pos += 1;

            let mut sm_iv =
                broadcast_common::len::fit_mask(self.reserved_6, RESERVED_6_MASK, "reserved_6")?;
            if self.still_mode {
                sm_iv |= 0x80;
            }
            if self.interlaced_video {
                sm_iv |= 0x40;
            }
            buf[pos] = sm_iv;
            pos += 1;
        }

        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for J2kVideoDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "J2K_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialize_round_trip(d: &J2kVideoDescriptor<'_>) {
        let mut buf = vec![0u8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, d.serialized_len());
        let reparsed = J2kVideoDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, reparsed, "round-trip mismatch");
    }

    #[test]
    fn round_trip_no_extended_capability() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: false,
            profile_and_level: 0x0102,
            horizontal_size: 1920,
            vertical_size: 1080,
            max_bit_rate: 10_000_000,
            max_buffer_size: 8_000_000,
            den_frame_rate: 1001,
            num_frame_rate: 24000,
            extended_capability: None,
            color_specification: Some(0x03),
            still_mode: false,
            interlaced_video: true,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_extended_capability_basic() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: true,
            profile_and_level: 0x0307,
            horizontal_size: 3840,
            vertical_size: 2160,
            max_bit_rate: 30_000_000,
            max_buffer_size: 15_000_000,
            den_frame_rate: 1001,
            num_frame_rate: 60000,
            extended_capability: Some(J2kExtendedCapability {
                stripe_flag: false,
                block_flag: false,
                mdm_flag: false,
                colour_primaries: 1,
                transfer_characteristics: 16,
                matrix_coefficients: 0,
                video_full_range_flag: true,
                reserved_5: RESERVED_5_EXTENDED_DEFAULT,
                reserved_7: RESERVED_7_DEFAULT,
                stripe: None,
                block: None,
                mdm: None,
            }),
            color_specification: None,
            still_mode: false,
            interlaced_video: false,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_extended_capability_stripe_only() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: true,
            profile_and_level: 0x0307,
            horizontal_size: 3840,
            vertical_size: 2160,
            max_bit_rate: 30_000_000,
            max_buffer_size: 15_000_000,
            den_frame_rate: 1,
            num_frame_rate: 60,
            extended_capability: Some(J2kExtendedCapability {
                stripe_flag: true,
                block_flag: false,
                mdm_flag: false,
                colour_primaries: 9,
                transfer_characteristics: 14,
                matrix_coefficients: 0,
                video_full_range_flag: false,
                reserved_5: RESERVED_5_EXTENDED_DEFAULT,
                reserved_7: RESERVED_7_DEFAULT,
                stripe: Some(J2kStripe {
                    strp_max_idx: 3,
                    strp_height: 1024,
                }),
                block: None,
                mdm: None,
            }),
            color_specification: None,
            still_mode: true,
            interlaced_video: false,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[0xAA],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_extended_capability_block_only() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: true,
            profile_and_level: 0x0307,
            horizontal_size: 1920,
            vertical_size: 1080,
            max_bit_rate: 20_000_000,
            max_buffer_size: 10_000_000,
            den_frame_rate: 1001,
            num_frame_rate: 30000,
            extended_capability: Some(J2kExtendedCapability {
                stripe_flag: false,
                block_flag: true,
                mdm_flag: false,
                colour_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
                video_full_range_flag: true,
                reserved_5: RESERVED_5_EXTENDED_DEFAULT,
                reserved_7: RESERVED_7_DEFAULT,
                stripe: None,
                block: Some(J2kBlock {
                    full_horizontal_size: 3840,
                    full_vertical_size: 2160,
                    blk_width: 1920,
                    blk_height: 1080,
                    max_blk_idx_h: 1,
                    max_blk_idx_v: 1,
                    blk_idx_h: 0,
                    blk_idx_v: 0,
                }),
                mdm: None,
            }),
            color_specification: None,
            still_mode: false,
            interlaced_video: false,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_extended_capability_mdm_only() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: true,
            profile_and_level: 0x0307,
            horizontal_size: 1920,
            vertical_size: 1080,
            max_bit_rate: 15_000_000,
            max_buffer_size: 8_000_000,
            den_frame_rate: 1,
            num_frame_rate: 25,
            extended_capability: Some(J2kExtendedCapability {
                stripe_flag: false,
                block_flag: false,
                mdm_flag: true,
                colour_primaries: 9,
                transfer_characteristics: 16,
                matrix_coefficients: 9,
                video_full_range_flag: false,
                reserved_5: RESERVED_5_EXTENDED_DEFAULT,
                reserved_7: RESERVED_7_DEFAULT,
                stripe: None,
                block: None,
                mdm: Some(J2kMdm {
                    x_c0: 1,
                    y_c0: 2,
                    x_c1: 3,
                    y_c1: 4,
                    x_c2: 5,
                    y_c2: 6,
                    x_wp: 7,
                    y_wp: 8,
                    l_max: 1000,
                    l_min: 5,
                    max_cll: 800,
                    max_fall: 400,
                }),
            }),
            color_specification: None,
            still_mode: false,
            interlaced_video: true,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[],
        };
        serialize_round_trip(&d);
    }

    #[test]
    fn round_trip_extended_capability_all() {
        let d = J2kVideoDescriptor {
            extended_capability_flag: true,
            profile_and_level: 0x0307,
            horizontal_size: 7680,
            vertical_size: 4320,
            max_bit_rate: 100_000_000,
            max_buffer_size: 50_000_000,
            den_frame_rate: 1001,
            num_frame_rate: 60000,
            extended_capability: Some(J2kExtendedCapability {
                stripe_flag: true,
                block_flag: true,
                mdm_flag: true,
                colour_primaries: 9,
                transfer_characteristics: 16,
                matrix_coefficients: 9,
                video_full_range_flag: true,
                reserved_5: RESERVED_5_EXTENDED_DEFAULT,
                reserved_7: RESERVED_7_DEFAULT,
                stripe: Some(J2kStripe {
                    strp_max_idx: 7,
                    strp_height: 540,
                }),
                block: Some(J2kBlock {
                    full_horizontal_size: 15360,
                    full_vertical_size: 8640,
                    blk_width: 7680,
                    blk_height: 4320,
                    max_blk_idx_h: 1,
                    max_blk_idx_v: 1,
                    blk_idx_h: 0,
                    blk_idx_v: 1,
                }),
                mdm: Some(J2kMdm {
                    x_c0: 6800,
                    y_c0: 3200,
                    x_c1: 2650,
                    y_c1: 6900,
                    x_c2: 1500,
                    y_c2: 600,
                    x_wp: 3127,
                    y_wp: 3290,
                    l_max: 4_000_000,
                    l_min: 50,
                    max_cll: 10_000,
                    max_fall: 5_000,
                }),
            }),
            color_specification: None,
            still_mode: true,
            interlaced_video: true,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[0xDD, 0xEE, 0xFF],
        };
        serialize_round_trip(&d);
    }

    /// Issue #1003: hand-assembled from H.222.0 Table 2-101 (NOT produced by
    /// this crate's serializer): after the stripe/block/mdm flags byte comes
    /// `still_mode | interlaced_video | reserved(6)`, THEN the colour
    /// parameters, then the stripe/block/mdm sub-blocks. Every field is
    /// asserted individually; the pre-fix reader took the still/interlaced
    /// byte as colour_primaries and shifted everything after it.
    #[test]
    fn extended_capability_spec_vector_field_by_field() {
        #[rustfmt::skip]
        let body: &[u8] = &[
            0x83, 0x07,                   // extended_capability_flag=1 | profile_and_level=0x0307
            0x00, 0x00, 0x1E, 0x00,       // horizontal_size = 7680
            0x00, 0x00, 0x10, 0xE0,       // vertical_size = 4320
            0x05, 0xF5, 0xE1, 0x00,       // max_bit_rate = 100_000_000
            0x02, 0xFA, 0xF0, 0x80,       // max_buffer_size = 50_000_000
            0x03, 0xE9,                   // DEN_frame_rate = 1001
            0xEA, 0x60,                   // NUM_frame_rate = 60000
            0xE0,                         // stripe|block|mdm = 1,1,1 | reserved(5)=0
            0xC1,                         // still_mode=1, interlaced=1, reserved(6)=1
            0x09, 0x10, 0x09, 0xFF,       // colour_primaries, transfer, matrix, full_range=1|rsv 0x7F
            0x07, 0x02, 0x1C,             // strp_max_idx=7, strp_height=540
            0x00, 0x00, 0x3C, 0x00,       // full_horizontal_size = 15360
            0x00, 0x00, 0x21, 0xC0,       // full_vertical_size = 8640
            0x1E, 0x00, 0x10, 0xE0,       // blk_width=7680, blk_height=4320
            0x01, 0x02, 0x03, 0x04,       // max_blk_idx_h/v, blk_idx_h/v
            0x1A, 0x90, 0x0C, 0x80,       // X_c0=6800 Y_c0=3200
            0x0A, 0x5A, 0x1A, 0xF4,       // X_c1=2650 Y_c1=6900
            0x05, 0xDC, 0x02, 0x58,       // X_c2=1500 Y_c2=600
            0x0C, 0x37, 0x0C, 0xDA,       // X_wp=3127 Y_wp=3290
            0x00, 0x3D, 0x09, 0x00,       // L_max = 4_000_000
            0x00, 0x00, 0x00, 0x32,       // L_min = 50
            0x27, 0x10, 0x13, 0x88,       // MaxCLL=10000 MaxFALL=5000
            0xDD, 0xEE,                   // private_data_byte x2
        ];
        let mut bytes = vec![TAG, body.len() as u8];
        bytes.extend_from_slice(body);
        let d = J2kVideoDescriptor::parse(&bytes).unwrap();
        assert!(d.extended_capability_flag);
        assert_eq!(d.profile_and_level, 0x0307);
        assert_eq!((d.horizontal_size, d.vertical_size), (7680, 4320));
        assert_eq!(
            (d.max_bit_rate, d.max_buffer_size),
            (100_000_000, 50_000_000)
        );
        assert_eq!((d.den_frame_rate, d.num_frame_rate), (1001, 60000));
        assert!(d.still_mode);
        assert!(d.interlaced_video);
        assert_eq!(d.reserved_6, 1);
        assert_eq!(d.color_specification, None);
        let ext = d.extended_capability.as_ref().unwrap();
        assert!(ext.stripe_flag && ext.block_flag && ext.mdm_flag);
        assert_eq!(
            (
                ext.colour_primaries,
                ext.transfer_characteristics,
                ext.matrix_coefficients
            ),
            (9, 16, 9)
        );
        assert!(ext.video_full_range_flag);
        assert_eq!(ext.reserved_7, 0x7F);
        let s = ext.stripe.as_ref().unwrap();
        assert_eq!((s.strp_max_idx, s.strp_height), (7, 540));
        let b = ext.block.as_ref().unwrap();
        assert_eq!(b.full_horizontal_size, 15360);
        assert_eq!(b.full_vertical_size, 8640);
        assert_eq!((b.blk_width, b.blk_height), (7680, 4320));
        assert_eq!(
            (b.max_blk_idx_h, b.max_blk_idx_v, b.blk_idx_h, b.blk_idx_v),
            (1, 2, 3, 4)
        );
        let m = ext.mdm.as_ref().unwrap();
        assert_eq!((m.x_c0, m.y_c0, m.x_c1, m.y_c1), (6800, 3200, 2650, 6900));
        assert_eq!((m.x_c2, m.y_c2, m.x_wp, m.y_wp), (1500, 600, 3127, 3290));
        assert_eq!((m.l_max, m.l_min), (4_000_000, 50));
        assert_eq!((m.max_cll, m.max_fall), (10_000, 5_000));
        assert_eq!(d.private_data, &[0xDD, 0xEE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let buf = [
            0x02, 22, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let err = J2kVideoDescriptor::parse(&buf).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = J2kVideoDescriptor::parse(&[TAG, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    fn plain<'a>() -> J2kVideoDescriptor<'a> {
        J2kVideoDescriptor {
            extended_capability_flag: false,
            profile_and_level: 0x0102,
            horizontal_size: 1920,
            vertical_size: 1080,
            max_bit_rate: 1,
            max_buffer_size: 2,
            den_frame_rate: 1001,
            num_frame_rate: 24000,
            extended_capability: None,
            color_specification: Some(0x03),
            still_mode: false,
            interlaced_video: false,
            reserved_6: RESERVED_6_DEFAULT,
            private_data: &[],
        }
    }

    /// r03-W5: the wire flags follow the Options, never the stored bools.
    #[test]
    fn serialize_derives_flags_from_options() {
        // Stored flag says extended, but no block exists -> legacy form.
        let mut d = plain();
        d.extended_capability_flag = true;
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = J2kVideoDescriptor::parse(&buf).unwrap();
        assert!(!re.extended_capability_flag);
        assert_eq!(re.color_specification, Some(0x03));

        // Extended block present but flag false, and stored sub-block flags
        // lie in both directions (stripe_flag without stripe, stripe without flag).
        let mut d = plain();
        d.color_specification = None;
        d.extended_capability = Some(J2kExtendedCapability {
            stripe_flag: true,
            block_flag: false,
            mdm_flag: true,
            colour_primaries: 1,
            transfer_characteristics: 2,
            matrix_coefficients: 3,
            video_full_range_flag: false,
            reserved_5: RESERVED_5_EXTENDED_DEFAULT,
            reserved_7: RESERVED_7_DEFAULT,
            stripe: None,
            block: Some(J2kBlock {
                full_horizontal_size: 1,
                full_vertical_size: 2,
                blk_width: 3,
                blk_height: 4,
                max_blk_idx_h: 5,
                max_blk_idx_v: 6,
                blk_idx_h: 7,
                blk_idx_v: 8,
            }),
            mdm: None,
        });
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = J2kVideoDescriptor::parse(&buf).unwrap();
        assert!(re.extended_capability_flag);
        let ext = re.extended_capability.unwrap();
        assert!(!ext.stripe_flag && ext.block_flag && !ext.mdm_flag);
        assert!(ext.stripe.is_none() && ext.block.is_some() && ext.mdm.is_none());
    }

    /// Legacy form with no color_specification used to leave a stale byte.
    #[test]
    fn serialize_rejects_missing_color_specification() {
        let mut d = plain();
        d.color_specification = None;
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf),
            Err(Error::ValueOutOfRange {
                field: "color_specification",
                ..
            })
        ));
    }

    /// H.222.0 Table 2-101. All three reserved regions are preserved
    /// verbatim (zero, ones and mixed values), both branches.
    #[test]
    fn j2k_video_reserved_preserved() {
        let head = |ext: bool| {
            let mut v = vec![TAG, 0u8];
            let b01: u16 = if ext { 0x8000 | 0x0307 } else { 0x0307 };
            v.extend_from_slice(&b01.to_be_bytes());
            v.extend_from_slice(&1920u32.to_be_bytes());
            v.extend_from_slice(&1080u32.to_be_bytes());
            v.extend_from_slice(&1u32.to_be_bytes());
            v.extend_from_slice(&2u32.to_be_bytes());
            v.extend_from_slice(&1001u16.to_be_bytes());
            v.extend_from_slice(&24000u16.to_be_bytes());
            v
        };
        for r6 in [0u8, 1, 0x15, 0x2A, 0x3F] {
            // Extended branch.
            for r5 in [0u8, 1, 0x0A, 0x1F] {
                for r7 in [0u8, 1, 0x55, 0x7F] {
                    let mut bytes = head(true);
                    bytes.extend_from_slice(&[r5, 0x80 | r6, 1, 16, 0, 0x80 | r7]);
                    bytes[1] = (bytes.len() - 2) as u8;
                    let d = J2kVideoDescriptor::parse(&bytes).unwrap();
                    assert_eq!(d.reserved_6, r6);
                    let ext = d.extended_capability.as_ref().unwrap();
                    assert_eq!((ext.reserved_5, ext.reserved_7), (r5, r7));
                    let mut out = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut out).unwrap();
                    assert_eq!(out, bytes);
                }
            }
            // Legacy branch.
            let mut bytes = head(false);
            bytes.extend_from_slice(&[0x03, 0x40 | r6]);
            bytes[1] = (bytes.len() - 2) as u8;
            let d = J2kVideoDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved_6, r6);
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
        }
    }

    /// Defaults: the explicit-extended-branch `reserved(5)` is `0` ("all bits
    /// to be set to '0'"); every other reserved region is all ones.
    #[test]
    fn j2k_video_default_is_spec_value() {
        let mut d = J2kVideoDescriptor::new(0x0307, 1920, 1080);
        d.color_specification = None;
        d.extended_capability = Some(J2kExtendedCapability::new(1, 16, 0));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        let tail = &out[out.len() - 6..];
        assert_eq!(tail, [0x00, 0x3F, 1, 16, 0, 0x7F]);

        let legacy = J2kVideoDescriptor::new(0x0102, 1920, 1080);
        let mut out = vec![0u8; legacy.serialized_len()];
        legacy.serialize_into(&mut out).unwrap();
        assert_eq!(&out[out.len() - 2..], [0x00, 0x3F]);
    }
}
