use super::*;

impl ExtensionBodyDef for S2XSatelliteDeliverySystem<'_> {
    const TAG_EXTENSION: u8 = 0x17;
    const NAME: &'static str = "S2X_SATELLITE_DELIVERY_SYSTEM";
}

// ===========================================================================
//  Section 0x17 — S2X_satellite_delivery_system_descriptor (Table 140, §6.4.6.5.2)
// ---------------------------------------------------------------------------
//  Primary-channel fields are typed. The S2X_mode==3 channel-bonding loop and
//  the trailing reserved_future_use bytes are irregular and kept raw (SAT
//  precedent); `tail` holds everything after the primary input_stream_identifier
//  / timeslice_number.
// ===========================================================================
/// S2X_satellite_delivery_system body (Table 140); `tail` is the raw remainder.
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
    /// Raw remainder: S2X_mode==3 channel-bond loop + reserved tail.
    pub tail: &'a [u8],
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
        if sel.len() < pos + S2X_PRIMARY_LEN {
            return Err(invalid("S2X: primary channel truncated"));
        }
        let frequency = u32::from_be_bytes([sel[pos], sel[pos + 1], sel[pos + 2], sel[pos + 3]]);
        let orbital_position = u16::from_be_bytes([sel[pos + 4], sel[pos + 5]]);
        let pb = sel[pos + 6];
        let west_east_flag = (pb & 0x80) != 0;
        let polarization = (pb >> 5) & 0x03;
        let multiple_input_stream_flag = (pb & 0x10) != 0;
        let roll_off = pb & 0x07;
        let symbol_rate = (u32::from(sel[pos + 7] & 0x0F) << 24)
            | (u32::from(sel[pos + 8]) << 16)
            | (u32::from(sel[pos + 9]) << 8)
            | u32::from(sel[pos + 10]);
        pos += S2X_PRIMARY_LEN;
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
            tail: &sel[pos..],
        })
    }
}

impl Serialize for S2XSatelliteDeliverySystem<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        2 + if self.scrambling_sequence_selector {
            S2X_SCRAMBLING_LEN
        } else {
            0
        } + S2X_PRIMARY_LEN
            + usize::from(self.input_stream_identifier.is_some())
            + usize::from(self.timeslice_number.is_some())
            + self.tail.len()
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
        buf[p..p + 4].copy_from_slice(&self.frequency.to_be_bytes());
        buf[p + 4..p + 6].copy_from_slice(&self.orbital_position.to_be_bytes());
        buf[p + 6] = (u8::from(self.west_east_flag) << 7)
            | ((self.polarization & 0x03) << 5)
            | (u8::from(self.multiple_input_stream_flag) << 4)
            | (self.roll_off & 0x07);
        let sr = self.symbol_rate & 0x0FFF_FFFF;
        buf[p + 7] = (sr >> 24) as u8 & 0x0F;
        buf[p + 8] = (sr >> 16) as u8;
        buf[p + 9] = (sr >> 8) as u8;
        buf[p + 10] = sr as u8;
        p += S2X_PRIMARY_LEN;
        if let Some(isi) = self.input_stream_identifier {
            buf[p] = isi;
            p += 1;
        }
        if let Some(ts) = self.timeslice_number {
            buf[p] = ts;
            p += 1;
        }
        buf[p..p + self.tail.len()].copy_from_slice(self.tail);
        Ok(len)
    }
}
