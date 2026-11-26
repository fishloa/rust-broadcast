//! EBML Variable-Size Integers: Element IDs and Element Data Sizes.
//!
//! RFC 8794 §4 (VINT), §5 (Element ID) and §6 (Element Data Size). An Element
//! ID keeps its marker bits, so its value is the whole encoded datum and its
//! width is implied by the value. An Element Data Size strips the marker and
//! carries its width separately, because §6.1 allows a size to be rendered at
//! more octets than its value needs; preserving that width is what makes a
//! re-serialize byte-identical.

use broadcast_core::{Parse, Serialize};

use crate::error::{Error, Result};

/// Most octets an Element ID may occupy (RFC 8794 §5).
pub const MAX_ELEMENT_ID_OCTETS: usize = 4;
/// Most octets an Element Data Size may occupy (RFC 8794 §6.1).
pub const MAX_VINT_OCTETS: usize = 8;
/// Bits carried by one octet.
const OCTET_BITS: u32 = 8;
/// Data bits carried by one VINT octet, the marker taking the eighth.
const VINT_DATA_BITS: u32 = 7;
/// The VINT marker bit in the first octet.
const VINT_MARKER: u8 = 0x80;
/// Mask of the first octet's data bits at width 1.
const VINT_FIRST_DATA_MASK: u8 = 0x7F;

/// Width of a Variable-Size Integer, 1..=8 octets (RFC 8794 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VintWidth(u8);

impl VintWidth {
    /// Narrowest VINT width.
    pub const MIN: Self = Self(1);
    /// Widest VINT width.
    pub const MAX: Self = Self(MAX_VINT_OCTETS as u8);

    /// Build a width, rejecting anything outside 1..=8 octets.
    pub fn try_new(width: usize) -> Result<Self> {
        if width < usize::from(Self::MIN.0) || width > MAX_VINT_OCTETS {
            return Err(Error::VintWidth { width });
        }
        let narrowed = u8::try_from(width).map_err(|_| Error::VintWidth { width })?;
        Ok(Self(narrowed))
    }

    /// The width in octets.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0 as usize
    }

    /// Bits of VINT_DATA this width carries (7 per octet).
    #[must_use]
    pub const fn data_bits(self) -> u32 {
        VINT_DATA_BITS * self.0 as u32
    }
}

/// An Element ID as read: 1..=4 octets including the marker bits
/// (RFC 8794 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ElementId(u32);

impl ElementId {
    /// Build an id from its stored value (marker bits included), rejecting an
    /// all-one VINT_DATA section and a non-shortest encoding.
    ///
    /// A one-octet id with all-zero VINT_DATA is accepted: RFC 8794 §5 forbids
    /// it, but RFC 9559 §27.1 (Erratum 7189) opens the one-octet range at
    /// `0x80` and the Matroska registry assigns `ChapterDisplay` that id.
    pub fn try_from_raw(value: u32) -> Result<Self> {
        let octets = octets_for(value);
        let width = VintWidth::try_new(octets)?;
        if octets > MAX_ELEMENT_ID_OCTETS {
            return Err(Error::ElementIdWidth { width: octets });
        }
        let mask = data_mask(width);
        let data = u64::from(value) & mask;
        if data == mask {
            return Err(Error::InvalidElementId { value });
        }
        if data == 0 && width != VintWidth::MIN {
            return Err(Error::InvalidElementId { value });
        }
        if minimal_width_for_data(data) != width {
            return Err(Error::InvalidElementId { value });
        }
        Ok(Self(value))
    }

    /// The stored value (marker bits included).
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The encoded width in octets, implied by the value.
    #[must_use]
    pub const fn octets(self) -> usize {
        octets_for(self.0)
    }

    /// Parse an Element ID from the front of `bytes`, returning it and the
    /// octets consumed.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        let first = *bytes.first().ok_or(Error::Truncated {
            what: "Element ID",
            need: 1,
            have: 0,
        })?;
        if first == 0 {
            return Err(Error::VintZeroStart);
        }
        let width = vint_width_from_first(first);
        if width > MAX_ELEMENT_ID_OCTETS {
            return Err(Error::ElementIdWidth { width });
        }
        let taken = bytes.get(..width).ok_or(Error::Truncated {
            what: "Element ID",
            need: width,
            have: bytes.len(),
        })?;
        let mut value = 0u32;
        for &octet in taken {
            value = (value << OCTET_BITS) | u32::from(octet);
        }
        Ok((Self::try_from_raw(value)?, width))
    }

    /// Write the id, returning the octets written.
    pub fn write(self, out: &mut [u8]) -> Result<usize> {
        let width = self.octets();
        let have = out.len();
        let target = out.get_mut(..width).ok_or(Error::Truncated {
            what: "Element ID output",
            need: width,
            have,
        })?;
        for (index, slot) in target.iter_mut().enumerate() {
            let shift = OCTET_BITS * (width - index - 1) as u32;
            *slot = (self.0 >> shift) as u8;
        }
        Ok(width)
    }
}

/// An Element Data Size: its value and the width it was encoded at
/// (RFC 8794 §6). The width is kept so a non-minimal encoding round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum VintSize {
    /// A known payload length in octets.
    Known {
        /// Payload length.
        value: u64,
        /// Encoded width.
        width: VintWidth,
    },
    /// The unknown size, used by live `Segment` and `Cluster` masters
    /// (RFC 8794 §6.2).
    Unknown {
        /// Encoded width.
        width: VintWidth,
    },
}

impl VintSize {
    /// The size at the narrowest width that holds `value`.
    pub fn minimal(value: u64) -> Result<Self> {
        let width = VintWidth::try_new(minimal_width_for_value(value))?;
        Ok(Self::Known { value, width })
    }

    /// The unknown size at the widest width.
    #[must_use]
    pub const fn unknown_max() -> Self {
        Self::Unknown {
            width: VintWidth::MAX,
        }
    }

    /// The known value, or `None` for the unknown size.
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        match self {
            Self::Known { value, .. } => Some(value),
            Self::Unknown { .. } => None,
        }
    }

    /// The encoded width.
    #[must_use]
    pub const fn width(self) -> VintWidth {
        match self {
            Self::Known { width, .. } | Self::Unknown { width } => width,
        }
    }

    /// Whether this is the unknown size.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    /// Parse an Element Data Size from the front of `bytes`, returning it and
    /// the octets consumed.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        let first = *bytes.first().ok_or(Error::Truncated {
            what: "Element Data Size",
            need: 1,
            have: 0,
        })?;
        if first == 0 {
            return Err(Error::VintZeroStart);
        }
        let width = VintWidth::try_new(vint_width_from_first(first))?;
        let taken = bytes.get(..width.get()).ok_or(Error::Truncated {
            what: "Element Data Size",
            need: width.get(),
            have: bytes.len(),
        })?;
        let first_data = first & (VINT_FIRST_DATA_MASK >> (width.get() - 1));
        let mut value = u64::from(first_data);
        for &octet in &taken[1..] {
            value = (value << OCTET_BITS) | u64::from(octet);
        }
        if value == data_mask(width) {
            return Ok((Self::Unknown { width }, width.get()));
        }
        Ok((Self::Known { value, width }, width.get()))
    }

    /// This size re-encoded for a new payload length, keeping the parsed
    /// width when it still fits. The unknown size stays unknown.
    pub fn resized(self, value: u64) -> Result<Self> {
        match self {
            Self::Unknown { width } => Ok(Self::Unknown { width }),
            Self::Known { width, .. } => {
                let needed = VintWidth::try_new(minimal_width_for_value(value))?;
                let width = if needed > width { needed } else { width };
                if value == data_mask(width) {
                    return Err(Error::SizeOverflow);
                }
                Ok(Self::Known { value, width })
            }
        }
    }

    /// The length of the encoded size field.
    #[must_use]
    pub const fn serialized_len(self) -> usize {
        self.width().get()
    }

    /// Write the size field, returning the octets written.
    pub fn write(self, out: &mut [u8]) -> Result<usize> {
        let width = self.width();
        let octets = width.get();
        let have = out.len();
        let target = out.get_mut(..octets).ok_or(Error::Truncated {
            what: "Element Data Size output",
            need: octets,
            have,
        })?;
        let raw = match self {
            Self::Known { value, .. } => {
                if value > data_mask(width) {
                    return Err(Error::SizeOverflow);
                }
                value
            }
            Self::Unknown { .. } => data_mask(width),
        };
        let marker_shift = VINT_DATA_BITS * (octets - 1) as u32;
        let marker = u64::from(VINT_MARKER) << marker_shift;
        let packed = marker | raw;
        for (index, slot) in target.iter_mut().enumerate() {
            let shift = OCTET_BITS * (octets - index - 1) as u32;
            *slot = (packed >> shift) as u8;
        }
        Ok(octets)
    }
}

impl<'a> Parse<'a> for VintSize {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self::parse(bytes)?.0)
    }
}

impl Serialize for VintSize {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        (*self).serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        (*self).write(buf)
    }
}

/// A VINT's width in octets, read from its non-zero first octet (§4.3): the
/// marker bit's distance from the top, plus one.
const fn vint_width_from_first(first: u8) -> usize {
    first.leading_zeros() as usize + 1
}

/// Octets needed to hold `value` as an unsigned integer.
const fn octets_for(value: u32) -> usize {
    let significant = (u32::BITS - value.leading_zeros()) as usize;
    if significant == 0 {
        return 1;
    }
    significant.div_ceil(OCTET_BITS as usize)
}

/// VINT_DATA mask for a width.
const fn data_mask(width: VintWidth) -> u64 {
    (1u64 << width.data_bits()) - 1
}

/// Narrowest width whose VINT_DATA can hold `data` without reading as the
/// all-one value that width reserves (RFC 8794 §5, §6.2).
fn minimal_width_for_data(data: u64) -> VintWidth {
    let mut width = VintWidth::MIN;
    while width < VintWidth::MAX && (data > data_mask(width) || data == data_mask(width)) {
        width = VintWidth(width.0 + 1);
    }
    width
}

/// Narrowest width whose VINT_DATA can hold `value`, capped at 8.
fn minimal_width_for_value(value: u64) -> usize {
    minimal_width_for_data(value).get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_one_byte_reads_the_value_and_the_unknown_marker() {
        // webm_demux's `vint_one_byte`: 0x81 -> width 1, value 1; 0xFF is the
        // one-octet unknown size (all data bits set).
        let (size, used) = VintSize::parse(&[0x81]).unwrap();
        assert_eq!(used, 1);
        assert_eq!(size.value(), Some(1));
        assert_eq!(size.width().get(), 1);
        let (size, used) = VintSize::parse(&[0xFF]).unwrap();
        assert_eq!(used, 1);
        assert!(size.is_unknown());
        assert_eq!(size.width().get(), 1);
    }

    #[test]
    fn vint_two_byte_reads_the_value() {
        // webm_demux's `vint_two_byte`: 0x40 0x02 -> width 2, value 2.
        let (size, used) = VintSize::parse(&[0x40, 0x02]).unwrap();
        assert_eq!(used, 2);
        assert_eq!(size.value(), Some(2));
        assert_eq!(size.width().get(), 2);
    }

    #[test]
    fn a_width_8_zero_length_field_is_a_known_zero() {
        // webm_demux's `every_vint_reader_agrees_on_a_width_8_zero_length_field`:
        // 0x01 starts an 8-octet VINT and the data bits are all zero, so this
        // is the known value 0 at width 8, never "truncated" or "unknown".
        let (size, used) =
            VintSize::parse(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(used, 8);
        assert_eq!(size.value(), Some(0));
        assert_eq!(size.width().get(), 8);
        assert!(!size.is_unknown());
    }

    #[test]
    fn element_id_reads_with_marker_bits_kept() {
        let (id, used) = ElementId::parse(&[0x18, 0x53, 0x80, 0x67]).unwrap();
        assert_eq!(used, 4);
        assert_eq!(id.value(), 0x1853_8067);
        assert_eq!(id.octets(), 4);

        let (id, used) = ElementId::parse(&[0xAE]).unwrap();
        assert_eq!(used, 1);
        assert_eq!(id.value(), 0xAE);
    }

    #[test]
    fn element_id_rejects_reserved_and_non_shortest_forms() {
        assert_eq!(
            ElementId::try_from_raw(0xFF),
            Err(Error::InvalidElementId { value: 0xFF })
        );
        // ChapterDisplay's one-octet id is the Matroska exception (Err7189).
        assert_eq!(ElementId::try_from_raw(0x80).unwrap().value(), 0x80);
        // A 2-octet encoding of the 1-octet id 0xBF is not the shortest form.
        assert_eq!(
            ElementId::try_from_raw(0x403F),
            Err(Error::InvalidElementId { value: 0x403F })
        );
    }

    #[test]
    fn element_id_round_trips_every_width() {
        for raw in [0x81u32, 0x407F, 0x203FFF, 0x101F_FFFF] {
            let id = ElementId::try_from_raw(raw).unwrap();
            let mut out = [0u8; MAX_ELEMENT_ID_OCTETS];
            let written = id.write(&mut out).unwrap();
            assert_eq!(&out[..written], &raw.to_be_bytes()[4 - written..]);
            let (back, used) = ElementId::parse(&out[..written]).unwrap();
            assert_eq!(used, written);
            assert_eq!(back, id);
        }
    }

    #[test]
    fn size_keeps_non_minimal_width() {
        // The vp9-opus.webm Segment declares a 3-octet size at 8-octet width.
        let (size, used) = VintSize::parse(&[0x01, 0, 0, 0, 0, 0x01, 0x67, 0x02]).unwrap();
        assert_eq!(used, 8);
        assert_eq!(size.value(), Some(0x1_6702));
        assert_eq!(size.width(), VintWidth::MAX);

        let mut out = [0u8; MAX_VINT_OCTETS];
        let written = size.write(&mut out).unwrap();
        assert_eq!(written, 8);
        assert_eq!(&out, &[0x01, 0, 0, 0, 0, 0x01, 0x67, 0x02]);
    }

    #[test]
    fn size_unknown_at_every_width() {
        for width in 1..=MAX_VINT_OCTETS {
            let encoded = [0u8; MAX_VINT_OCTETS];
            let size = VintSize::Unknown {
                width: VintWidth::try_new(width).unwrap(),
            };
            let mut out = encoded;
            let written = size.write(&mut out).unwrap();
            assert_eq!(written, width);
            let (back, used) = VintSize::parse(&out).unwrap();
            assert_eq!(used, width);
            assert_eq!(back, size);
        }
    }

    #[test]
    fn size_zero_is_known_not_unknown() {
        let (size, used) = VintSize::parse(&[0x80]).unwrap();
        assert_eq!(used, 1);
        assert_eq!(
            size,
            VintSize::Known {
                value: 0,
                width: VintWidth::MIN
            }
        );
    }

    #[test]
    fn minimal_size_is_the_narrowest_width() {
        assert_eq!(
            VintSize::minimal(0).unwrap(),
            VintSize::Known {
                value: 0,
                width: VintWidth::MIN
            }
        );
        assert_eq!(VintSize::minimal(126).unwrap().width().get(), 1);
        assert_eq!(VintSize::minimal(127).unwrap().width().get(), 2);
    }

    #[test]
    fn vint_width_rejects_out_of_range() {
        assert_eq!(VintWidth::try_new(0), Err(Error::VintWidth { width: 0 }));
        assert_eq!(VintWidth::try_new(9), Err(Error::VintWidth { width: 9 }));
    }
}
