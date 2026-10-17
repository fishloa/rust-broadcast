//! IP/MAC_generic_stream_location_descriptor — ETSI EN 301 192 V1.7.1
//! §8.4.5.15 Table 33 (tag 0x15), `modulation_system_type` per Table 34,
//! `PHY_stream_id` per Table 35.
//!
//! Locates an IP/MAC stream carried in a *generic* (not MPEG-2 TS) stream of a
//! DVB modulation system, via `interactive_network_id`,
//! `modulation_system_type`, `modulation_system_id` and `PHY_stream_id`.
//! `selector_length_flag`/`selector_flags`/`selector_length`/`selector_byte`
//! "shall be encoded according to ETSI TS 102 606-2, clause 5.2.2.8" (not
//! transcribed in this workspace, so the selector content is kept opaque).
//! INT platform and operational loops (Table 19).

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for IP/MAC_generic_stream_location_descriptor.
pub const TAG: u8 = 0x15;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 8;
const SELECTOR_LENGTH_FLAG: u8 = 0x80;
const SELECTOR_VALUE_MASK: u8 = 0x7F;
const SELECTOR_VALUE_BITS: u32 = 7;

/// `modulation_system_type` — ETSI EN 301 192 §8.4.5.15 Table 34.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ModulationSystemType {
    /// 0x00 — DVB-S2 (EN 302 307-1).
    Dvbs2,
    /// 0x01 — DVB-T2 (EN 302 755).
    Dvbt2,
    /// 0x02 — DVB-C2 (EN 302 769).
    Dvbc2,
    /// 0x03 — DVB-NGH (DVB BlueBook A160).
    DvbNgh,
    /// 0x04 — DVB-S2X (EN 302 307-2).
    Dvbs2x,
    /// 0x05..=0xFF — reserved for future use.
    Reserved(u8),
}

impl ModulationSystemType {
    /// Decode from the wire value. Every value maps (lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Dvbs2,
            0x01 => Self::Dvbt2,
            0x02 => Self::Dvbc2,
            0x03 => Self::DvbNgh,
            0x04 => Self::Dvbs2x,
            _ => Self::Reserved(v),
        }
    }

    /// Encode to the wire value. Inverse of [`from_u8`](Self::from_u8).
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Dvbs2 => 0x00,
            Self::Dvbt2 => 0x01,
            Self::Dvbc2 => 0x02,
            Self::DvbNgh => 0x03,
            Self::Dvbs2x => 0x04,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 34).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Dvbs2 => "DVB-S2",
            Self::Dvbt2 => "DVB-T2",
            Self::Dvbc2 => "DVB-C2",
            Self::DvbNgh => "DVB-NGH",
            Self::Dvbs2x => "DVB-S2X",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ModulationSystemType, Reserved);

/// Bit shift of the high sub-field of a two-sub-field `PHY_stream_id`.
const PHY_HIGH_SHIFT: u32 = 8;

/// `PHY_stream_id` decoded per its `modulation_system_type` — EN 301 192
/// §8.4.5.15 Table 35 (verified on the rendered page 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum PhyStreamId {
    /// DVB-S2: the 16-bit Input Stream Identifier (ISI).
    Isi(u16),
    /// DVB-T2 / DVB-NGH: the 16-bit Physical Layer Pipe (PLP) identifier.
    Plp(u16),
    /// DVB-C2: data slice id in `b15..b8`, PLP id in `b7..b0`.
    DataSlicePlp {
        /// Data slice identifier (`b15..b8`).
        data_slice_id: u8,
        /// PLP identifier (`b7..b0`).
        plp_id: u8,
    },
    /// DVB-S2X: time slice number in `b15..b8`, ISI in `b7..b0`.
    TimeSliceIsi {
        /// Time slice number (`b15..b8`).
        time_slice: u8,
        /// Input Stream Identifier (`b7..b0`).
        isi: u8,
    },
    /// Reserved `modulation_system_type`: Table 35 defines no encoding, so
    /// the raw 16 bits are kept.
    Unspecified(u16),
}

impl PhyStreamId {
    /// Decode the 16-bit wire value for a given `modulation_system_type`.
    #[must_use]
    pub fn decode(system: ModulationSystemType, raw: u16) -> Self {
        let [hi, lo] = raw.to_be_bytes();
        match system {
            ModulationSystemType::Dvbs2 => Self::Isi(raw),
            ModulationSystemType::Dvbt2 | ModulationSystemType::DvbNgh => Self::Plp(raw),
            ModulationSystemType::Dvbc2 => Self::DataSlicePlp {
                data_slice_id: hi,
                plp_id: lo,
            },
            ModulationSystemType::Dvbs2x => Self::TimeSliceIsi {
                time_slice: hi,
                isi: lo,
            },
            ModulationSystemType::Reserved(_) => Self::Unspecified(raw),
        }
    }

    /// Encode to the 16-bit wire value. Inverse of [`decode`](Self::decode).
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        match self {
            Self::Isi(v) | Self::Plp(v) | Self::Unspecified(v) => v,
            Self::DataSlicePlp {
                data_slice_id,
                plp_id,
            } => ((data_slice_id as u16) << PHY_HIGH_SHIFT) | plp_id as u16,
            Self::TimeSliceIsi { time_slice, isi } => {
                ((time_slice as u16) << PHY_HIGH_SHIFT) | isi as u16
            }
        }
    }
}

/// The selector of a generic stream location: either seven flag bits or a
/// length-delimited byte string, discriminated by `selector_length_flag`.
/// The content is defined by ETSI TS 102 606-2 §5.2.2.8; that standard is not
/// vendored (no PDF or transcription of it exists in the repository), so the
/// selector is carried opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum GenericStreamSelector<'a> {
    /// `selector_length_flag == 0`: the 7-bit `selector_flags`.
    Flags(u8),
    /// `selector_length_flag == 1`: `selector_length` then `selector_byte`s.
    Bytes(&'a [u8]),
}

/// IP/MAC_generic_stream_location_descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IpMacGenericStreamLocationDescriptor<'a> {
    /// `interactive_network_id` — set to the same value as the `network_id`.
    pub interactive_network_id: u16,
    /// `modulation_system_type` (Table 34).
    pub modulation_system_type: ModulationSystemType,
    /// `modulation_system_id` — S2/T2/C2/NGH_system_id within the network.
    pub modulation_system_id: u16,
    /// `PHY_stream_id` — ISI / PLP / slice+PLP, encoded per Table 35.
    pub phy_stream_id: u16,
    /// `selector_length_flag` + `selector_flags` / `selector_length` +
    /// `selector_byte`s.
    pub selector: GenericStreamSelector<'a>,
    /// Bytes after the selector inside `descriptor_length`; the spec defines
    /// none (kept for byte-identical re-serialization).
    pub trailing: &'a [u8],
}

impl<'a> IpMacGenericStreamLocationDescriptor<'a> {
    /// `PHY_stream_id` split into its Table 35 sub-fields according to
    /// [`modulation_system_type`](Self::modulation_system_type).
    #[must_use]
    pub fn phy_stream(&self) -> PhyStreamId {
        PhyStreamId::decode(self.modulation_system_type, self.phy_stream_id)
    }

    fn selector_len(&self) -> usize {
        match self.selector {
            GenericStreamSelector::Flags(_) => 1,
            GenericStreamSelector::Bytes(b) => 1 + b.len(),
        }
    }
}

impl<'a> Parse<'a> for IpMacGenericStreamLocationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IpMacGenericStreamLocationDescriptor",
            "unexpected tag for IP/MAC_generic_stream_location_descriptor",
        )?;
        let (f, rest) = body
            .split_first_chunk::<FIXED_LEN>()
            .ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "IP/MAC_generic_stream_location_descriptor body shorter than 8 bytes",
            })?;
        let value = f[7] & SELECTOR_VALUE_MASK;
        let (selector, trailing) = if f[7] & SELECTOR_LENGTH_FLAG == 0 {
            (GenericStreamSelector::Flags(value), rest)
        } else {
            let n = usize::from(value);
            if rest.len() < n {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "selector_length runs past descriptor end",
                });
            }
            let (sel, trailing) = rest.split_at(n);
            (GenericStreamSelector::Bytes(sel), trailing)
        };
        Ok(Self {
            interactive_network_id: u16::from_be_bytes([f[0], f[1]]),
            modulation_system_type: ModulationSystemType::from_u8(f[2]),
            modulation_system_id: u16::from_be_bytes([f[3], f[4]]),
            phy_stream_id: u16::from_be_bytes([f[5], f[6]]),
            selector,
            trailing,
        })
    }
}

impl Serialize for IpMacGenericStreamLocationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN - 1 + self.selector_len() + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let (flag, value, sel): (u8, u8, &[u8]) = match self.selector {
            GenericStreamSelector::Flags(flags) => (
                0,
                broadcast_common::len::fit_mask(flags, SELECTOR_VALUE_MASK, "selector_flags")?,
                &[],
            ),
            GenericStreamSelector::Bytes(b) => (
                SELECTOR_LENGTH_FLAG,
                u8::try_from(broadcast_common::len::fit_bits(
                    b.len() as u64,
                    SELECTOR_VALUE_BITS,
                    "selector_length",
                )?)
                .map_err(|_| Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "selector_length does not fit",
                })?,
                b,
            ),
        };
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2..4].copy_from_slice(&self.interactive_network_id.to_be_bytes());
        buf[4] = self.modulation_system_type.to_u8();
        buf[5..7].copy_from_slice(&self.modulation_system_id.to_be_bytes());
        buf[7..9].copy_from_slice(&self.phy_stream_id.to_be_bytes());
        buf[9] = flag | value;
        let sel_end = 10 + sel.len();
        buf[10..sel_end].copy_from_slice(sel);
        buf[sel_end..len].copy_from_slice(self.trailing);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for IpMacGenericStreamLocationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IP_MAC_GENERIC_STREAM_LOCATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_round_trip_flags_selector() {
        // TSDuck test-022: network 0x1234, DVB-T2, system 0, PHY 0, flags 0.
        let bytes = [TAG, 8, 0x12, 0x34, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        let d = IpMacGenericStreamLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.interactive_network_id, 0x1234);
        assert_eq!(d.modulation_system_type, ModulationSystemType::Dvbt2);
        assert_eq!(d.selector, GenericStreamSelector::Flags(0));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn parse_and_round_trip_byte_selector() {
        // TSDuck test-022: type 0xAB (reserved), system 0x7894, PHY 0x4568,
        // selector bytes 12 34 56 78.
        let bytes = [
            TAG, 12, 0x12, 0x34, 0xAB, 0x78, 0x94, 0x45, 0x68, 0x84, 0x12, 0x34, 0x56, 0x78,
        ];
        let d = IpMacGenericStreamLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.modulation_system_type,
            ModulationSystemType::Reserved(0xAB)
        );
        assert_eq!(d.modulation_system_id, 0x7894);
        assert_eq!(d.phy_stream_id, 0x4568);
        assert_eq!(
            d.selector,
            GenericStreamSelector::Bytes(&[0x12, 0x34, 0x56, 0x78])
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn trailing_round_trip_and_errors() {
        let bytes = [TAG, 9, 0, 1, 4, 0, 2, 0, 3, 0x05, 0xEE];
        let d = IpMacGenericStreamLocationDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.modulation_system_type, ModulationSystemType::Dvbs2x);
        assert_eq!(d.selector, GenericStreamSelector::Flags(5));
        assert_eq!(d.trailing, &[0xEE]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        // selector_length (3) runs past the end.
        assert!(
            IpMacGenericStreamLocationDescriptor::parse(&[TAG, 9, 0, 1, 4, 0, 2, 0, 3, 0x83, 1])
                .is_err()
        );
        assert!(
            IpMacGenericStreamLocationDescriptor::parse(&[TAG, 7, 0, 0, 0, 0, 0, 0, 0]).is_err()
        );
    }

    #[test]
    fn serialize_rejects_over_range_selector() {
        let d = IpMacGenericStreamLocationDescriptor {
            interactive_network_id: 0,
            modulation_system_type: ModulationSystemType::Dvbs2,
            modulation_system_id: 0,
            phy_stream_id: 0,
            selector: GenericStreamSelector::Flags(0x80),
            trailing: &[],
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
        let big = [0u8; 128];
        let d = IpMacGenericStreamLocationDescriptor {
            selector: GenericStreamSelector::Bytes(&big),
            ..d
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }

    #[test]
    fn phy_stream_id_sub_fields_per_table_35() {
        let mk = |t, id| IpMacGenericStreamLocationDescriptor {
            interactive_network_id: 0,
            modulation_system_type: t,
            modulation_system_id: 0,
            phy_stream_id: id,
            selector: GenericStreamSelector::Flags(0),
            trailing: &[],
        };
        assert_eq!(
            mk(ModulationSystemType::Dvbs2, 0x1234).phy_stream(),
            PhyStreamId::Isi(0x1234)
        );
        assert_eq!(
            mk(ModulationSystemType::DvbNgh, 0x1234).phy_stream(),
            PhyStreamId::Plp(0x1234)
        );
        assert_eq!(
            mk(ModulationSystemType::Dvbc2, 0x1234).phy_stream(),
            PhyStreamId::DataSlicePlp {
                data_slice_id: 0x12,
                plp_id: 0x34
            }
        );
        assert_eq!(
            mk(ModulationSystemType::Dvbs2x, 0x1234).phy_stream(),
            PhyStreamId::TimeSliceIsi {
                time_slice: 0x12,
                isi: 0x34
            }
        );
        assert_eq!(
            mk(ModulationSystemType::Reserved(9), 0x1234).phy_stream(),
            PhyStreamId::Unspecified(0x1234)
        );
        for t in [
            ModulationSystemType::Dvbs2,
            ModulationSystemType::Dvbt2,
            ModulationSystemType::Dvbc2,
            ModulationSystemType::DvbNgh,
            ModulationSystemType::Dvbs2x,
            ModulationSystemType::Reserved(7),
        ] {
            for raw in [0u16, 0x00FF, 0xFF00, 0xABCD, 0xFFFF] {
                assert_eq!(PhyStreamId::decode(t, raw).to_u16(), raw);
            }
        }
    }

    #[test]
    fn modulation_system_type_is_lossless() {
        for b in 0..=u8::MAX {
            assert_eq!(ModulationSystemType::from_u8(b).to_u8(), b);
        }
        assert_eq!(ModulationSystemType::Dvbt2.to_string(), "DVB-T2");
    }
}
