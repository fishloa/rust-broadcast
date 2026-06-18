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
use dvb_common::{Parse, Serialize};

/// Descriptor tag for J2K_video_descriptor.
pub const TAG: u8 = 0x32;
const HEADER_LEN: usize = 2;

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
}

/// J2K Video Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct J2kVideoDescriptor<'a> {
    /// Extended capability flag.
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
}

fn parse_extended_capability(
    body: &[u8],
    mut pos: usize,
) -> Result<(J2kExtendedCapability, usize)> {
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
        },
        pos,
    ))
}

fn serialize_extended_capability(
    ext: &J2kExtendedCapability,
    buf: &mut [u8],
    mut pos: usize,
) -> usize {
    let mut sb = 0u8;
    if ext.stripe_flag {
        sb |= 0x80;
    }
    if ext.block_flag {
        sb |= 0x40;
    }
    if ext.mdm_flag {
        sb |= 0x20;
    }
    buf[pos] = sb;
    pos += 1;

    buf[pos] = ext.colour_primaries;
    buf[pos + 1] = ext.transfer_characteristics;
    buf[pos + 2] = ext.matrix_coefficients;
    buf[pos + 3] = if ext.video_full_range_flag {
        0x80
    } else {
        0x00
    };
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

    pos
}

fn extended_capability_serialized_len(ext: &J2kExtendedCapability) -> usize {
    let mut len: usize = 5; // flags(1) + colour_primaries(1) + transfer(1) + matrix(1) + vfr(1)
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
        let mut pos = 22;

        let (extended_capability, color_specification, mut pos) = if extended_capability_flag {
            let (ext, new_pos) = parse_extended_capability(body, pos)?;
            (Some(ext), None, new_pos)
        } else {
            // color_specification(1)
            if body.len() < pos + 1 {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "J2K_video_descriptor too short for color_specification",
                });
            }
            let cs = body[pos];
            pos += 1;
            (None, Some(cs), pos)
        };

        // still_mode(1) | interlaced_video(1) | reserved(6) = 1 byte
        if body.len() < pos + 1 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "J2K_video_descriptor too short for still_mode/interlaced byte",
            });
        }
        let sm_iv = body[pos];
        let still_mode = (sm_iv & 0x80) != 0;
        let interlaced_video = (sm_iv & 0x40) != 0;
        pos += 1;

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
        })
    }
}

impl Serialize for J2kVideoDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let mut len: usize = HEADER_LEN + 23; // pre-flag fields(22) + still_mode byte(1)
        if let Some(ref ext) = self.extended_capability {
            len += extended_capability_serialized_len(ext);
        } else {
            len += 1; // color_specification
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
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;

        // extended_capability_flag(1) | profile_and_level(15)
        let mut b01 = self.profile_and_level & 0x7FFF;
        if self.extended_capability_flag {
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
            pos = serialize_extended_capability(ext, buf, pos);
        } else if let Some(cs) = self.color_specification {
            buf[pos] = cs;
            pos += 1;
        }

        let mut sm_iv = 0u8;
        if self.still_mode {
            sm_iv |= 0x80;
        }
        if self.interlaced_video {
            sm_iv |= 0x40;
        }
        buf[pos] = sm_iv;
        pos += 1;

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
                stripe: None,
                block: None,
                mdm: None,
            }),
            color_specification: None,
            still_mode: false,
            interlaced_video: false,
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
            private_data: &[0xDD, 0xEE, 0xFF],
        };
        serialize_round_trip(&d);
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
}
