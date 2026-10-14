//! Image Icon Descriptor — ETSI EN 300 468 §6.4.7 (tag_extension 0x00).
use super::*;

/// `descriptor_number(4)`/`last_descriptor_number(4)` raw field mask —
/// masked at the shift so a wide value cannot bleed across the nibble
/// boundary (r03-W2).
const DESCRIPTOR_NUMBER_MASK: u8 = 0x0F;
/// `icon_id(3)` raw field mask (Table 145).
const ICON_ID_MASK: u8 = 0x07;
/// `icon_transport_mode(2)` value mask — bounds `IconTransportMode::Reserved(v)`.
const TRANSPORT_MODE_VALUE_MASK: u8 = 0x03;
/// `coordinate_system(3)` raw field mask (Table 147).
const COORDINATE_SYSTEM_MASK: u8 = 0x07;

/// Spec default of the 5-bit `reserved_future_use` fields (the one in byte 1
/// and the one that follows `position_flag == 0`): all ones, EN 300 468 Table
/// 145 in every edition V1.9.1-V1.20.1
/// (`dvb-si/docs/reserved-bits-edition-audit.md` §6).
pub const IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT: u8 = 0x1F;
/// Spec default of the 2-bit `reserved_future_use` after `coordinate_system`:
/// all ones (same source as [`IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT`]).
pub const IMAGE_ICON_RESERVED_FUTURE_USE_2_DEFAULT: u8 = 0x03;
const RESERVED_5_MASK: u8 = 0x1F;
const RESERVED_2_MASK: u8 = 0x03;

/// Icon transport mode — ETSI EN 300 468 §6.4.8 Table 146
/// (`dvb-si/docs/enums/en_300_468/146-icon-transport-mode-coding.md`).
///
/// 2-bit field. Selects where the icon data is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IconTransportMode {
    /// `0` — the icon is delivered in the `icon_data_byte` sequence of bytes.
    InlineData,
    /// `1` — the location of the icon file is identified by the URL conveyed in
    /// the `url_char` sequence of bytes.
    Url,
    /// `2`–`3` — reserved for future use.
    Reserved(u8),
}

impl IconTransportMode {
    #[must_use]
    /// Creates a value from a 2-bit wire nibble (upper bits masked off),
    /// preserving every possible value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::InlineData,
            1 => Self::Url,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the 2-bit wire nibble for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::InlineData => 0,
            Self::Url => 1,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns the spec token for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::InlineData => "inline data",
            Self::Url => "URL",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(IconTransportMode, Reserved);

impl<'a> ExtensionBodyDef<'a> for ImageIcon<'a> {
    const TAG_EXTENSION: u8 = 0x00;
    const NAME: &'static str = "IMAGE_ICON";
}

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
#[non_exhaustive]
pub struct ImageIcon<'a> {
    /// `descriptor_number` (4 bits) — `[7:4]` of byte 0.
    pub descriptor_number: u8,
    /// `last_descriptor_number` (4 bits) — `[3:0]` of byte 0.
    pub last_descriptor_number: u8,
    /// `reserved_future_use` (5 bits, right-aligned) — `[7:3]` of byte 1.
    /// Preserved on parse; [`ImageIcon::new`] uses
    /// [`IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT`] (all ones).
    pub reserved_future_use: u8,
    /// `icon_id` (3 bits) — `[2:0]` of byte 1.
    pub icon_id: u8,
    /// First-segment metadata vs. continuation payload (keyed by `descriptor_number == 0`).
    pub body: ImageIconBody<'a>,
}

/// First-segment metadata vs. continuation payload (keyed by `descriptor_number == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub enum ImageIconBody<'a> {
    /// `descriptor_number == 0`: icon metadata + first payload chunk.
    First(ImageIconFirst<'a>),
    /// `descriptor_number != 0`: a continuation icon_data chunk (length-prefixed).
    Continuation {
        /// Length-delimited icon_data_byte run.
        icon_data: &'a [u8],
    },
}

impl<'a> ImageIcon<'a> {
    /// Build a descriptor with the spec-default (all ones) reserved bits.
    #[must_use]
    pub const fn new(
        descriptor_number: u8,
        last_descriptor_number: u8,
        icon_id: u8,
        body: ImageIconBody<'a>,
    ) -> Self {
        Self {
            descriptor_number,
            last_descriptor_number,
            reserved_future_use: IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT,
            icon_id,
            body,
        }
    }
}

/// First-segment metadata (`descriptor_number == 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct ImageIconFirst<'a> {
    /// `icon_transport_mode` (2 bits) — Table 146.
    pub icon_transport_mode: IconTransportMode,
    /// `position` is `Some` iff `position_flag == 1`.
    pub position: Option<IconPosition>,
    /// `reserved_future_use` (5 bits, right-aligned) that follows
    /// `position_flag == 0`; `Some` iff `position` is `None`. Preserved on
    /// parse; a hand-built `None` serializes as
    /// [`IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT`].
    pub reserved_future_use: Option<u8>,
    /// `icon_type_char` text run (length-delimited in the wire).
    pub icon_type: crate::text::DvbText<'a>,
    /// Transport-mode-dependent payload (Table 146):
    /// `0` → icon data bytes; `1` → URL text; `2`-`3` → empty.
    pub payload: IconLocation<'a>,
}

/// Transport-mode-dependent payload for the first segment of an image_icon
/// descriptor (Table 146, §6.4.8).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub enum IconLocation<'a> {
    /// `icon_transport_mode` 0: raw icon data bytes (PNG, JPEG, etc.).
    Data(&'a [u8]),
    /// `icon_transport_mode` 1: `url_char` text pointing to the icon resource.
    Url(crate::text::DvbText<'a>),
    /// `icon_transport_mode` 2–3: reserved/empty.
    None,
}

impl<'a> ImageIconFirst<'a> {
    /// Build first-segment metadata with the spec-default (all ones) reserved
    /// bits for the given `position` presence.
    #[must_use]
    pub const fn new(
        icon_transport_mode: IconTransportMode,
        position: Option<IconPosition>,
        icon_type: crate::text::DvbText<'a>,
        payload: IconLocation<'a>,
    ) -> Self {
        let reserved_future_use = if position.is_some() {
            None
        } else {
            Some(IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT)
        };
        Self {
            icon_transport_mode,
            position,
            reserved_future_use,
            icon_type,
            payload,
        }
    }
}

/// Position data (present iff `position_flag == 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IconPosition {
    /// `coordinate_system` (3 bits) — Table 147.
    pub coordinate_system: u8,
    /// `reserved_future_use` (2 bits, right-aligned) after `coordinate_system`.
    /// Preserved on parse; [`IconPosition::new`] uses
    /// [`IMAGE_ICON_RESERVED_FUTURE_USE_2_DEFAULT`] (all ones).
    pub reserved_future_use: u8,
    /// `icon_horizontal_origin` (12 bits).
    pub icon_horizontal_origin: u16,
    /// `icon_vertical_origin` (12 bits).
    pub icon_vertical_origin: u16,
}

impl IconPosition {
    /// Build a position with the spec-default (all ones) reserved bits.
    #[must_use]
    pub const fn new(
        coordinate_system: u8,
        icon_horizontal_origin: u16,
        icon_vertical_origin: u16,
    ) -> Self {
        Self {
            coordinate_system,
            reserved_future_use: IMAGE_ICON_RESERVED_FUTURE_USE_2_DEFAULT,
            icon_horizontal_origin,
            icon_vertical_origin,
        }
    }
}

impl<'a> Parse<'a> for ImageIcon<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        // Need at least byte0 (descriptor_number / last_descriptor_number)
        // and byte1 (reserved + icon_id).
        if sel.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: sel.len(),
                what: "image_icon body",
            });
        }
        let descriptor_number = sel[0] >> 4;
        let last_descriptor_number = sel[0] & 0x0F;
        let icon_id = sel[1] & ICON_ID_MASK;
        let reserved_future_use = (sel[1] >> 3) & RESERVED_5_MASK;

        let body = if descriptor_number == 0x00 {
            // First segment: metadata + payload.
            if sel.len() < 3 {
                return Err(Error::BufferTooShort {
                    need: 3,
                    have: sel.len(),
                    what: "image_icon body",
                });
            }
            let packed = sel[2];
            let icon_transport_mode = IconTransportMode::from_u8(packed >> 6);
            let position_flag = (packed >> 5) & 0x01;
            let mut pos = 3usize;

            let mut no_position_reserved = None;
            let position = if position_flag == 1 {
                let coordinate_system = (packed >> 2) & 0x07;
                let position_reserved = packed & RESERVED_2_MASK;
                if sel.len() < pos + 3 {
                    return Err(Error::BufferTooShort {
                        need: pos + 3,
                        have: sel.len(),
                        what: "image_icon body",
                    });
                }
                let b0 = sel[pos];
                let b1 = sel[pos + 1];
                let b2 = sel[pos + 2];
                let icon_horizontal_origin = (u16::from(b0) << 4) | (u16::from(b1) >> 4);
                let icon_vertical_origin = ((u16::from(b1) & 0x0F) << 8) | u16::from(b2);
                pos += 3;
                Some(IconPosition {
                    coordinate_system,
                    reserved_future_use: position_reserved,
                    icon_horizontal_origin,
                    icon_vertical_origin,
                })
            } else {
                no_position_reserved = Some(packed & RESERVED_5_MASK);
                None
            };

            // icon_type_length + run
            if sel.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "image_icon body",
                });
            }
            let icon_type_length = sel[pos] as usize;
            pos += 1;
            if sel.len() < pos + icon_type_length {
                return Err(Error::BufferTooShort {
                    need: pos + icon_type_length,
                    have: sel.len(),
                    what: "image_icon body",
                });
            }
            let icon_type = crate::text::DvbText::new(&sel[pos..pos + icon_type_length]);
            pos += icon_type_length;

            // Transport-mode-dependent payload
            let payload = match icon_transport_mode {
                IconTransportMode::InlineData => {
                    if sel.len() < pos + 1 {
                        return Err(Error::BufferTooShort {
                            need: pos + 1,
                            have: sel.len(),
                            what: "image_icon body",
                        });
                    }
                    let payload_len = sel[pos] as usize;
                    pos += 1;
                    if sel.len() < pos + payload_len {
                        return Err(Error::BufferTooShort {
                            need: pos + payload_len,
                            have: sel.len(),
                            what: "image_icon body",
                        });
                    }
                    let p = &sel[pos..pos + payload_len];
                    pos += payload_len;
                    IconLocation::Data(p)
                }
                IconTransportMode::Url => {
                    if sel.len() < pos + 1 {
                        return Err(Error::BufferTooShort {
                            need: pos + 1,
                            have: sel.len(),
                            what: "image_icon body",
                        });
                    }
                    let payload_len = sel[pos] as usize;
                    pos += 1;
                    if sel.len() < pos + payload_len {
                        return Err(Error::BufferTooShort {
                            need: pos + payload_len,
                            have: sel.len(),
                            what: "image_icon body",
                        });
                    }
                    let t = crate::text::DvbText::new(&sel[pos..pos + payload_len]);
                    pos += payload_len;
                    IconLocation::Url(t)
                }
                _ => IconLocation::None,
            };

            if pos != sel.len() {
                return Err(invalid("image_icon: trailing bytes"));
            }

            ImageIconBody::First(ImageIconFirst {
                icon_transport_mode,
                position,
                reserved_future_use: no_position_reserved,
                icon_type,
                payload,
            })
        } else {
            // Continuation segment.
            if sel.len() < 3 {
                return Err(Error::BufferTooShort {
                    need: 3,
                    have: sel.len(),
                    what: "image_icon body",
                });
            }
            let icon_data_length = sel[2] as usize;
            if sel.len() < 3 + icon_data_length {
                return Err(Error::BufferTooShort {
                    need: 3 + icon_data_length,
                    have: sel.len(),
                    what: "image_icon body",
                });
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
            reserved_future_use,
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
                    + match &f.payload {
                        IconLocation::Data(d) => 1 + d.len(),
                        IconLocation::Url(u) => 1 + u.len(),
                        IconLocation::None => 0,
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
        buf[0] = (broadcast_common::len::fit_mask(
            self.descriptor_number,
            DESCRIPTOR_NUMBER_MASK,
            "descriptor_number",
        )? << 4)
            | broadcast_common::len::fit_mask(
                self.last_descriptor_number,
                DESCRIPTOR_NUMBER_MASK,
                "last_descriptor_number",
            )?;
        // byte1: reserved_future_use(5) (preserved) | icon_id(3)
        buf[1] = (broadcast_common::len::fit_mask(
            self.reserved_future_use,
            RESERVED_5_MASK,
            "reserved_future_use",
        )? << 3)
            | broadcast_common::len::fit_mask(self.icon_id, ICON_ID_MASK, "icon_id")?;
        let mut p = 2;
        match &self.body {
            ImageIconBody::First(f) => {
                // Packed byte: icon_transport_mode(2) | position_flag(1) | ...
                let position_flag = u8::from(f.position.is_some());
                let itm = broadcast_common::len::fit_mask(
                    f.icon_transport_mode.to_u8(),
                    TRANSPORT_MODE_VALUE_MASK,
                    "icon_transport_mode",
                )?;
                if let Some(pos) = &f.position {
                    // ... | coordinate_system(3) | reserved_future_use(2) (preserved)
                    buf[p] = (itm << 6)
                        | (position_flag << 5)
                        | (broadcast_common::len::fit_mask(
                            pos.coordinate_system,
                            COORDINATE_SYSTEM_MASK,
                            "coordinate_system",
                        )? << 2)
                        | broadcast_common::len::fit_mask(
                            pos.reserved_future_use,
                            RESERVED_2_MASK,
                            "reserved_future_use",
                        )?;
                    p += 1;
                    // 3 origin bytes: 12+12 bits packed
                    let h = broadcast_common::len::fit_mask(
                        pos.icon_horizontal_origin,
                        0x0FFF,
                        "icon_horizontal_origin",
                    )?;
                    let v = broadcast_common::len::fit_mask(
                        pos.icon_vertical_origin,
                        0x0FFF,
                        "icon_vertical_origin",
                    )?;
                    buf[p] = (h >> 4) as u8;
                    buf[p + 1] = (((h & 0x0F) << 4) | ((v >> 8) & 0x0F)) as u8;
                    buf[p + 2] = v as u8;
                    p += 3;
                } else {
                    // ... | position_flag(1) | reserved_future_use(5) (preserved)
                    buf[p] = (itm << 6)
                        | (position_flag << 5)
                        | broadcast_common::len::fit_mask(
                            f.reserved_future_use
                                .unwrap_or(IMAGE_ICON_RESERVED_FUTURE_USE_5_DEFAULT),
                            RESERVED_5_MASK,
                            "reserved_future_use",
                        )?;
                    p += 1;
                }
                // icon_type_length + run
                buf[p] = broadcast_common::len::fit_u8(f.icon_type.len(), "icon_type_length")?;
                p += 1;
                buf[p..p + f.icon_type.len()].copy_from_slice(f.icon_type.raw());
                p += f.icon_type.len();
                // Payload
                match &f.payload {
                    IconLocation::Data(d) => {
                        buf[p] = broadcast_common::len::fit_u8(d.len(), "icon_data_length")?;
                        p += 1;
                        buf[p..p + d.len()].copy_from_slice(d);
                    }
                    IconLocation::Url(u) => {
                        buf[p] = broadcast_common::len::fit_u8(u.len(), "icon_url_length")?;
                        p += 1;
                        buf[p..p + u.len()].copy_from_slice(u.raw());
                    }
                    IconLocation::None => {}
                }
            }
            ImageIconBody::Continuation { icon_data } => {
                buf[p] = broadcast_common::len::fit_u8(icon_data.len(), "icon_data_length")?;
                p += 1;
                buf[p..p + icon_data.len()].copy_from_slice(icon_data);
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_image_icon_first_position_mode0() {
        // descriptor_number=0, last=3, icon_id=5
        // position_flag=1, coordinate_system=1, h_origin=100, v_origin=200
        // icon_type=[0xDE, 0xAD], mode=0 icon_data=[0x01, 0x02, 0x03]
        // byte0: (0<<4)|3 = 0x03
        // byte1: icon_id=5, reserved=0 = 0x05
        // packed: (0<<6)|(1<<5)|(1<<2) = 0x24
        // origin: h=100=0x0064, v=200=0x00C8 → b0=0x06, b1=0x40, b2=0xC8
        let sel = [
            0x03, 0x05, 0x24, 0x06, 0x40, 0xC8, 0x02, 0xDE, 0xAD, 0x03, 0x01, 0x02, 0x03,
        ];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ImageIcon));
        match &d.body {
            ExtensionBody::ImageIcon(b) => {
                assert_eq!(b.descriptor_number, 0);
                assert_eq!(b.last_descriptor_number, 3);
                assert_eq!(b.icon_id, 5);
                match &b.body {
                    ImageIconBody::First(f) => {
                        assert_eq!(f.icon_transport_mode, IconTransportMode::InlineData);
                        assert!(f.position.is_some());
                        let pos = f.position.as_ref().unwrap();
                        assert_eq!(pos.coordinate_system, 1);
                        assert_eq!(pos.icon_horizontal_origin, 100);
                        assert_eq!(pos.icon_vertical_origin, 200);
                        assert_eq!(f.icon_type.raw(), &[0xDE, 0xAD]);
                        match &f.payload {
                            IconLocation::Data(d) => assert_eq!(*d, &[0x01, 0x02, 0x03]),
                            other => panic!("expected Data, got {other:?}"),
                        }
                    }
                    other => panic!("expected First, got {other:?}"),
                }
            }
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_first_no_position_mode1() {
        // descriptor_number=0, last=1, icon_id=7
        // position_flag=0, mode=1 (URL), icon_type=[0xAB], url=b"http"
        // byte0: (0<<4)|1 = 0x01; byte1: 0x07
        // packed: (1<<6)|(0<<5) = 0x40
        let sel = [0x01, 0x07, 0x40, 0x01, 0xAB, 0x04, b'h', b't', b't', b'p'];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ImageIcon(b) => match &b.body {
                ImageIconBody::First(f) => {
                    assert_eq!(f.icon_transport_mode, IconTransportMode::Url);
                    assert!(f.position.is_none());
                    assert_eq!(f.icon_type.raw(), &[0xAB]);
                    match &f.payload {
                        IconLocation::Url(u) => assert_eq!(u.decode(), "http"),
                        other => panic!("expected Url, got {other:?}"),
                    }
                }
                other => panic!("expected First, got {other:?}"),
            },
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_first_mode2_empty_payload() {
        // descriptor_number=0, last=0, icon_id=0, mode=2 (reserved),
        // position_flag=0, icon_type=0 bytes, empty payload
        // byte0: 0x00; byte1: 0x00; packed: (2<<6) = 0x80
        let sel = [0x00, 0x00, 0x80, 0x00];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ImageIcon(b) => match &b.body {
                ImageIconBody::First(f) => {
                    assert_eq!(f.icon_transport_mode, IconTransportMode::Reserved(2));
                    assert!(f.position.is_none());
                    assert!(f.icon_type.is_empty());
                    assert!(matches!(f.payload, IconLocation::None));
                }
                other => panic!("expected First, got {other:?}"),
            },
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_continuation() {
        // descriptor_number=2, last=3, icon_id=1
        // icon_data=[0xAA, 0xBB, 0xCC, 0xDD]
        // byte0: (2<<4)|3 = 0x23; byte1: 0x01; length=4
        let sel = [0x23, 0x01, 0x04, 0xAA, 0xBB, 0xCC, 0xDD];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ImageIcon));
        match &d.body {
            ExtensionBody::ImageIcon(b) => {
                assert_eq!(b.descriptor_number, 2);
                assert_eq!(b.last_descriptor_number, 3);
                assert_eq!(b.icon_id, 1);
                match &b.body {
                    ImageIconBody::Continuation { icon_data } => {
                        assert_eq!(icon_data, &[0xAA, 0xBB, 0xCC, 0xDD]);
                    }
                    other => panic!("expected Continuation, got {other:?}"),
                }
            }
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Every value of each reserved_future_use region (5-bit in byte 1,
    /// 2-bit after coordinate_system, 5-bit after position_flag==0)
    /// re-serializes byte-identically.
    #[test]
    fn image_icon_reserved_preserved() {
        let check = |sel: &[u8], what: &str, r: u8| {
            let bytes = wrap(0x00, sel);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "{what} r={r:#x}");
        };
        for r in 0u8..=RESERVED_5_MASK {
            // continuation, byte1 reserved
            check(&[0x23, (r << 3) | 0x01, 0x01, 0xAA], "byte1", r);
            // first, no position: packed reserved(5)
            check(&[0x00, 0xF8, 0x40 | r, 0x00, 0x00], "no-position", r);
        }
        for r in 0u8..=RESERVED_2_MASK {
            // first, position (cs=1), mode 0, icon_type 0 bytes, data 0 bytes
            check(
                &[
                    0x00,
                    0xF8,
                    0x20 | (1 << 2) | r,
                    0x06,
                    0x40,
                    0xC8,
                    0x00,
                    0x00,
                ],
                "position",
                r,
            );
        }
    }

    #[test]
    fn image_icon_default_is_spec_value() {
        let pos = IconPosition::new(1, 100, 200);
        let first = ImageIconFirst::new(
            IconTransportMode::InlineData,
            Some(pos),
            crate::text::DvbText::new(&[]),
            IconLocation::Data(&[]),
        );
        let d = ImageIcon::new(0, 0, 5, ImageIconBody::First(first));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // byte1 = 0xF8|5; packed = pos flag 0x20 | cs 1<<2 | rfu 0x03.
        assert_eq!(
            buf,
            [0x00, 0xFD, 0x20 | 0x04 | 0x03, 0x06, 0x40, 0xC8, 0x00, 0x00]
        );
        let first = ImageIconFirst::new(
            IconTransportMode::Url,
            None,
            crate::text::DvbText::new(&[]),
            IconLocation::Url(crate::text::DvbText::new(&[])),
        );
        let d = ImageIcon::new(0, 0, 0, ImageIconBody::First(first));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0x00, 0xF8, 0x40 | 0x1F, 0x00, 0x00]);
    }

    #[test]
    fn parse_image_icon_rejects_trailing_bytes() {
        // First segment with an extra trailing byte.
        // mode=2, one extra byte 0xFF after the complete parse.
        let sel = [0x00, 0x00, 0x80, 0x00, 0xFF];
        let bytes = wrap(0x00, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::InvalidDescriptor {
                tag: super::TAG,
                ..
            }
        ));
    }

    #[test]
    fn parse_image_icon_rejects_truncated_continuation() {
        // Continuation with length=5 but only 3 data bytes.
        let sel = [0x23, 0x01, 0x05, 0xAA, 0xBB, 0xCC];
        let bytes = wrap(0x00, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_image_icon() {
        let d = ExtensionDescriptor {
            tag_extension: 0x00,
            body: ExtensionBody::ImageIcon(ImageIcon::new(
                2,
                3,
                1,
                ImageIconBody::Continuation {
                    icon_data: &[0xAA, 0xBB],
                },
            )),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":0"));
        assert!(json.contains("\"imageIcon\""));
    }

    #[test]
    fn icon_transport_mode_full_range_round_trip() {
        for v in 0u8..=0x03 {
            let itm = IconTransportMode::from_u8(v);
            assert_eq!(
                itm.to_u8(),
                v,
                "IconTransportMode round-trip failed for {v}"
            );
        }
    }

    #[test]
    fn icon_transport_mode_known_values() {
        assert_eq!(IconTransportMode::from_u8(0), IconTransportMode::InlineData);
        assert_eq!(IconTransportMode::from_u8(1), IconTransportMode::Url);
        assert_eq!(
            IconTransportMode::from_u8(2),
            IconTransportMode::Reserved(2)
        );
        assert_eq!(IconTransportMode::InlineData.name(), "inline data");
        assert_eq!(IconTransportMode::Url.name(), "URL");
        assert_eq!(IconTransportMode::Reserved(3).name(), "reserved");
    }
}
