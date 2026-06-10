use super::*;

impl ExtensionBodyDef for ImageIcon<'_> {
    const TAG_EXTENSION: u8 = 0x00;
    const NAME: &'static str = "IMAGE_ICON";
}

// ===========================================================================
//  Section 0x00 — image_icon_descriptor (Table 145, §6.4.7;
//               icon_transport_mode Table 146, §6.4.8;
//               coordinate_system Table 147, §6.4.8)
// ---------------------------------------------------------------------------
//  A fully typed length-determined descriptor. descriptor_number 0 carries
//  metadata + first payload chunk; descriptor_number ≠ 0 are continuations
//  with icon_data only. The caller reassembles the icon across the
//  descriptor_number 0..=last_descriptor_number sequence.
// ===========================================================================

/// image_icon body (Table 145). One descriptor instance; a full icon
/// spans `descriptor_number` 0..=`last_descriptor_number`, reassembled
/// by the caller.
///
/// Icon transport mode: Table 146
/// (`0` = data bytes, `1` = URL, `2`-`3` = reserved).
/// Coordinate system: Table 147
/// (`0` = 720×576, `1` = 1280×720, `2` = 1920×1080,
/// `3`-`6` = reserved, `7` = user defined).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ImageIcon<'a> {
    /// `descriptor_number` (4 bits) — `[7:4]` of byte 0.
    pub descriptor_number: u8,
    /// `last_descriptor_number` (4 bits) — `[3:0]` of byte 0.
    pub last_descriptor_number: u8,
    /// `icon_id` (3 bits) — `[2:0]` of byte 1.
    pub icon_id: u8,
    /// First-segment metadata vs. continuation payload (keyed by `descriptor_number == 0`).
    pub body: ImageIconBody<'a>,
}

/// First-segment metadata vs. continuation payload (keyed by `descriptor_number == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub enum ImageIconBody<'a> {
    /// `descriptor_number == 0`: icon metadata + first payload chunk.
    First(ImageIconFirst<'a>),
    /// `descriptor_number != 0`: a continuation icon_data chunk (length-prefixed).
    Continuation {
        /// Length-delimited icon_data_byte run.
        icon_data: &'a [u8],
    },
}

/// First-segment metadata (`descriptor_number == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ImageIconFirst<'a> {
    /// `icon_transport_mode` (2 bits) — Table 146.
    pub icon_transport_mode: u8,
    /// `position` is `Some` iff `position_flag == 1`.
    pub position: Option<IconPosition>,
    /// `icon_type_char` run (length-delimited in the wire).
    pub icon_type: &'a [u8],
    /// Transport-mode-dependent payload:
    /// `icon_transport_mode` 0 → `icon_data` bytes;
    /// 1 → URL bytes; 2-3 → empty.
    pub payload: &'a [u8],
}

/// Position data (present iff `position_flag == 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IconPosition {
    /// `coordinate_system` (3 bits) — Table 147.
    pub coordinate_system: u8,
    /// `icon_horizontal_origin` (12 bits).
    pub icon_horizontal_origin: u16,
    /// `icon_vertical_origin` (12 bits).
    pub icon_vertical_origin: u16,
}

impl<'a> Parse<'a> for ImageIcon<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        // Need at least byte0 (descriptor_number / last_descriptor_number)
        // and byte1 (reserved + icon_id).
        if sel.len() < 2 {
            return Err(invalid("image_icon: header truncated"));
        }
        let descriptor_number = sel[0] >> 4;
        let last_descriptor_number = sel[0] & 0x0F;
        let icon_id = sel[1] & 0x07;

        let body = if descriptor_number == 0x00 {
            // First segment: metadata + payload.
            if sel.len() < 3 {
                return Err(invalid("image_icon: first-segment packed byte missing"));
            }
            let packed = sel[2];
            let icon_transport_mode = packed >> 6;
            let position_flag = (packed >> 5) & 0x01;
            let mut pos = 3usize;

            let position = if position_flag == 1 {
                let coordinate_system = (packed >> 2) & 0x07;
                if sel.len() < pos + 3 {
                    return Err(invalid("image_icon: origin bytes truncated"));
                }
                let b0 = sel[pos];
                let b1 = sel[pos + 1];
                let b2 = sel[pos + 2];
                let icon_horizontal_origin = (u16::from(b0) << 4) | (u16::from(b1) >> 4);
                let icon_vertical_origin = ((u16::from(b1) & 0x0F) << 8) | u16::from(b2);
                pos += 3;
                Some(IconPosition {
                    coordinate_system,
                    icon_horizontal_origin,
                    icon_vertical_origin,
                })
            } else {
                None
            };

            // icon_type_length + run
            if sel.len() < pos + 1 {
                return Err(invalid("image_icon: icon_type_length truncated"));
            }
            let icon_type_length = sel[pos] as usize;
            pos += 1;
            if sel.len() < pos + icon_type_length {
                return Err(invalid("image_icon: icon_type overruns body"));
            }
            let icon_type = &sel[pos..pos + icon_type_length];
            pos += icon_type_length;

            // Transport-mode-dependent payload
            let payload = match icon_transport_mode {
                0 | 1 => {
                    if sel.len() < pos + 1 {
                        return Err(invalid("image_icon: payload length truncated"));
                    }
                    let payload_len = sel[pos] as usize;
                    pos += 1;
                    if sel.len() < pos + payload_len {
                        return Err(invalid("image_icon: payload overruns body"));
                    }
                    let p = &sel[pos..pos + payload_len];
                    pos += payload_len;
                    p
                }
                _ => &[][..],
            };

            if pos != sel.len() {
                return Err(invalid("image_icon: trailing bytes"));
            }

            ImageIconBody::First(ImageIconFirst {
                icon_transport_mode,
                position,
                icon_type,
                payload,
            })
        } else {
            // Continuation segment.
            if sel.len() < 3 {
                return Err(invalid("image_icon: continuation length truncated"));
            }
            let icon_data_length = sel[2] as usize;
            if sel.len() < 3 + icon_data_length {
                return Err(invalid("image_icon: continuation data overruns body"));
            }
            let icon_data = &sel[3..3 + icon_data_length];
            if 3 + icon_data_length != sel.len() {
                return Err(invalid("image_icon: trailing bytes"));
            }
            ImageIconBody::Continuation { icon_data }
        };

        Ok(ImageIcon {
            descriptor_number,
            last_descriptor_number,
            icon_id,
            body,
        })
    }
}

impl Serialize for ImageIcon<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 + match &self.body {
            ImageIconBody::First(f) => {
                1 // packed byte
                    + if f.position.is_some() { 3 } else { 0 }
                    + 1 // icon_type_length
                    + f.icon_type.len()
                    + match f.icon_transport_mode {
                        0 | 1 => 1 + f.payload.len(),
                        _ => 0,
                    }
            }
            ImageIconBody::Continuation { icon_data } => 1 + icon_data.len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // byte0: descriptor_number(4) | last_descriptor_number(4)
        buf[0] = (self.descriptor_number << 4) | (self.last_descriptor_number & 0x0F);
        // byte1: reserved_future_use(5)=1 | icon_id(3)
        buf[1] = 0xF8 | (self.icon_id & 0x07);
        let mut p = 2;
        match &self.body {
            ImageIconBody::First(f) => {
                // Packed byte: icon_transport_mode(2) | position_flag(1) | ...
                let position_flag = u8::from(f.position.is_some());
                if let Some(pos) = &f.position {
                    // ... | coordinate_system(3) | reserved_future_use(2)=1
                    buf[p] = (f.icon_transport_mode << 6)
                        | (position_flag << 5)
                        | ((pos.coordinate_system & 0x07) << 2)
                        | 0x03;
                    p += 1;
                    // 3 origin bytes: 12+12 bits packed
                    let h = pos.icon_horizontal_origin & 0x0FFF;
                    let v = pos.icon_vertical_origin & 0x0FFF;
                    buf[p] = (h >> 4) as u8;
                    buf[p + 1] = (((h & 0x0F) << 4) | ((v >> 8) & 0x0F)) as u8;
                    buf[p + 2] = v as u8;
                    p += 3;
                } else {
                    // ... | position_flag(1) | reserved_future_use(5)=1
                    buf[p] = (f.icon_transport_mode << 6) | (position_flag << 5) | 0x1F;
                    p += 1;
                }
                // icon_type_length + run
                buf[p] = f.icon_type.len() as u8;
                p += 1;
                buf[p..p + f.icon_type.len()].copy_from_slice(f.icon_type);
                p += f.icon_type.len();
                // Payload (mode 0 or 1 only)
                if f.icon_transport_mode == 0 || f.icon_transport_mode == 1 {
                    buf[p] = f.payload.len() as u8;
                    p += 1;
                    buf[p..p + f.payload.len()].copy_from_slice(f.payload);
                }
            }
            ImageIconBody::Continuation { icon_data } => {
                buf[p] = icon_data.len() as u8;
                p += 1;
                buf[p..p + icon_data.len()].copy_from_slice(icon_data);
            }
        }
        Ok(len)
    }
}
