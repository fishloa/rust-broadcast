//! S2X Satellite Delivery System Descriptor — ETSI EN 300 468 §6.4.6.5.2 (tag_extension 0x17).
//!
//! The `reserved_tail` field holds trailing `reserved_future_use` bytes
//! verbatim; future spec growth is surfaced via additive typed accessors.
use super::*;

impl super::sealed::Sealed for S2XSatelliteDeliverySystem<'_> {}
impl ExtensionBodyDef for S2XSatelliteDeliverySystem<'_> {
    const TAG_EXTENSION: u8 = 0x17;
    const NAME: &'static str = "S2X_SATELLITE_DELIVERY_SYSTEM";
}
/// A single channel-bond entry (Table 140 inner `for` loop).
///
/// Layout mirrors the primary channel: frequency(4) + orbital_position(2) +
/// packed byte + symbol_rate(4) + optional input_stream_identifier(1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct S2XChannelBond {
    /// frequency(32).
    pub frequency: u32,
    /// orbital_position(16).
    pub orbital_position: u16,
    /// west_east_flag(1).
    pub west_east_flag: bool,
    /// polarization(2).
    pub polarization: u8,
    /// bonded_channel_multiple_input_stream_flag(1).
    pub multiple_input_stream_flag: bool,
    /// roll_off(3).
    pub roll_off: u8,
    /// symbol_rate(28).
    pub symbol_rate: u32,
    /// input_stream_identifier(8), present iff `multiple_input_stream_flag`.
    pub input_stream_identifier: Option<u8>,
}

/// S2X_satellite_delivery_system body (Table 140).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct S2XSatelliteDeliverySystem<'a> {
    /// receiver_profiles(5) — Table 141.
    pub receiver_profiles: u8,
    /// S2X_mode(2) — Table 142.
    pub s2x_mode: u8,
    /// scrambling_sequence_selector(1).
    pub scrambling_sequence_selector: bool,
    /// TS_GS_S2X_mode(2) — Table 143.
    pub ts_gs_s2x_mode: u8,
    /// scrambling_sequence_index(18), present iff `scrambling_sequence_selector`.
    pub scrambling_sequence_index: Option<u32>,
    /// frequency(32) — primary channel.
    pub frequency: u32,
    /// orbital_position(16).
    pub orbital_position: u16,
    /// west_east_flag(1).
    pub west_east_flag: bool,
    /// polarization(2).
    pub polarization: u8,
    /// multiple_input_stream_flag(1).
    pub multiple_input_stream_flag: bool,
    /// roll_off(3) — Table 144.
    pub roll_off: u8,
    /// symbol_rate(28).
    pub symbol_rate: u32,
    /// input_stream_identifier(8), present iff `multiple_input_stream_flag`.
    pub input_stream_identifier: Option<u8>,
    /// timeslice_number(8), present iff `s2x_mode == 2`.
    pub timeslice_number: Option<u8>,
    /// S2X_mode==3 channel-bond entries (empty unless s2x_mode==3).
    pub channel_bonds: Vec<S2XChannelBond>,
    /// Trailing reserved_future_use bytes (opaque), preserved verbatim.
    pub reserved_tail: &'a [u8],
}

const BOND_BASE_LEN: usize = S2X_PRIMARY_LEN;

fn parse_channel_common(
    sel: &[u8],
    pos: &mut usize,
) -> Result<(u32, u16, bool, u8, bool, u8, u32)> {
    if sel.len() < *pos + BOND_BASE_LEN {
        return Err(invalid("S2X: channel block truncated"));
    }
    let frequency = u32::from_be_bytes([sel[*pos], sel[*pos + 1], sel[*pos + 2], sel[*pos + 3]]);
    let orbital_position = u16::from_be_bytes([sel[*pos + 4], sel[*pos + 5]]);
    let pb = sel[*pos + 6];
    let west_east_flag = (pb & 0x80) != 0;
    let polarization = (pb >> 5) & 0x03;
    let multiple_input_stream_flag = (pb & 0x10) != 0;
    let roll_off = pb & 0x07;
    let symbol_rate = (u32::from(sel[*pos + 7] & 0x0F) << 24)
        | (u32::from(sel[*pos + 8]) << 16)
        | (u32::from(sel[*pos + 9]) << 8)
        | u32::from(sel[*pos + 10]);
    *pos += BOND_BASE_LEN;
    Ok((
        frequency,
        orbital_position,
        west_east_flag,
        polarization,
        multiple_input_stream_flag,
        roll_off,
        symbol_rate,
    ))
}

fn write_channel_common(
    buf: &mut [u8],
    p: &mut usize,
    frequency: u32,
    orbital_position: u16,
    packed: u8,
    symbol_rate: u32,
) {
    buf[*p..*p + 4].copy_from_slice(&frequency.to_be_bytes());
    buf[*p + 4..*p + 6].copy_from_slice(&orbital_position.to_be_bytes());
    buf[*p + 6] = packed;
    let sr = symbol_rate & 0x0FFF_FFFF;
    buf[*p + 7] = (sr >> 24) as u8 & 0x0F;
    buf[*p + 8] = (sr >> 16) as u8;
    buf[*p + 9] = (sr >> 8) as u8;
    buf[*p + 10] = sr as u8;
    *p += BOND_BASE_LEN;
}

fn pack_we_pol_mis_ro(we: bool, pol: u8, mis: bool, ro: u8) -> u8 {
    (u8::from(we) << 7) | ((pol & 0x03) << 5) | (u8::from(mis) << 4) | (ro & 0x07)
}

impl<'a> Parse<'a> for S2XSatelliteDeliverySystem<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        // receiver_profiles byte + S2X mode/flags byte = 2 fixed bytes.
        if sel.len() < 2 {
            return Err(invalid("S2X: flags truncated"));
        }
        let receiver_profiles = sel[0] >> 3;
        let b1 = sel[1];
        // Table 140 byte 1, MSB-first: S2X_mode(2) scrambling_sequence_selector(1)
        // reserved_zero_future_use(3) TS_GS_S2X_mode(2).
        let s2x_mode = (b1 >> 6) & 0x03;
        let scrambling_sequence_selector = (b1 & 0x20) != 0;
        let ts_gs_s2x_mode = b1 & 0x03;
        let mut pos = 2;
        let scrambling_sequence_index = if scrambling_sequence_selector {
            if sel.len() < pos + S2X_SCRAMBLING_LEN {
                return Err(invalid("S2X: scrambling_sequence_index truncated"));
            }
            let idx = (u32::from(sel[pos] & 0x03) << 16)
                | (u32::from(sel[pos + 1]) << 8)
                | u32::from(sel[pos + 2]);
            pos += S2X_SCRAMBLING_LEN;
            Some(idx)
        } else {
            None
        };
        // Primary channel (Table 140): frequency(32) orbital_position(16)
        //   packed byte = west_east(1) polarization(2) mis(1) reserved(1) roll_off(3)
        //   then reserved(4) | symbol_rate[27:24], and 3 bytes symbol_rate[23:0].
        let (
            frequency,
            orbital_position,
            west_east_flag,
            polarization,
            multiple_input_stream_flag,
            roll_off,
            symbol_rate,
        ) = parse_channel_common(sel, &mut pos)?;
        let input_stream_identifier = if multiple_input_stream_flag {
            if sel.len() < pos + 1 {
                return Err(invalid("S2X: input_stream_identifier truncated"));
            }
            let isi = sel[pos];
            pos += 1;
            Some(isi)
        } else {
            None
        };
        let timeslice_number = if s2x_mode == 2 {
            if sel.len() < pos + 1 {
                return Err(invalid("S2X: timeslice_number truncated"));
            }
            let ts = sel[pos];
            pos += 1;
            Some(ts)
        } else {
            None
        };
        let (channel_bonds, reserved_tail) = if s2x_mode == 3 {
            // --- channel bonding loop (Table 140) ---
            if sel.len() < pos + 1 {
                return Err(invalid("S2X: channel-bond count byte truncated"));
            }
            let bond_byte = sel[pos];
            pos += 1;
            // reserved_zero_future_use(7) | num_channel_bonds_minus_one(1)
            let num_channel_bonds = (bond_byte & 0x01) as usize + 1;
            let mut bonds = Vec::with_capacity(num_channel_bonds);
            for _ in 0..num_channel_bonds {
                let (freq, orb, we, pol, mis, ro, sr) = parse_channel_common(sel, &mut pos)?;
                let isi = if mis {
                    if sel.len() < pos + 1 {
                        return Err(invalid("S2X: channel bond overruns body"));
                    }
                    let v = sel[pos];
                    pos += 1;
                    Some(v)
                } else {
                    None
                };
                bonds.push(S2XChannelBond {
                    frequency: freq,
                    orbital_position: orb,
                    west_east_flag: we,
                    polarization: pol,
                    multiple_input_stream_flag: mis,
                    roll_off: ro,
                    symbol_rate: sr,
                    input_stream_identifier: isi,
                });
            }
            (bonds, &sel[pos..])
        } else {
            (Vec::new(), &sel[pos..])
        };
        Ok(S2XSatelliteDeliverySystem {
            receiver_profiles,
            s2x_mode,
            scrambling_sequence_selector,
            ts_gs_s2x_mode,
            scrambling_sequence_index,
            frequency,
            orbital_position,
            west_east_flag,
            polarization,
            multiple_input_stream_flag,
            roll_off,
            symbol_rate,
            input_stream_identifier,
            timeslice_number,
            channel_bonds,
            reserved_tail,
        })
    }
}

impl Serialize for S2XSatelliteDeliverySystem<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let bond_len: usize = if self.s2x_mode == 3 {
            1 + self
                .channel_bonds
                .iter()
                .map(|b| BOND_BASE_LEN + usize::from(b.input_stream_identifier.is_some()))
                .sum::<usize>()
        } else {
            0
        };
        2 + if self.scrambling_sequence_selector {
            S2X_SCRAMBLING_LEN
        } else {
            0
        } + S2X_PRIMARY_LEN
            + usize::from(self.input_stream_identifier.is_some())
            + usize::from(self.timeslice_number.is_some())
            + bond_len
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
        buf[0] = self.receiver_profiles << 3;
        buf[1] = ((self.s2x_mode & 0x03) << 6)
            | (u8::from(self.scrambling_sequence_selector) << 5)
            | (self.ts_gs_s2x_mode & 0x03);
        let mut p = 2;
        if self.scrambling_sequence_selector {
            let idx = self.scrambling_sequence_index.unwrap_or(0) & 0x3FFFF;
            buf[p] = (idx >> 16) as u8 & 0x03;
            buf[p + 1] = (idx >> 8) as u8;
            buf[p + 2] = idx as u8;
            p += S2X_SCRAMBLING_LEN;
        }
        write_channel_common(
            buf,
            &mut p,
            self.frequency,
            self.orbital_position,
            pack_we_pol_mis_ro(
                self.west_east_flag,
                self.polarization,
                self.multiple_input_stream_flag,
                self.roll_off,
            ),
            self.symbol_rate,
        );
        if let Some(isi) = self.input_stream_identifier {
            buf[p] = isi;
            p += 1;
        }
        if let Some(ts) = self.timeslice_number {
            buf[p] = ts;
            p += 1;
        }
        if self.s2x_mode == 3 {
            // reserved_zero_future_use(7) | num_channel_bonds_minus_one(1)
            buf[p] = (self.channel_bonds.len() as u8).saturating_sub(1) & 0x01;
            p += 1;
            for bond in &self.channel_bonds {
                write_channel_common(
                    buf,
                    &mut p,
                    bond.frequency,
                    bond.orbital_position,
                    pack_we_pol_mis_ro(
                        bond.west_east_flag,
                        bond.polarization,
                        bond.multiple_input_stream_flag,
                        bond.roll_off,
                    ),
                    bond.symbol_rate,
                );
                if let Some(isi) = bond.input_stream_identifier {
                    buf[p] = isi;
                    p += 1;
                }
            }
        }
        buf[p..p + self.reserved_tail.len()].copy_from_slice(self.reserved_tail);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_s2x_primary_with_isi_and_timeslice() {
        // receiver_profiles=0x05; s2x_mode=2, scram_sel=0, ts_gs=1; ISI + timeslice
        let b0 = 0x05 << 3;
        let b1 = (0x02 << 6) | 0x01; // mode 2 [7:6], no scrambling, ts_gs 1 [1:0]
        let mut sel = vec![b0, b1];
        sel.extend_from_slice(&0x0102_0304u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0x00C8u16.to_be_bytes()); // orbital_position
        sel.push((1 << 7) | (0x02 << 5) | (1 << 4) | 0x03); // we=1 pol=2 mis=1 roll=3
        let sr: u32 = 0x0AB_CDEF; // symbol_rate (28-bit)
        sel.push((sr >> 24) as u8 & 0x0F);
        sel.push((sr >> 16) as u8);
        sel.push((sr >> 8) as u8);
        sel.push(sr as u8);
        sel.push(0x42); // input_stream_identifier (mis=1)
        sel.push(0x09); // timeslice_number (mode==2)
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.receiver_profiles, 0x05);
                assert_eq!(b.s2x_mode, 2);
                assert!(!b.scrambling_sequence_selector);
                assert_eq!(b.ts_gs_s2x_mode, 1);
                assert_eq!(b.frequency, 0x0102_0304);
                assert_eq!(b.orbital_position, 0x00C8);
                assert!(b.west_east_flag);
                assert_eq!(b.polarization, 2);
                assert!(b.multiple_input_stream_flag);
                assert_eq!(b.roll_off, 3);
                assert_eq!(b.symbol_rate, 0x0AB_CDEF);
                assert_eq!(b.input_stream_identifier, Some(0x42));
                assert_eq!(b.timeslice_number, Some(0x09));
                assert!(b.channel_bonds.is_empty());
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_with_scrambling_index() {
        let b0 = 0x01 << 3;
        let b1 = (0x01 << 6) | 0x20; // mode 1 [7:6], scrambling selector set [5]
        let mut sel = vec![b0, b1];
        // scrambling index 0x2ABCD (18-bit)
        sel.push(0x02);
        sel.push(0xAB);
        sel.push(0xCD);
        sel.extend_from_slice(&0u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0u16.to_be_bytes()); // orbital
        sel.push(0x00); // packed (mis=0)
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert!(b.scrambling_sequence_selector);
                assert_eq!(b.scrambling_sequence_index, Some(0x2ABCD));
                assert_eq!(b.input_stream_identifier, None);
                assert_eq!(b.timeslice_number, None);
                assert!(b.channel_bonds.is_empty());
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_mode1_tail_preserved() {
        // mode 1 — no channel bonds; trailing bytes become reserved_tail.
        let b0 = 0x01 << 3;
        let b1 = 0x01 << 6; // mode 1 [7:6], no scrambling, ts_gs 0
        let mut sel = vec![b0, b1];
        sel.extend_from_slice(&0u32.to_be_bytes());
        sel.extend_from_slice(&0u16.to_be_bytes());
        sel.push(0x00); // mis=0
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
        sel.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // reserved_future_use tail
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.s2x_mode, 1);
                assert_eq!(b.timeslice_number, None);
                assert!(b.channel_bonds.is_empty());
                assert_eq!(b.reserved_tail, &[0xAA, 0xBB, 0xCC]);
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_mode3_channel_bonds() {
        // mode 3 — 2 channel bonds (one with MIS/ISI, one without) + empty tail.
        let b0 = 0x01 << 3;
        let b1 = 0x03 << 6; // mode 3 [7:6], no scrambling, ts_gs 0
        let mut sel = vec![b0, b1];
        // Primary channel
        sel.extend_from_slice(&0x1111_1111u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0x0001u16.to_be_bytes()); // orbital
        sel.push(0x00); // mis=0
        sel.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // symbol_rate

        // Bond count: reserved(7)=0 | num_channel_bonds_minus_one(1)=1 → 2 bonds
        sel.push(0x01);

        // Bond 0 (with MIS/ISI): frequency=0x22222222, orbital=0x0002,
        //   we=1 pol=2 mis=1 roll=3, symbol_rate=0x0ABCDEF, isi=0x77
        sel.extend_from_slice(&0x2222_2222u32.to_be_bytes());
        sel.extend_from_slice(&0x0002u16.to_be_bytes());
        sel.push((1 << 7) | (0x02 << 5) | (1 << 4) | 0x03); // we=1 pol=2 mis=1 roll=3
        let sr: u32 = 0x0AB_CDEF;
        sel.push((sr >> 24) as u8 & 0x0F);
        sel.push((sr >> 16) as u8);
        sel.push((sr >> 8) as u8);
        sel.push(sr as u8);
        sel.push(0x77); // input_stream_identifier

        // Bond 1 (no MIS): frequency=0x33333333, orbital=0x0003,
        //   we=0 pol=1 mis=0 roll=4, symbol_rate=0x0054321
        sel.extend_from_slice(&0x3333_3333u32.to_be_bytes());
        sel.extend_from_slice(&0x0003u16.to_be_bytes());
        sel.push((0x01 << 5) | 0x04); // we=0 pol=1 mis=0 roll=4
        let sr2: u32 = 0x005_4321;
        sel.push((sr2 >> 24) as u8 & 0x0F);
        sel.push((sr2 >> 16) as u8);
        sel.push((sr2 >> 8) as u8);
        sel.push(sr2 as u8);

        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.s2x_mode, 3);
                assert_eq!(b.channel_bonds.len(), 2);

                let b0 = &b.channel_bonds[0];
                assert_eq!(b0.frequency, 0x2222_2222);
                assert_eq!(b0.orbital_position, 0x0002);
                assert!(b0.west_east_flag);
                assert_eq!(b0.polarization, 2);
                assert!(b0.multiple_input_stream_flag);
                assert_eq!(b0.roll_off, 3);
                assert_eq!(b0.symbol_rate, 0x0AB_CDEF);
                assert_eq!(b0.input_stream_identifier, Some(0x77));

                let b1 = &b.channel_bonds[1];
                assert_eq!(b1.frequency, 0x3333_3333);
                assert_eq!(b1.orbital_position, 0x0003);
                assert!(!b1.west_east_flag);
                assert_eq!(b1.polarization, 1);
                assert!(!b1.multiple_input_stream_flag);
                assert_eq!(b1.roll_off, 4);
                assert_eq!(b1.symbol_rate, 0x005_4321);
                assert_eq!(b1.input_stream_identifier, None);

                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn tsduck_s2x_mode3_byte_exact() {
        // TSDuck reference test-015: real s2x_mode==3 descriptor with
        // scrambling, 2 channel bonds, and a 1-byte reserved tail.
        let hex = "7f2a1750e3023456876543210037250456789601065432180340f600246754bd00654367123451000087642e";
        let bytes = from_hex(hex);
        let d = ExtensionDescriptor::parse(&bytes)
            .unwrap_or_else(|e| panic!("parse tsduck s2x: {e:?}"));

        assert_eq!(d.kind(), Some(ExtensionTag::S2XSatelliteDeliverySystem));
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.s2x_mode, 3);
                assert!(b.scrambling_sequence_selector);
                assert_eq!(b.scrambling_sequence_index, Some(0x023456));
                assert!(!b.channel_bonds.is_empty());
                assert_eq!(b.channel_bonds.len(), 2);

                let b0 = &b.channel_bonds[0];
                assert_eq!(b0.frequency, 0x0654_3218);
                assert_eq!(b0.orbital_position, 0x0340);
                assert!(b0.west_east_flag);
                assert_eq!(b0.polarization, 3);
                assert!(b0.multiple_input_stream_flag);
                assert_eq!(b0.roll_off, 6);
                assert_eq!(b0.symbol_rate, 0x0024_6754);
                assert_eq!(b0.input_stream_identifier, Some(0xBD));

                let b1 = &b.channel_bonds[1];
                assert_eq!(b1.frequency, 0x0065_4367);
                assert_eq!(b1.orbital_position, 0x1234);
                assert!(!b1.west_east_flag);
                assert_eq!(b1.polarization, 2);
                assert!(b1.multiple_input_stream_flag);
                assert_eq!(b1.roll_off, 1);
                assert_eq!(b1.symbol_rate, 0x0000_8764);
                assert_eq!(b1.input_stream_identifier, Some(0x2E));

                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected S2X, got {other:?}"),
        }

        // Byte-exact round-trip: serialize must match input exactly
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(
            &out[..n],
            &bytes[..],
            "S2X mode 3 byte-exact re-serialize failed"
        );
    }
}
