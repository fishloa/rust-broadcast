//! T2 Delivery System Descriptor — ETSI EN 300 468 §6.4.6.3 (tag_extension 0x04).
use super::*;

impl<'a> ExtensionBodyDef<'a> for T2DeliverySystem {
    const TAG_EXTENSION: u8 = 0x04;
    const NAME: &'static str = "T2_DELIVERY_SYSTEM";
}

// ---------------------------------------------------------------------------
//  T2-specific enums (Tables 134-137)
// ---------------------------------------------------------------------------

/// SISO/MISO mode — ETSI EN 300 468 Table 134.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum T2SisoMiso {
    /// Single Input, Single Output (SISO).
    Siso,
    /// Multiple Input, Single Output (MISO).
    Miso,
    /// Reserved / future use.
    Reserved(u8),
}

impl T2SisoMiso {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => T2SisoMiso::Siso,
            1 => T2SisoMiso::Miso,
            other => T2SisoMiso::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            T2SisoMiso::Siso => 0,
            T2SisoMiso::Miso => 1,
            T2SisoMiso::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            T2SisoMiso::Siso => "SISO",
            T2SisoMiso::Miso => "MISO",
            T2SisoMiso::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(T2SisoMiso, Reserved);

/// Bandwidth — ETSI EN 300 468 Table 135.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum T2Bandwidth {
    /// 8 MHz.
    Mhz8,
    /// 7 MHz.
    Mhz7,
    /// 6 MHz.
    Mhz6,
    /// 5 MHz.
    Mhz5,
    /// 10 MHz.
    Mhz10,
    /// 1.712 MHz.
    Mhz1_712,
    /// Reserved / future use.
    Reserved(u8),
}

impl T2Bandwidth {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => T2Bandwidth::Mhz8,
            1 => T2Bandwidth::Mhz7,
            2 => T2Bandwidth::Mhz6,
            3 => T2Bandwidth::Mhz5,
            4 => T2Bandwidth::Mhz10,
            5 => T2Bandwidth::Mhz1_712,
            other => T2Bandwidth::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            T2Bandwidth::Mhz8 => 0,
            T2Bandwidth::Mhz7 => 1,
            T2Bandwidth::Mhz6 => 2,
            T2Bandwidth::Mhz5 => 3,
            T2Bandwidth::Mhz10 => 4,
            T2Bandwidth::Mhz1_712 => 5,
            T2Bandwidth::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            T2Bandwidth::Mhz8 => "8 MHz",
            T2Bandwidth::Mhz7 => "7 MHz",
            T2Bandwidth::Mhz6 => "6 MHz",
            T2Bandwidth::Mhz5 => "5 MHz",
            T2Bandwidth::Mhz10 => "10 MHz",
            T2Bandwidth::Mhz1_712 => "1.712 MHz",
            T2Bandwidth::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(T2Bandwidth, Reserved);

/// Guard interval — ETSI EN 300 468 Table 136.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum T2GuardInterval {
    /// 1/32.
    G1_32,
    /// 1/16.
    G1_16,
    /// 1/8.
    G1_8,
    /// 1/4.
    G1_4,
    /// 1/128.
    G1_128,
    /// 19/128.
    G19_128,
    /// 19/256.
    G19_256,
    /// Reserved / future use.
    Reserved(u8),
}

impl T2GuardInterval {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => T2GuardInterval::G1_32,
            1 => T2GuardInterval::G1_16,
            2 => T2GuardInterval::G1_8,
            3 => T2GuardInterval::G1_4,
            4 => T2GuardInterval::G1_128,
            5 => T2GuardInterval::G19_128,
            6 => T2GuardInterval::G19_256,
            other => T2GuardInterval::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            T2GuardInterval::G1_32 => 0,
            T2GuardInterval::G1_16 => 1,
            T2GuardInterval::G1_8 => 2,
            T2GuardInterval::G1_4 => 3,
            T2GuardInterval::G1_128 => 4,
            T2GuardInterval::G19_128 => 5,
            T2GuardInterval::G19_256 => 6,
            T2GuardInterval::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            T2GuardInterval::G1_32 => "1/32",
            T2GuardInterval::G1_16 => "1/16",
            T2GuardInterval::G1_8 => "1/8",
            T2GuardInterval::G1_4 => "1/4",
            T2GuardInterval::G1_128 => "1/128",
            T2GuardInterval::G19_128 => "19/128",
            T2GuardInterval::G19_256 => "19/256",
            T2GuardInterval::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(T2GuardInterval, Reserved);

/// Transmission mode — ETSI EN 300 468 Table 137.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum T2TransmissionMode {
    /// 2k mode.
    Mode2k,
    /// 8k mode.
    Mode8k,
    /// 4k mode.
    Mode4k,
    /// 1k mode.
    Mode1k,
    /// 16k mode.
    Mode16k,
    /// 32k mode.
    Mode32k,
    /// Reserved / future use.
    Reserved(u8),
}

impl T2TransmissionMode {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => T2TransmissionMode::Mode2k,
            1 => T2TransmissionMode::Mode8k,
            2 => T2TransmissionMode::Mode4k,
            3 => T2TransmissionMode::Mode1k,
            4 => T2TransmissionMode::Mode16k,
            5 => T2TransmissionMode::Mode32k,
            other => T2TransmissionMode::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            T2TransmissionMode::Mode2k => 0,
            T2TransmissionMode::Mode8k => 1,
            T2TransmissionMode::Mode4k => 2,
            T2TransmissionMode::Mode1k => 3,
            T2TransmissionMode::Mode16k => 4,
            T2TransmissionMode::Mode32k => 5,
            T2TransmissionMode::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            T2TransmissionMode::Mode2k => "2k",
            T2TransmissionMode::Mode8k => "8k",
            T2TransmissionMode::Mode4k => "4k",
            T2TransmissionMode::Mode1k => "1k",
            T2TransmissionMode::Mode16k => "16k",
            T2TransmissionMode::Mode32k => "32k",
            T2TransmissionMode::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(T2TransmissionMode, Reserved);

// ---------------------------------------------------------------------------
//  Structs
// ---------------------------------------------------------------------------

/// One T2 cell (Table 133 inner `for`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct T2Cell {
    /// cell_id(16).
    pub cell_id: u16,
    /// centre_frequency list. When tfs_flag, the length-prefixed loop;
    /// otherwise exactly one frequency.
    pub centre_frequencies: Vec<u32>,
    /// subcell entries.
    pub subcells: Vec<T2Subcell>,
}

impl T2Cell {
    /// Decode all centre_frequencies to Hz (×10 Hz field resolution).
    #[must_use]
    pub fn centre_frequencies_hz(&self) -> Vec<u64> {
        self.centre_frequencies
            .iter()
            .map(|&f| u64::from(f) * 10)
            .collect()
    }
}

/// One T2 subcell (Table 133 innermost `for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct T2Subcell {
    /// cell_id_extension(8).
    pub cell_id_extension: u8,
    /// transposer_frequency(32) — ×10 Hz units.
    pub transposer_frequency: u32,
}

impl T2Subcell {
    /// Decode transposer_frequency to Hz (×10 Hz field resolution).
    #[must_use]
    pub fn transposer_frequency_hz(&self) -> u64 {
        u64::from(self.transposer_frequency) * 10
    }
}

/// T2_delivery_system body (Table 133). The cell loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct T2DeliverySystem {
    /// PLP identifier.
    pub plp_id: u8,
    /// T2 system identifier.
    pub t2_system_id: u16,
    /// SISO_MISO, present iff `descriptor_length > 4` (flags block present).
    pub siso_miso: Option<T2SisoMiso>,
    /// bandwidth, present with `siso_miso`.
    pub bandwidth: Option<T2Bandwidth>,
    /// guard_interval, present with `siso_miso`.
    pub guard_interval: Option<T2GuardInterval>,
    /// transmission_mode, present with `siso_miso`.
    pub transmission_mode: Option<T2TransmissionMode>,
    /// other_frequency_flag, present with `siso_miso`.
    pub other_frequency_flag: Option<bool>,
    /// tfs_flag, present with `siso_miso`.
    pub tfs_flag: Option<bool>,
    /// Cell loop entries (present only when flags block is present).
    pub cells: Vec<T2Cell>,
}

impl<'a> Parse<'a> for T2DeliverySystem {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < T2_FIXED_PREFIX_LEN {
            return Err(Error::BufferTooShort {
                need: T2_FIXED_PREFIX_LEN,
                have: sel.len(),
                what: "T2_delivery_system body",
            });
        }
        let plp_id = sel[0];
        let t2_system_id = u16::from_be_bytes([sel[1], sel[2]]);
        let mut pos = T2_FIXED_PREFIX_LEN;
        let (
            siso_miso,
            bandwidth,
            guard_interval,
            transmission_mode,
            other_frequency_flag,
            tfs_flag,
        ) = if sel.len() > T2_FIXED_PREFIX_LEN {
            if sel.len() < T2_FIXED_PREFIX_LEN + T2_FLAGS_BLOCK_LEN {
                return Err(Error::BufferTooShort {
                    need: T2_FIXED_PREFIX_LEN + T2_FLAGS_BLOCK_LEN,
                    have: sel.len(),
                    what: "T2_delivery_system body",
                });
            }
            let b0 = sel[pos];
            let b1 = sel[pos + 1];
            pos += T2_FLAGS_BLOCK_LEN;
            (
                Some(T2SisoMiso::from_u8(b0 >> 6)),
                Some(T2Bandwidth::from_u8((b0 >> 2) & 0x0F)),
                Some(T2GuardInterval::from_u8(b1 >> 5)),
                Some(T2TransmissionMode::from_u8((b1 >> 2) & 0x07)),
                Some((b1 & 0x02) != 0),
                Some((b1 & 0x01) != 0),
            )
        } else {
            (None, None, None, None, None, None)
        };
        let cells = if siso_miso.is_some() {
            let tfs = tfs_flag.unwrap();
            let mut cells = Vec::new();
            while pos < sel.len() {
                if pos + 2 > sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 2,
                        have: sel.len(),
                        what: "T2_delivery_system body",
                    });
                }
                let cell_id = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
                pos += 2;
                let centre_frequencies = if tfs {
                    if pos >= sel.len() {
                        return Err(Error::BufferTooShort {
                            need: pos + 1,
                            have: sel.len(),
                            what: "T2_delivery_system body",
                        });
                    }
                    let freq_loop_len = sel[pos] as usize;
                    pos += 1;
                    if freq_loop_len % 4 != 0 {
                        return Err(invalid(
                            "T2_delivery_system: frequency_loop_length not a multiple of 4",
                        ));
                    }
                    if pos + freq_loop_len > sel.len() {
                        return Err(Error::BufferTooShort {
                            need: pos + freq_loop_len,
                            have: sel.len(),
                            what: "T2_delivery_system body",
                        });
                    }
                    let end = pos + freq_loop_len;
                    let mut freqs = Vec::with_capacity(freq_loop_len / 4);
                    while pos < end {
                        freqs.push(u32::from_be_bytes([
                            sel[pos],
                            sel[pos + 1],
                            sel[pos + 2],
                            sel[pos + 3],
                        ]));
                        pos += 4;
                    }
                    freqs
                } else {
                    if pos + 4 > sel.len() {
                        return Err(Error::BufferTooShort {
                            need: pos + 4,
                            have: sel.len(),
                            what: "T2_delivery_system body",
                        });
                    }
                    let freq =
                        u32::from_be_bytes([sel[pos], sel[pos + 1], sel[pos + 2], sel[pos + 3]]);
                    pos += 4;
                    vec![freq]
                };
                if pos >= sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 1,
                        have: sel.len(),
                        what: "T2_delivery_system body",
                    });
                }
                let subcell_loop_len = sel[pos] as usize;
                pos += 1;
                if subcell_loop_len % 5 != 0 {
                    return Err(invalid(
                        "T2_delivery_system: subcell_info_loop_length not a multiple of 5",
                    ));
                }
                if pos + subcell_loop_len > sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + subcell_loop_len,
                        have: sel.len(),
                        what: "T2_delivery_system body",
                    });
                }
                let end = pos + subcell_loop_len;
                let mut subcells = Vec::with_capacity(subcell_loop_len / 5);
                while pos < end {
                    subcells.push(T2Subcell {
                        cell_id_extension: sel[pos],
                        transposer_frequency: u32::from_be_bytes([
                            sel[pos + 1],
                            sel[pos + 2],
                            sel[pos + 3],
                            sel[pos + 4],
                        ]),
                    });
                    pos += 5;
                }
                cells.push(T2Cell {
                    cell_id,
                    centre_frequencies,
                    subcells,
                });
            }
            cells
        } else {
            Vec::new()
        };
        Ok(T2DeliverySystem {
            plp_id,
            t2_system_id,
            siso_miso,
            bandwidth,
            guard_interval,
            transmission_mode,
            other_frequency_flag,
            tfs_flag,
            cells,
        })
    }
}

impl Serialize for T2DeliverySystem {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let mut len = T2_FIXED_PREFIX_LEN;
        if self.siso_miso.is_some() {
            len += T2_FLAGS_BLOCK_LEN;
            let tfs = self.tfs_flag.unwrap_or(false);
            for cell in &self.cells {
                len += 2; // cell_id
                if tfs {
                    len += 1 + cell.centre_frequencies.len() * 4;
                } else {
                    len += 4;
                }
                len += 1 + cell.subcells.len() * 5;
            }
        }
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
        buf[0] = self.plp_id;
        buf[1..3].copy_from_slice(&self.t2_system_id.to_be_bytes());
        let mut p = T2_FIXED_PREFIX_LEN;
        if let (Some(sm), Some(bw), Some(gi), Some(tm), Some(off), Some(tfs)) = (
            self.siso_miso,
            self.bandwidth,
            self.guard_interval,
            self.transmission_mode,
            self.other_frequency_flag,
            self.tfs_flag,
        ) {
            buf[p] = (sm.to_u8() << 6) | ((bw.to_u8() & 0x0F) << 2) | 0x03;
            buf[p + 1] = (gi.to_u8() << 5)
                | ((tm.to_u8() & 0x07) << 2)
                | (u8::from(off) << 1)
                | u8::from(tfs);
            p += T2_FLAGS_BLOCK_LEN;
            for cell in &self.cells {
                buf[p..p + 2].copy_from_slice(&cell.cell_id.to_be_bytes());
                p += 2;
                if tfs {
                    let freq_len = (cell.centre_frequencies.len() * 4) as u8;
                    buf[p] = freq_len;
                    p += 1;
                    for &freq in &cell.centre_frequencies {
                        buf[p..p + 4].copy_from_slice(&freq.to_be_bytes());
                        p += 4;
                    }
                } else {
                    let freq = cell.centre_frequencies.first().copied().unwrap_or(0);
                    buf[p..p + 4].copy_from_slice(&freq.to_be_bytes());
                    p += 4;
                }
                let subcell_len = (cell.subcells.len() * 5) as u8;
                buf[p] = subcell_len;
                p += 1;
                for sc in &cell.subcells {
                    buf[p] = sc.cell_id_extension;
                    buf[p + 1..p + 5].copy_from_slice(&sc.transposer_frequency.to_be_bytes());
                    p += 5;
                }
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn t2_bandwidth_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(T2Bandwidth::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn t2_guard_interval_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(T2GuardInterval::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn t2_transmission_mode_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(T2TransmissionMode::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn parse_t2_minimal() {
        // body = plp + system_id = 3 bytes => no flags block
        let sel = [0x07, 0x12, 0x34];
        let bytes = wrap(0x04, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x07);
                assert_eq!(b.t2_system_id, 0x1234);
                assert_eq!(b.siso_miso, None);
                assert!(b.cells.is_empty());
            }
            other => panic!("expected T2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_t2_structured_flags_and_cells() {
        // prefix + flags block (siso=0, bw=4(10MHz), gi=6(19/256), tm=3(1k), off=0, tfs=1)
        // + 2 cells: one empty, one with 3 freqs + 2 subcells
        let b0: u8 = ((0x04 & 0x0F) << 2) | 0x03; // siso_miso=0, bandwidth=4, reserved=11
        let b1: u8 = (0x06 << 5) | ((0x03 & 0x07) << 2) | (u8::from(false) << 1) | u8::from(true);
        // cell 1: cell_id=0x1234, freq_len=0, subcell_len=0
        let cell1 = [0x12, 0x34, 0x00, 0x00];
        // cell 2: cell_id=0x5678, freq_len=12 (3 freqs), three freqs, subcell_len=10 (2 subcells), two subcells
        let f1 = 0x01020304u32;
        let f2 = 0x05060708u32;
        let f3 = 0x090A0B0Cu32;
        let sc1_id = 0x10u8;
        let sc1_freq = 0x11121314u32;
        let sc2_id = 0x20u8;
        let sc2_freq = 0x21222324u32;
        let mut cell2 = Vec::new();
        cell2.extend_from_slice(&0x5678u16.to_be_bytes());
        cell2.push(12);
        cell2.extend_from_slice(&f1.to_be_bytes());
        cell2.extend_from_slice(&f2.to_be_bytes());
        cell2.extend_from_slice(&f3.to_be_bytes());
        cell2.push(10);
        cell2.push(sc1_id);
        cell2.extend_from_slice(&sc1_freq.to_be_bytes());
        cell2.push(sc2_id);
        cell2.extend_from_slice(&sc2_freq.to_be_bytes());
        let mut sel = vec![0x07, 0x12, 0x34, b0, b1];
        sel.extend_from_slice(&cell1);
        sel.extend_from_slice(&cell2);
        let bytes = wrap(0x04, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x07);
                assert_eq!(b.t2_system_id, 0x1234);
                assert_eq!(b.siso_miso, Some(T2SisoMiso::Siso));
                assert_eq!(b.bandwidth, Some(T2Bandwidth::Mhz10));
                assert_eq!(b.guard_interval, Some(T2GuardInterval::G19_256));
                assert_eq!(b.transmission_mode, Some(T2TransmissionMode::Mode1k));
                assert_eq!(b.other_frequency_flag, Some(false));
                assert_eq!(b.tfs_flag, Some(true));
                assert_eq!(b.cells.len(), 2);
                // cell 0: empty
                assert_eq!(b.cells[0].cell_id, 0x1234);
                assert!(b.cells[0].centre_frequencies.is_empty());
                assert!(b.cells[0].subcells.is_empty());
                // cell 1: 3 freqs + 2 subcells
                assert_eq!(b.cells[1].cell_id, 0x5678);
                assert_eq!(b.cells[1].centre_frequencies, vec![f1, f2, f3]);
                assert_eq!(b.cells[1].subcells.len(), 2);
                assert_eq!(b.cells[1].subcells[0].cell_id_extension, sc1_id);
                assert_eq!(b.cells[1].subcells[0].transposer_frequency, sc1_freq);
                assert_eq!(b.cells[1].subcells[1].cell_id_extension, sc2_id);
                assert_eq!(b.cells[1].subcells[1].transposer_frequency, sc2_freq);

                // Accessor tests
                assert_eq!(
                    b.cells[1].subcells[0].transposer_frequency_hz(),
                    u64::from(sc1_freq) * 10
                );
                assert_eq!(
                    b.cells[1].centre_frequencies_hz(),
                    vec![u64::from(f1) * 10, u64::from(f2) * 10, u64::from(f3) * 10]
                );
            }
            other => panic!("expected T2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn tsduck_t2_reference() {
        let bytes = from_hex(
            "7f240456789a13cd12340000678a0c075bcd1505e30a780fd22c320a1217ea6406fa0aa9fc59",
        );
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x56);
                assert_eq!(b.t2_system_id, 0x789A);
                assert_eq!(b.siso_miso, Some(T2SisoMiso::Siso));
                assert_eq!(b.bandwidth, Some(T2Bandwidth::Mhz10));
                assert_eq!(b.guard_interval, Some(T2GuardInterval::G19_256));
                assert_eq!(b.transmission_mode, Some(T2TransmissionMode::Mode1k));
                assert_eq!(b.other_frequency_flag, Some(false));
                assert_eq!(b.tfs_flag, Some(true));
                assert_eq!(b.cells.len(), 2);

                assert_eq!(b.cells[0].cell_id, 0x1234);
                assert!(b.cells[0].centre_frequencies.is_empty());
                assert!(b.cells[0].subcells.is_empty());

                assert_eq!(b.cells[1].cell_id, 0x678A);
                assert_eq!(
                    b.cells[1].centre_frequencies,
                    vec![0x075BCD15, 0x05E30A78, 0x0FD22C32]
                );
                assert_eq!(b.cells[1].subcells.len(), 2);
                assert_eq!(b.cells[1].subcells[0].cell_id_extension, 0x12);
                assert_eq!(b.cells[1].subcells[0].transposer_frequency, 0x17EA6406);
                assert_eq!(b.cells[1].subcells[1].cell_id_extension, 0xFA);
                assert_eq!(b.cells[1].subcells[1].transposer_frequency, 0x0AA9FC59);
            }
            other => panic!("expected T2DeliverySystem, got {other:?}"),
        }
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(
            out[..n],
            bytes[..],
            "byte-exact re-serialize for tsduck T2 reference"
        );
    }
}
