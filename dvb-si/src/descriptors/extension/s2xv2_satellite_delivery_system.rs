//! S2Xv2 Satellite Delivery System Descriptor — ETSI EN 300 468 §6.4.6.5.3
//! (tag_extension `0x24`).
//!
//! Body layout: Table 144a (outer shell) + Table 144b
//! (`S2Xv2_satellite_delivery_system_info()`).
//! Mode coding: Table 144c.
//!
//! The descriptor has a fixed 18-byte core followed by several conditional
//! blocks gated on `S2Xv2_mode`, `multiple_input_stream_flag`,
//! `channel_bond`, and sub-flags. The trailing `for` loop of
//! `reserved_zero_future_use` bytes is kept verbatim in `reserved_tail`.
use super::*;
use crate::descriptors::satellite_delivery_system::{Polarization, RollOff};

impl<'a> ExtensionBodyDef<'a> for S2Xv2SatelliteDeliverySystem<'a> {
    const TAG_EXTENSION: u8 = 0x24;
    const NAME: &'static str = "S2XV2_SATELLITE_DELIVERY_SYSTEM";
}

// Fixed-size constants (Table 144b).
/// Fixed core: delivery_system_id(4) + packed_byte_4(1) + packed_byte_5(1)
/// + packed_byte_6(1) + satellite_id(3) + frequency(4) + symbol_rate(4) = 18.
const S2XV2_CORE_LEN: usize = 18;
/// Scrambling sequence block: reserved(6 bits) + scrambling_sequence_index(18 bits) = 3 bytes.
const S2XV2_SCRAMBLING_LEN: usize = 3;
/// Superframe block minimum (no beamhopping): 4 + 3 + 1 = 8 bytes.
const S2XV2_SUPERFRAME_MIN_LEN: usize = 8;
/// Superframe block with beamhopping: 8 + 4 = 12 bytes.
const S2XV2_SUPERFRAME_BH_LEN: usize = 12;

/// S2Xv2 mode — ETSI EN 300 468 Table 144c.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum S2Xv2Mode {
    /// Reserved for future use (0).
    Reserved0,
    /// S2X (1).
    S2X,
    /// S2X + time slicing (2).
    S2XTimeSlicing,
    /// Reserved for future use (3).
    Reserved3,
    /// S2X superframe (Annex E of ETSI EN 302 307-2) (4).
    S2XSuperframe,
    /// S2X superframe + timeslicing (5).
    S2XSuperframeTimeSlicing,
    /// Reserved / future use (6–15).
    Reserved(u8),
}

impl S2Xv2Mode {
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Reserved0,
            1 => Self::S2X,
            2 => Self::S2XTimeSlicing,
            3 => Self::Reserved3,
            4 => Self::S2XSuperframe,
            5 => Self::S2XSuperframeTimeSlicing,
            other => Self::Reserved(other),
        }
    }

    /// Inverse of [`from_u8`](Self::from_u8); `Self::Reserved` emits its stored value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved0 => 0,
            Self::S2X => 1,
            Self::S2XTimeSlicing => 2,
            Self::Reserved3 => 3,
            Self::S2XSuperframe => 4,
            Self::S2XSuperframeTimeSlicing => 5,
            Self::Reserved(v) => v,
        }
    }

    /// Human-readable spec name per Table 144c.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved0 => "reserved for future use",
            Self::S2X => "S2X",
            Self::S2XTimeSlicing => "S2X + time slicing",
            Self::Reserved3 => "reserved for future use",
            Self::S2XSuperframe => "S2X superframe",
            Self::S2XSuperframeTimeSlicing => "S2X superframe + timeslicing",
            Self::Reserved(_) => "reserved",
        }
    }

    /// True for modes where `scrambling_sequence_selector` / scrambling index apply
    /// (Table 144b: modes 1 and 2).
    #[must_use]
    pub fn has_scrambling_selector(self) -> bool {
        matches!(self, Self::S2X | Self::S2XTimeSlicing)
    }

    /// True for modes where `timeslice_number` is present
    /// (Table 144b: modes 2 and 5).
    #[must_use]
    pub fn has_timeslice(self) -> bool {
        matches!(self, Self::S2XTimeSlicing | Self::S2XSuperframeTimeSlicing)
    }

    /// True for modes where the superframe block is present
    /// (Table 144b: modes 4 and 5).
    #[must_use]
    pub fn has_superframe(self) -> bool {
        matches!(self, Self::S2XSuperframe | Self::S2XSuperframeTimeSlicing)
    }
}

/// Superframe block present in S2Xv2_mode 4 and 5 (Table 144b §6.4.6.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct S2Xv2Superframe {
    /// `SOSF_WH_sequence_number` (8 bits).
    pub sosf_wh_sequence_number: u8,
    /// `SFFI_selector` (1 bit) — gates `sffi`.
    pub sffi_selector: bool,
    /// `beam_hopping_time_plan_selector` (1 bit) — gates `beamhopping_time_plan_id`.
    pub beam_hopping_time_plan_selector: bool,
    /// `reference_scrambling_index` (20 bits).
    pub reference_scrambling_index: u32,
    /// `SFFI` (4 bits), present iff `sffi_selector == 1`; stored as raw nibble.
    pub sffi: Option<u8>,
    /// `payload_scrambling_index` (20 bits).
    pub payload_scrambling_index: u32,
    /// `beamhopping_time_plan_id` (32 bits), present iff
    /// `beam_hopping_time_plan_selector == 1`.
    pub beamhopping_time_plan_id: Option<u32>,
    /// `superframe_pilots_WH_sequence_number` (5 bits) — `[7:3]` of the final byte.
    pub superframe_pilots_wh_sequence_number: u8,
    /// `postamble_PLI` (3 bits) — `[2:0]` of the final byte.
    pub postamble_pli: u8,
}

impl S2Xv2Superframe {
    /// Wire length in bytes.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        if self.beam_hopping_time_plan_selector {
            S2XV2_SUPERFRAME_BH_LEN
        } else {
            S2XV2_SUPERFRAME_MIN_LEN
        }
    }
}

/// S2Xv2_satellite_delivery_system body (Tables 144a–144b, §6.4.6.5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct S2Xv2SatelliteDeliverySystem<'a> {
    /// `delivery_system_id` (32 bits).
    pub delivery_system_id: u32,
    /// `S2Xv2_mode` (4 bits) — Table 144c.
    pub s2xv2_mode: S2Xv2Mode,
    /// `multiple_input_stream_flag` (1 bit).
    pub multiple_input_stream_flag: bool,
    /// `roll_off` (3 bits) — EN 300 468 Table 144 (same table as S2X).
    pub roll_off: RollOff,
    /// `NCR_reference` (1 bit) — Table 144c1.
    pub ncr_reference: bool,
    /// `NCR_version` (1 bit).
    pub ncr_version: bool,
    /// `channel_bond` (2 bits).
    pub channel_bond: u8,
    /// `polarization` (2 bits).
    pub polarization: Polarization,
    /// `scrambling_sequence_selector` (1 bit), present (and parsed) iff
    /// `s2xv2_mode ∈ {1, 2}`; `None` means the bit was `reserved_zero_future_use`.
    pub scrambling_sequence_selector: Option<bool>,
    /// `TS_GS_S2X_mode` (2 bits) — Table 143 (raw value).
    pub ts_gs_s2x_mode: u8,
    /// `receiver_profiles` (5 bits) — raw bitmask (Table 141 semantics apply).
    pub receiver_profiles: u8,
    /// `satellite_id` (24 bits).
    pub satellite_id: u32,
    /// `frequency` (32 bits).
    pub frequency: u32,
    /// `symbol_rate` (32 bits).
    pub symbol_rate: u32,
    /// `input_stream_identifier` (8 bits), present iff `multiple_input_stream_flag`.
    pub input_stream_identifier: Option<u8>,
    /// `scrambling_sequence_index` (18 bits), present iff
    /// `s2xv2_mode ∈ {1, 2}` **and** `scrambling_sequence_selector == 1`.
    pub scrambling_sequence_index: Option<u32>,
    /// `timeslice_number` (8 bits), present iff `s2xv2_mode ∈ {2, 5}`.
    pub timeslice_number: Option<u8>,
    /// Secondary delivery system IDs from the `channel_bond` loop (one per
    /// entry, 32 bits each), present iff `channel_bond == 1`.
    pub secondary_delivery_system_ids: Vec<u32>,
    /// Superframe block, present iff `s2xv2_mode ∈ {4, 5}`.
    pub superframe: Option<S2Xv2Superframe>,
    /// Trailing `reserved_zero_future_use` bytes (verbatim from the closing `for` loop).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub reserved_tail: &'a [u8],
}

impl<'a> Parse<'a> for S2Xv2SatelliteDeliverySystem<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < S2XV2_CORE_LEN {
            return Err(Error::BufferTooShort {
                need: S2XV2_CORE_LEN,
                have: sel.len(),
                what: "S2Xv2 body",
            });
        }
        // Bytes 0–3: delivery_system_id.
        let delivery_system_id = u32::from_be_bytes([sel[0], sel[1], sel[2], sel[3]]);
        // Byte 4: S2Xv2_mode(4) | multiple_input_stream_flag(1) | roll_off(3).
        let b4 = sel[4];
        let s2xv2_mode = S2Xv2Mode::from_u8(b4 >> 4);
        let multiple_input_stream_flag = (b4 & 0x08) != 0;
        let roll_off = RollOff::from_u8(b4 & 0x07);
        // Byte 5: reserved(2) | NCR_reference(1) | NCR_version(1) | channel_bond(2) | polarization(2).
        let b5 = sel[5];
        let ncr_reference = (b5 & 0x20) != 0;
        let ncr_version = (b5 & 0x10) != 0;
        let channel_bond = (b5 >> 2) & 0x03;
        let polarization = Polarization::from_u8(b5 & 0x03);
        // Byte 6: scrambling_or_reserved(1) | TS_GS_S2X_mode(2) | receiver_profiles(5).
        let b6 = sel[6];
        let raw_scrambling_bit = (b6 >> 7) & 0x01;
        let ts_gs_s2x_mode = (b6 >> 5) & 0x03;
        let receiver_profiles = b6 & 0x1F;
        let scrambling_sequence_selector = if s2xv2_mode.has_scrambling_selector() {
            Some(raw_scrambling_bit != 0)
        } else {
            None
        };
        // Bytes 7–9: satellite_id (24 bits, big-endian).
        let satellite_id = (u32::from(sel[7]) << 16) | (u32::from(sel[8]) << 8) | u32::from(sel[9]);
        // Bytes 10–13: frequency (32 bits).
        let frequency = u32::from_be_bytes([sel[10], sel[11], sel[12], sel[13]]);
        // Bytes 14–17: symbol_rate (32 bits).
        let symbol_rate = u32::from_be_bytes([sel[14], sel[15], sel[16], sel[17]]);
        let mut pos = S2XV2_CORE_LEN;

        // Conditional: input_stream_identifier.
        let input_stream_identifier = if multiple_input_stream_flag {
            if sel.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "S2Xv2 body (input_stream_identifier)",
                });
            }
            let isi = sel[pos];
            pos += 1;
            Some(isi)
        } else {
            None
        };

        // Conditional: scrambling_sequence_index (mode 1 or 2, selector == 1).
        let scrambling_sequence_index = if scrambling_sequence_selector == Some(true) {
            if sel.len() < pos + S2XV2_SCRAMBLING_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + S2XV2_SCRAMBLING_LEN,
                    have: sel.len(),
                    what: "S2Xv2 body (scrambling_sequence_index)",
                });
            }
            // reserved(6) | scrambling_sequence_index(18).
            let idx = (u32::from(sel[pos] & 0x03) << 16)
                | (u32::from(sel[pos + 1]) << 8)
                | u32::from(sel[pos + 2]);
            pos += S2XV2_SCRAMBLING_LEN;
            Some(idx)
        } else {
            None
        };

        // Conditional: timeslice_number (mode 2 or 5).
        let timeslice_number = if s2xv2_mode.has_timeslice() {
            if sel.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "S2Xv2 body (timeslice_number)",
                });
            }
            let ts = sel[pos];
            pos += 1;
            Some(ts)
        } else {
            None
        };

        // Conditional: channel_bond loop.
        let secondary_delivery_system_ids = if channel_bond == 1 {
            if sel.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "S2Xv2 body (channel_bond header)",
                });
            }
            let bond_byte = sel[pos];
            pos += 1;
            // reserved(7) | num_channel_bonds_minus_one(1).
            let n = (bond_byte & 0x01) as usize + 1;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                if sel.len() < pos + 4 {
                    return Err(Error::BufferTooShort {
                        need: pos + 4,
                        have: sel.len(),
                        what: "S2Xv2 body (secondary_delivery_system_id)",
                    });
                }
                let id = u32::from_be_bytes([sel[pos], sel[pos + 1], sel[pos + 2], sel[pos + 3]]);
                ids.push(id);
                pos += 4;
            }
            ids
        } else {
            Vec::new()
        };

        // Conditional: superframe block (mode 4 or 5).
        let superframe = if s2xv2_mode.has_superframe() {
            if sel.len() < pos + S2XV2_SUPERFRAME_MIN_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + S2XV2_SUPERFRAME_MIN_LEN,
                    have: sel.len(),
                    what: "S2Xv2 body (superframe)",
                });
            }
            let sosf_wh_sequence_number = sel[pos];
            // pos+1: SFFI_selector(1) | BH_selector(1) | reserved(2) | ref_scram_hi(4).
            let b1 = sel[pos + 1];
            let sffi_selector = (b1 & 0x80) != 0;
            let beam_hopping_time_plan_selector = (b1 & 0x40) != 0;
            let ref_scram_hi = u32::from(b1 & 0x0F);
            let reference_scrambling_index =
                (ref_scram_hi << 16) | (u32::from(sel[pos + 2]) << 8) | u32::from(sel[pos + 3]);
            // pos+4: SFFI_or_reserved(4) | psi_hi(4).
            let b4s = sel[pos + 4];
            let sffi = if sffi_selector { Some(b4s >> 4) } else { None };
            let psi_hi = u32::from(b4s & 0x0F);
            let payload_scrambling_index =
                (psi_hi << 16) | (u32::from(sel[pos + 5]) << 8) | u32::from(sel[pos + 6]);
            pos += 7;
            // Conditional: beamhopping_time_plan_id.
            let beamhopping_time_plan_id = if beam_hopping_time_plan_selector {
                if sel.len() < pos + 4 {
                    return Err(Error::BufferTooShort {
                        need: pos + 4,
                        have: sel.len(),
                        what: "S2Xv2 body (beamhopping_time_plan_id)",
                    });
                }
                let id = u32::from_be_bytes([sel[pos], sel[pos + 1], sel[pos + 2], sel[pos + 3]]);
                pos += 4;
                Some(id)
            } else {
                None
            };
            // Final superframe byte: pilots(5) | postamble_PLI(3).
            if sel.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "S2Xv2 body (superframe final byte)",
                });
            }
            let last = sel[pos];
            let superframe_pilots_wh_sequence_number = (last >> 3) & 0x1F;
            let postamble_pli = last & 0x07;
            pos += 1;
            Some(S2Xv2Superframe {
                sosf_wh_sequence_number,
                sffi_selector,
                beam_hopping_time_plan_selector,
                reference_scrambling_index,
                sffi,
                payload_scrambling_index,
                beamhopping_time_plan_id,
                superframe_pilots_wh_sequence_number,
                postamble_pli,
            })
        } else {
            None
        };

        Ok(S2Xv2SatelliteDeliverySystem {
            delivery_system_id,
            s2xv2_mode,
            multiple_input_stream_flag,
            roll_off,
            ncr_reference,
            ncr_version,
            channel_bond,
            polarization,
            scrambling_sequence_selector,
            ts_gs_s2x_mode,
            receiver_profiles,
            satellite_id,
            frequency,
            symbol_rate,
            input_stream_identifier,
            scrambling_sequence_index,
            timeslice_number,
            secondary_delivery_system_ids,
            superframe,
            reserved_tail: &sel[pos..],
        })
    }
}

impl Serialize for S2Xv2SatelliteDeliverySystem<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        S2XV2_CORE_LEN
            + usize::from(self.input_stream_identifier.is_some())
            + if self.scrambling_sequence_selector == Some(true) {
                S2XV2_SCRAMBLING_LEN
            } else {
                0
            }
            + usize::from(self.timeslice_number.is_some())
            + if self.channel_bond == 1 {
                1 + self.secondary_delivery_system_ids.len() * 4
            } else {
                0
            }
            + self.superframe.as_ref().map_or(0, |sf| sf.serialized_len())
            + self.reserved_tail.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // Bytes 0–3: delivery_system_id.
        buf[0..4].copy_from_slice(&self.delivery_system_id.to_be_bytes());
        // Byte 4: S2Xv2_mode(4) | multiple_input_stream_flag(1) | roll_off(3).
        buf[4] = ((self.s2xv2_mode.to_u8() & 0x0F) << 4)
            | (u8::from(self.multiple_input_stream_flag) << 3)
            | (self.roll_off.to_u8() & 0x07);
        // Byte 5: reserved(2)=0 | NCR_reference(1) | NCR_version(1) | channel_bond(2) | polarization(2).
        buf[5] = (u8::from(self.ncr_reference) << 5)
            | (u8::from(self.ncr_version) << 4)
            | ((self.channel_bond & 0x03) << 2)
            | (self.polarization.to_u8() & 0x03);
        // Byte 6: scrambling_or_reserved(1) | TS_GS_S2X_mode(2) | receiver_profiles(5).
        let scram_bit: u8 = match self.scrambling_sequence_selector {
            Some(true) => 1,
            _ => 0,
        };
        buf[6] = (scram_bit << 7)
            | ((self.ts_gs_s2x_mode & 0x03) << 5)
            | (self.receiver_profiles & 0x1F);
        // Bytes 7–9: satellite_id (24 bits).
        buf[7] = (self.satellite_id >> 16) as u8;
        buf[8] = (self.satellite_id >> 8) as u8;
        buf[9] = self.satellite_id as u8;
        // Bytes 10–13: frequency.
        buf[10..14].copy_from_slice(&self.frequency.to_be_bytes());
        // Bytes 14–17: symbol_rate.
        buf[14..18].copy_from_slice(&self.symbol_rate.to_be_bytes());
        let mut p = S2XV2_CORE_LEN;
        // Conditional: input_stream_identifier.
        if let Some(isi) = self.input_stream_identifier {
            buf[p] = isi;
            p += 1;
        }
        // Conditional: scrambling block.
        if self.scrambling_sequence_selector == Some(true) {
            let idx = self.scrambling_sequence_index.unwrap_or(0) & 0x0003_FFFF;
            buf[p] = (idx >> 16) as u8 & 0x03;
            buf[p + 1] = (idx >> 8) as u8;
            buf[p + 2] = idx as u8;
            p += S2XV2_SCRAMBLING_LEN;
        }
        // Conditional: timeslice_number.
        if let Some(ts) = self.timeslice_number {
            buf[p] = ts;
            p += 1;
        }
        // Conditional: channel bond loop.
        if self.channel_bond == 1 {
            // reserved(7)=0 | num_channel_bonds_minus_one(1).
            let n = self.secondary_delivery_system_ids.len().saturating_sub(1) as u8 & 0x01;
            buf[p] = n;
            p += 1;
            for &id in &self.secondary_delivery_system_ids {
                buf[p..p + 4].copy_from_slice(&id.to_be_bytes());
                p += 4;
            }
        }
        // Conditional: superframe block.
        if let Some(sf) = &self.superframe {
            buf[p] = sf.sosf_wh_sequence_number;
            let ref_hi = (sf.reference_scrambling_index >> 16) as u8 & 0x0F;
            buf[p + 1] = (u8::from(sf.sffi_selector) << 7)
                | (u8::from(sf.beam_hopping_time_plan_selector) << 6)
                | ref_hi;
            buf[p + 2] = (sf.reference_scrambling_index >> 8) as u8;
            buf[p + 3] = sf.reference_scrambling_index as u8;
            let sffi_nibble: u8 = sf.sffi.unwrap_or(0) & 0x0F;
            let psi_hi = (sf.payload_scrambling_index >> 16) as u8 & 0x0F;
            buf[p + 4] = (sffi_nibble << 4) | psi_hi;
            buf[p + 5] = (sf.payload_scrambling_index >> 8) as u8;
            buf[p + 6] = sf.payload_scrambling_index as u8;
            p += 7;
            if let Some(bh_id) = sf.beamhopping_time_plan_id {
                buf[p..p + 4].copy_from_slice(&bh_id.to_be_bytes());
                p += 4;
            }
            buf[p] =
                ((sf.superframe_pilots_wh_sequence_number & 0x1F) << 3) | (sf.postamble_pli & 0x07);
            p += 1;
        }
        // Reserved tail.
        buf[p..p + self.reserved_tail.len()].copy_from_slice(self.reserved_tail);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    /// Build a minimal valid 18-byte selector for S2Xv2 mode 1 (no optional fields).
    fn minimal_mode1_sel() -> Vec<u8> {
        let mut sel = Vec::new();
        // delivery_system_id = 0x0102_0304
        sel.extend_from_slice(&0x0102_0304u32.to_be_bytes());
        // b4: S2Xv2_mode=1(0x10), mis=0, roll_off=2(0x02) → 0x12
        sel.push(0x12);
        // b5: reserved(2)=0, NCR_reference=1(0x20), NCR_version=0,
        //     channel_bond=0, polarization=1(LinearVertical) → 0x21
        sel.push(0x21);
        // b6: scram_sel=0(mode 1 → uses actual bit), ts_gs=3(0x60), recv_prof=0x07 → 0x67
        sel.push(0x67);
        // satellite_id = 0x010203
        sel.extend_from_slice(&[0x01, 0x02, 0x03]);
        // frequency = 0x1234_5678
        sel.extend_from_slice(&0x1234_5678u32.to_be_bytes());
        // symbol_rate = 0xABCD_EF01
        sel.extend_from_slice(&0xABCD_EF01u32.to_be_bytes());
        sel
    }

    #[test]
    fn parse_s2xv2_mode1_no_scrambling_round_trip() {
        // S2Xv2 mode 1, no scrambling, no MIS, no tail.
        let sel = minimal_mode1_sel();
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::S2Xv2SatelliteDeliverySystem));
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.delivery_system_id, 0x0102_0304);
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::S2X);
                assert!(!b.multiple_input_stream_flag);
                assert_eq!(b.roll_off, RollOff::Alpha020);
                assert!(b.ncr_reference);
                assert!(!b.ncr_version);
                assert_eq!(b.channel_bond, 0);
                assert_eq!(b.polarization, Polarization::LinearVertical);
                assert_eq!(b.scrambling_sequence_selector, Some(false));
                assert_eq!(b.ts_gs_s2x_mode, 3);
                assert_eq!(b.receiver_profiles, 0x07);
                assert_eq!(b.satellite_id, 0x010203);
                assert_eq!(b.frequency, 0x1234_5678);
                assert_eq!(b.symbol_rate, 0xABCD_EF01);
                assert!(b.input_stream_identifier.is_none());
                assert!(b.scrambling_sequence_index.is_none());
                assert!(b.timeslice_number.is_none());
                assert!(b.secondary_delivery_system_ids.is_empty());
                assert!(b.superframe.is_none());
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_mode1_scrambling_and_mis_round_trip() {
        // S2Xv2 mode 1, scrambling_selector=1, MIS=1, ISI=0x42,
        // scrambling_sequence_index=0x1ABCD.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes()); // delivery_system_id
                                                              // b4: mode=1(0x10), mis=1(0x08), roll_off=1(0x01) → 0x19
        sel.push(0x19);
        // b5: NCR_reference=0, NCR_version=1(0x10), channel_bond=0, polarization=3 → 0x13
        sel.push(0x13);
        // b6: scram_sel=1(0x80), ts_gs=0, recv_prof=0x1F → 0x9F
        sel.push(0x9F);
        sel.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // satellite_id
        sel.extend_from_slice(&0x0000_0001u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0x0000_0002u32.to_be_bytes()); // symbol_rate
        sel.push(0x42); // input_stream_identifier (MIS=1)
                        // scrambling block: reserved(6)=0 | scrambling_sequence_index(18) = 0x1ABCD
                        // 0x1ABCD >> 16 = 0x01 (& 0x03), mid = 0xAB, lo = 0xCD
        sel.extend_from_slice(&[0x01, 0xAB, 0xCD]);
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.delivery_system_id, 0xDEAD_BEEF);
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::S2X);
                assert!(b.multiple_input_stream_flag);
                // b4 = 0x19: mode=1, mis=1, roll_off=1 → Alpha025
                assert_eq!(b.roll_off, RollOff::Alpha025);
                assert!(!b.ncr_reference);
                assert!(b.ncr_version);
                assert_eq!(b.channel_bond, 0);
                assert_eq!(b.polarization, Polarization::CircularRight);
                assert_eq!(b.scrambling_sequence_selector, Some(true));
                assert_eq!(b.ts_gs_s2x_mode, 0);
                assert_eq!(b.receiver_profiles, 0x1F);
                assert_eq!(b.input_stream_identifier, Some(0x42));
                assert_eq!(b.scrambling_sequence_index, Some(0x1ABCD));
                assert!(b.timeslice_number.is_none());
                assert!(b.secondary_delivery_system_ids.is_empty());
                assert!(b.superframe.is_none());
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_mode2_timeslice_round_trip() {
        // S2Xv2 mode 2 (S2X + timeslicing): timeslice_number present, no MIS.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0x0000_0001u32.to_be_bytes());
        // b4: mode=2(0x20), mis=0, roll_off=0 → 0x20
        sel.push(0x20);
        sel.push(0x00); // b5
                        // b6: scram_sel=0, ts_gs=0, recv_prof=1 → 0x01
        sel.push(0x01);
        sel.extend_from_slice(&[0, 0, 0]); // satellite_id
        sel.extend_from_slice(&[0, 0, 0, 0]); // frequency
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
        sel.push(0x07); // timeslice_number (mode 2 → has_timeslice)
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::S2XTimeSlicing);
                assert_eq!(b.timeslice_number, Some(0x07));
                assert!(b.scrambling_sequence_index.is_none());
                assert!(b.secondary_delivery_system_ids.is_empty());
                assert!(b.superframe.is_none());
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_channel_bond_round_trip() {
        // S2Xv2 mode 0 (reserved), channel_bond=1, two secondary IDs.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // delivery_system_id
                                                              // b4: mode=0, mis=0, roll_off=0 → 0x00
        sel.push(0x00);
        // b5: channel_bond=1(0x04), polarization=0 → 0x04
        sel.push(0x04);
        // b6: all zero
        sel.push(0x00);
        sel.extend_from_slice(&[0, 0, 0]); // satellite_id
        sel.extend_from_slice(&[0, 0, 0, 0]); // frequency
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
                                              // channel_bond header: reserved(7)=0 | num_channel_bonds_minus_one(1)=1 → N=2
        sel.push(0x01);
        sel.extend_from_slice(&0x1111_1111u32.to_be_bytes()); // id[0]
        sel.extend_from_slice(&0x2222_2222u32.to_be_bytes()); // id[1]
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.channel_bond, 1);
                assert_eq!(
                    b.secondary_delivery_system_ids,
                    vec![0x1111_1111, 0x2222_2222]
                );
                assert!(b.superframe.is_none());
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_superframe_no_beamhopping_round_trip() {
        // S2Xv2 mode 4 (superframe), SFFI_selector=1, no beamhopping.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0x0000_0002u32.to_be_bytes()); // delivery_system_id
                                                              // b4: mode=4(0x40), mis=0, roll_off=0 → 0x40
        sel.push(0x40);
        sel.push(0x00); // b5
                        // b6: no scrambling_selector (mode 4), ts_gs=1(0x20), recv_prof=3(0x03) → 0x23
        sel.push(0x23);
        sel.extend_from_slice(&[0, 0, 0]); // satellite_id
        sel.extend_from_slice(&[0, 0, 0, 0]); // frequency
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
                                              // Superframe block:
        sel.push(0xAB); // sosf_wh_sequence_number
                        // b1: SFFI_selector=1(0x80), BH=0, reserved=0, ref_scram_hi=0x05 → 0x85
        sel.push(0x85);
        sel.push(0x12); // ref_scram_mid
        sel.push(0x34); // ref_scram_lo → ref_scram = 0x051234
                        // b4: SFFI=0x9(0x90), psi_hi=0x06(0x06) → 0x96
        sel.push(0x96);
        sel.push(0x78); // psi_mid
        sel.push(0x90); // psi_lo → psi = 0x067890
                        // no beamhopping_time_plan_id
                        // final: pilots=0x1A(5b)=0b11010, postamble=0x03(3b)=0b011 → 0b11010_011 = 0xD3
        sel.push(0xD3);
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::S2XSuperframe);
                assert!(b.superframe.is_some());
                let sf = b.superframe.as_ref().unwrap();
                assert_eq!(sf.sosf_wh_sequence_number, 0xAB);
                assert!(sf.sffi_selector);
                assert!(!sf.beam_hopping_time_plan_selector);
                assert_eq!(sf.reference_scrambling_index, 0x05_1234);
                assert_eq!(sf.sffi, Some(0x9));
                assert_eq!(sf.payload_scrambling_index, 0x06_7890);
                assert!(sf.beamhopping_time_plan_id.is_none());
                assert_eq!(sf.superframe_pilots_wh_sequence_number, 0x1A);
                assert_eq!(sf.postamble_pli, 0x03);
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_superframe_with_beamhopping_round_trip() {
        // S2Xv2 mode 5 (superframe + timeslicing), SFFI=0, beam_hopping=1.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0x0000_0003u32.to_be_bytes()); // delivery_system_id
                                                              // b4: mode=5(0x50), mis=0, roll_off=0 → 0x50
        sel.push(0x50);
        sel.push(0x00); // b5
                        // b6: mode 5 has no scrambling_selector, so scram bit is reserved→0; ts_gs=0, recv_prof=0
        sel.push(0x00);
        sel.extend_from_slice(&[0, 0, 0]); // satellite_id
        sel.extend_from_slice(&[0, 0, 0, 0]); // frequency
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
                                              // timeslice_number (mode 5 → has_timeslice)
        sel.push(0x0F);
        // Superframe block:
        sel.push(0x01); // sosf_wh_sequence_number
                        // b1: SFFI=0, BH=1(0x40), reserved=0, ref_scram_hi=0x0A → 0x4A
        sel.push(0x4A);
        sel.push(0xBC); // ref_scram_mid
        sel.push(0xDE); // ref_scram_lo → ref_scram = 0x0ABCDE
                        // b4: sffi_or_reserved(4)=0, psi_hi=0x00 → 0x00
        sel.push(0x00);
        sel.push(0x00); // psi_mid
        sel.push(0xFF); // psi_lo → psi = 0x0000FF
                        // beamhopping_time_plan_id = 0x1234_5678
        sel.extend_from_slice(&0x1234_5678u32.to_be_bytes());
        // final: pilots=0x05(5b)=0b00101, postamble=0x07(3b)=0b111 → 0b00101_111 = 0x2F
        sel.push(0x2F);
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::S2XSuperframeTimeSlicing);
                assert_eq!(b.timeslice_number, Some(0x0F));
                let sf = b.superframe.as_ref().unwrap();
                assert!(!sf.sffi_selector);
                assert!(sf.beam_hopping_time_plan_selector);
                assert_eq!(sf.reference_scrambling_index, 0x0A_BCDE);
                assert!(sf.sffi.is_none());
                assert_eq!(sf.payload_scrambling_index, 0x0000FF);
                assert_eq!(sf.beamhopping_time_plan_id, Some(0x1234_5678));
                assert_eq!(sf.superframe_pilots_wh_sequence_number, 0x05);
                assert_eq!(sf.postamble_pli, 0x07);
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_reserved_tail_preserved() {
        // Mode 0 (reserved): no conditionals fire, tail preserved verbatim.
        let mut sel = Vec::new();
        sel.extend_from_slice(&0x0000_0000u32.to_be_bytes());
        // b4: mode=0, mis=0, roll_off=2 → 0x02
        sel.push(0x02);
        sel.push(0x00); // b5
        sel.push(0x00); // b6
        sel.extend_from_slice(&[0, 0, 0]);
        sel.extend_from_slice(&[0, 0, 0, 0]);
        sel.extend_from_slice(&[0, 0, 0, 0]);
        sel.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // reserved_tail
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2Xv2SatelliteDeliverySystem(b) => {
                assert_eq!(b.s2xv2_mode, S2Xv2Mode::Reserved0);
                assert_eq!(b.reserved_tail, &[0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected S2Xv2, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2xv2_rejects_truncated() {
        // Only 10 bytes — less than S2XV2_CORE_LEN (18).
        let sel = vec![0u8; 10];
        let bytes = wrap(0x24, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn s2xv2_mode_roundtrip_all_values() {
        for b in 0..=0x0Fu8 {
            assert_eq!(S2Xv2Mode::from_u8(b).to_u8(), b);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_s2xv2() {
        let sel = minimal_mode1_sel();
        let bytes = wrap(0x24, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        let json = serde_json::to_string(&d).unwrap();
        // tag_extension = 0x24 = 36
        assert!(json.contains("\"tag_extension\":36"));
        assert!(json.contains("\"s2Xv2SatelliteDeliverySystem\""));
    }
}
