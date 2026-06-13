//! BBHEADER (Base-Band Header) parser and builder.
//!
//! Supports both Normal Mode (NM) and High Efficiency Mode (HEM)
//! per EN 302 755 v1.4.1 §5.1.7.

use dvb_common::{Parse, Serialize};
use num_enum::TryFromPrimitive;

use crate::crc::crc8;
use crate::error::Error;
use crate::issy::{decode_issy_long, decode_issy_short, Issy};

/// Total bytes in a BBHEADER.
pub const BBHEADER_LEN: usize = 10;
/// Loosest valid DFL upper bound in bits across the standards this crate parses.
///
/// DVB-S2 normal FECFRAME caps the data field near 64800 bits; DVB-T2 is tighter
/// (EN 302 755 Table 2: DFL in [0, 53760]). A BBHEADER does not by itself say
/// which standard produced it, so this generous bound avoids rejecting any valid
/// S2/S2X/T2 frame.
pub const DFL_MAX_BITS: u16 = 64800;

/// Input stream format as described by the TS/GS field (MATYPE-1 bits `[7:6]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
pub enum TsGs {
    /// Generic Packetized Stream.
    Gfps = 0b00,
    /// Transport Stream (MPEG-2 TS, 188-byte packets).
    Ts = 0b11,
    /// Generic Continuous Stream.
    Gcs = 0b01,
    /// Generic Encapsulated Stream.
    Gse = 0b10,
}

impl From<TsGs> for u8 {
    fn from(t: TsGs) -> Self {
        t as u8
    }
}

impl From<num_enum::TryFromPrimitiveError<TsGs>> for Error {
    fn from(e: num_enum::TryFromPrimitiveError<TsGs>) -> Self {
        Error::UnsupportedTsGs { ts_gs: e.number }
    }
}

impl std::fmt::Display for TsGs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TsGs::{self:?}")
    }
}

/// Operating mode: Normal or High Efficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum Mode {
    /// Normal Mode — UPL/SYNC/SYNCD present, CRC-8 per UP.
    Normal = 0,
    /// High Efficiency Mode — ISSY replaces UPL/SYNC, no per-UP CRC-8.
    HighEfficiency = 1,
}

impl From<num_enum::TryFromPrimitiveError<Mode>> for Error {
    fn from(e: num_enum::TryFromPrimitiveError<Mode>) -> Self {
        Error::InvalidMode { mode: e.number }
    }
}

/// The pair of MATYPE bytes describing the input stream format and mode adaptation.
///
/// Per EN 302 755 Table 1:
/// - MATYPE-1 (byte 0): TS/GS, SIS/MIS, CCM/ACM, ISSYI, NPD, `EXT[1:0]`
/// - MATYPE-2 (byte 1): ISI (0-255) or reserved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Matype {
    /// Input stream format — see [`TsGs`].
    pub ts_gs: TsGs,
    /// Single-input stream (true) or multi-input stream (false).
    pub sis: bool,
    /// Constant coding and modulation (true) or adaptive (false).
    pub ccm: bool,
    /// Input Stream Synchronization Indicator — ISSY field is active.
    pub issyi: bool,
    /// Null Packet Deletion is active.
    pub npd: bool,
    /// Extension bits — RO in DVB-S2, reserved in DVB-T2.
    pub ext: u8,
    /// Input Stream Identifier — meaningful only if `sis == false`.
    pub isi: u8,
}

impl Matype {
    const MASK_TS_GS: u8 = 0xC0;
    const MASK_SIS: u8 = 0x20;
    const MASK_CCM: u8 = 0x10;
    const MASK_ISSYI: u8 = 0x08;
    const MASK_NPD: u8 = 0x04;
    const MASK_EXT: u8 = 0x03;
}

/// Roll-off factor encoded in MATYPE-1 EXT bits `[1:0]` (DVB-S2/S2X context).
///
/// EN 302 307 / EN 302 755 — roll-off is signalled in the MATYPE-1 extension
/// bits when the TS/GS field indicates TS or GFPS.  In DVB-T2 the EXT field is
/// reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RollOff {
    /// 0b00 — α0.35 (0.35).
    Alpha035,
    /// 0b01 — α0.25 (0.25).
    Alpha025,
    /// 0b10 — α0.20 (0.20).
    Alpha020,
    /// 0b11 — reserved / S2X low roll-off.
    Reserved,
}

impl RollOff {
    /// Decode from 2-bit EXT field.
    /// Decode from 2-bit EXT field.
    #[must_use]
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::Alpha035,
            1 => Self::Alpha025,
            2 => Self::Alpha020,
            _ => Self::Reserved,
        }
    }

    /// Encode to 2-bit value.
    /// Encode to 2-bit value.
    #[must_use]
    pub fn to_bits(self) -> u8 {
        match self {
            Self::Alpha035 => 0,
            Self::Alpha025 => 1,
            Self::Alpha020 => 2,
            Self::Reserved => 3,
        }
    }

    /// Human-readable roll-off label.
    /// Human-readable spec display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Alpha035 => "α0.35",
            Self::Alpha025 => "α0.25",
            Self::Alpha020 => "α0.20",
            Self::Reserved => "reserved/S2X-low",
        }
    }
}

impl Matype {
    /// Decode the roll-off factor from the EXT bits `[1:0]`.
    ///
    /// Meaningful in DVB-S2/S2X context; reserved in DVB-T2.
    #[must_use]
    pub fn roll_off(&self) -> RollOff {
        RollOff::from_bits(self.ext & 0x03)
    }
}

impl TryFrom<[u8; 2]> for Matype {
    type Error = Error;

    fn try_from(bytes: [u8; 2]) -> Result<Self, Self::Error> {
        let matype1 = bytes[0];
        let matype2 = bytes[1];

        let ts_gs = TsGs::try_from((matype1 & Matype::MASK_TS_GS) >> 6)?;
        let sis = matype1 & Matype::MASK_SIS != 0;
        let ccm = matype1 & Matype::MASK_CCM != 0;
        let issyi = matype1 & Matype::MASK_ISSYI != 0;
        let npd = matype1 & Matype::MASK_NPD != 0;
        let ext = matype1 & Matype::MASK_EXT;

        Ok(Matype {
            ts_gs,
            sis,
            ccm,
            issyi,
            npd,
            ext,
            isi: matype2,
        })
    }
}

impl From<Matype> for [u8; 2] {
    fn from(m: Matype) -> Self {
        // MATYPE-1 layout per EN 302 755 Table 1:
        //   bits 7..6 TS/GS, bit 5 SIS/MIS, bit 4 CCM/ACM,
        //   bit 3 ISSYI, bit 2 NPD, bits 1..0 EXT.
        let mut matype1: u8 = 0;
        matype1 |= (u8::from(m.ts_gs) << 6) & Matype::MASK_TS_GS;
        if m.sis {
            matype1 |= Matype::MASK_SIS;
        }
        if m.ccm {
            matype1 |= Matype::MASK_CCM;
        }
        if m.issyi {
            matype1 |= Matype::MASK_ISSYI;
        }
        if m.npd {
            matype1 |= Matype::MASK_NPD;
        }
        matype1 |= m.ext & Matype::MASK_EXT;
        [matype1, m.isi]
    }
}

/// Parsed 10-byte BBHEADER.
///
/// Fields vary based on [`Mode`]:
/// - **NM** (MODE=0): `upl`, `sync` are from the header; `issy_in_header` is None.
/// - **HEM** (MODE=1): `upl` and `sync` are both zero; `issy_in_header` carries the
///   3 ISSY bytes that reuse the NM UPL/SYNC layout.
///
/// Detection: `crc8(bytes[0..9]) ^ bytes[9]` yields 0 for NM, 1 for HEM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Bbheader {
    /// MATYPE field.
    pub matype: Matype,
    /// User Packet Length in bits (NM only; 0 in HEM).
    pub upl: u16,
    /// Copy of the User Packet Sync-byte (NM only; 0 in HEM).
    pub sync: u8,
    /// Data Field Length in bits.
    pub dfl: u16,
    /// Distance in bits from DATA FIELD start to the first complete UP beginning.
    pub syncd: u16,
    /// Detected mode (NM vs HEM).
    pub mode: Mode,
    /// 3-byte ISSY from header bytes (HEM only; None in NM).
    pub issy_in_header: Option<[u8; 3]>,
}

impl<'a> Parse<'a> for Bbheader {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, Self::Error> {
        if bytes.len() < BBHEADER_LEN {
            return Err(Error::BufferTooShort {
                need: BBHEADER_LEN,
                have: bytes.len(),
                what: "BBHEADER",
            });
        }

        let matype_bytes = [bytes[0], bytes[1]];
        let matype = Matype::try_from(matype_bytes)?;
        let dfl = u16::from_be_bytes([bytes[4], bytes[5]]);
        let syncd = u16::from_be_bytes([bytes[7], bytes[8]]);
        let crc_stored = bytes[9];

        if dfl > DFL_MAX_BITS {
            return Err(Error::DflOutOfRange {
                dfl,
                max: DFL_MAX_BITS,
            });
        }

        // Mode detection per EN 302 755 §5.1.7: the byte on the wire is
        // `crc8(bytes[0..9]) XOR MODE` (MODE: 0 = NM, 1 = HEM). The XOR is
        // itself the integrity check — corruption that lands `mode_val`
        // outside {0, 1} is rejected by `Mode::try_from` as InvalidMode; a
        // residual flip into the other valid mode is undetectable by design
        // of the spec's scheme (there is no separate "HEM CRC init").
        let computed_crc = crc8(&bytes[..9]);
        let mode_val = computed_crc ^ crc_stored;
        let mode = Mode::try_from(mode_val)?;

        let (upl, sync, issy_in_header) = match mode {
            Mode::Normal => (u16::from_be_bytes([bytes[2], bytes[3]]), bytes[6], None),
            Mode::HighEfficiency => {
                // In HEM, bytes[2..4] are ISSY_2MSB, byte[6] is ISSY_1LSB —
                // UPL and SYNC are repurposed for ISSY.
                (0, 0, Some([bytes[2], bytes[3], bytes[6]]))
            }
        };

        Ok(Bbheader {
            matype,
            upl,
            sync,
            dfl,
            syncd,
            mode,
            issy_in_header,
        })
    }
}

impl Serialize for Bbheader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BBHEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.len() < BBHEADER_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: BBHEADER_LEN,
                have: buf.len(),
            });
        }

        let ma = <[u8; 2]>::from(self.matype);
        buf[0] = ma[0];
        buf[1] = ma[1];

        match self.mode {
            Mode::Normal => {
                let upl = self.upl.to_be_bytes();
                buf[2] = upl[0];
                buf[3] = upl[1];
                let dfl = self.dfl.to_be_bytes();
                buf[4] = dfl[0];
                buf[5] = dfl[1];
                buf[6] = self.sync;
                let syncd = self.syncd.to_be_bytes();
                buf[7] = syncd[0];
                buf[8] = syncd[1];
            }
            Mode::HighEfficiency => {
                if let Some(issy) = self.issy_in_header {
                    buf[2] = issy[0];
                    buf[3] = issy[1];
                    let dfl = self.dfl.to_be_bytes();
                    buf[4] = dfl[0];
                    buf[5] = dfl[1];
                    buf[6] = issy[2];
                    let syncd = self.syncd.to_be_bytes();
                    buf[7] = syncd[0];
                    buf[8] = syncd[1];
                } else {
                    // No ISSY in header: explicitly zero the three ISSY positions
                    // so serialize_into is fully deterministic regardless of whether
                    // the caller's buffer was zero-initialised or not. Without this,
                    // stale bytes at buf[2], buf[3], buf[6] corrupt the CRC-8.
                    buf[2] = 0;
                    buf[3] = 0;
                    let dfl = self.dfl.to_be_bytes();
                    buf[4] = dfl[0];
                    buf[5] = dfl[1];
                    buf[6] = 0;
                    let syncd = self.syncd.to_be_bytes();
                    buf[7] = syncd[0];
                    buf[8] = syncd[1];
                }
            }
        }

        // CRC-8 = crc8(bytes[0..9], init=0x00) XOR MODE
        let computed = crc8(&buf[..9]);
        buf[9] = computed ^ (self.mode as u8);

        Ok(BBHEADER_LEN)
    }
}

impl Bbheader {
    /// Parse a 10-byte BBHEADER, detecting NM vs HEM automatically.
    ///
    /// Mode detection per EN 302 755 §5.1.7:
    /// `mode = crc8(bytes[0..9]) ^ bytes[9]` (0 = NM, 1 = HEM).
    /// Values other than 0 or 1 return `Error::InvalidMode`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        <Self as Parse>::parse(bytes)
    }

    /// Serialize the BBHEADER back to its 10-byte wire format.
    pub fn serialize(&self) -> [u8; BBHEADER_LEN] {
        let v = <Self as Serialize>::to_bytes(self);
        let mut buf = [0u8; BBHEADER_LEN];
        buf.copy_from_slice(&v);
        buf
    }

    /// Decode the ISSY field from the header bytes (HEM only).
    ///
    /// In HEM the 3-byte ISSY occupies the UPL/SYNC positions. This method
    /// tries [`decode_issy_long`] first (the common case for 3-byte ISSY);
    /// if that fails because the form bit indicates a short-form ISSY,
    /// falls back to [`decode_issy_short`] on the first 2 bytes.
    ///
    /// Returns `None` in Normal Mode (no ISSY in header) or if decoding
    /// produces an unexpected error.
    #[must_use]
    pub fn issy(&self) -> Option<Issy> {
        let bytes = self.issy_in_header?;
        decode_issy_long(bytes)
            .ok()
            .or_else(|| decode_issy_short([bytes[0], bytes[1]]).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_buffer_shorter_than_10() {
        assert!(Bbheader::parse(&[0u8; 9]).is_err());
    }

    #[test]
    fn parse_nm_ts_extracts_all_fields() {
        // Craft a valid NM BBHEADER with known values.
        // MATYPE-1 = 0xF0: TS input (0b11), SIS (1), CCM (1), ISSYI (0), NPD (0), EXT (00)
        // MATYPE-2 = 0x00 (single stream)
        // UPL = 0x0718 = 1816 bits (188*8 - CRC-8 - sync = 1504-8 = 1496... let me just pick a value)
        // DFL = 0xBC00 = 50304-50432? Let me pick simpler values.
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF0; // MATYPE-1: TS, SIS, CCM
        hdr[1] = 0x00; // MATYPE-2: not MIS
        let upl: u16 = 0x07D0; // 2000 bits = 250 bytes
        hdr[2..4].copy_from_slice(&upl.to_be_bytes());
        let dfl: u16 = 0xBC00; // 48320 bits
        hdr[4..6].copy_from_slice(&dfl.to_be_bytes());
        hdr[6] = 0x47; // SYNC byte
        let syncd: u16 = 0x0000; // First UP aligned
        hdr[7..9].copy_from_slice(&syncd.to_be_bytes());
        hdr[9] = crc8(&hdr[..9]); // CRC-8

        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::Normal);
        assert_eq!(result.matype.ts_gs, TsGs::Ts);
        assert!(result.matype.sis);
        assert!(result.matype.ccm);
        assert!(!result.matype.issyi);
        assert!(!result.matype.npd);
        assert_eq!(result.matype.ext, 0);
        assert_eq!(result.matype.isi, 0x00);
        assert_eq!(result.upl, upl);
        assert_eq!(result.sync, 0x47);
        assert_eq!(result.dfl, dfl);
        assert_eq!(result.syncd, syncd);
    }

    #[test]
    fn parse_nm_gcs_treats_sync_as_transport_protocol_byte() {
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0x50; // MATYPE-1: GCS (0b01), SIS, CCM, ISSYI=0, NPD=0, EXT=00
        hdr[1] = 0x00;
        let upl: u16 = 0x0000; // GCS: UPL=0
        hdr[2..4].copy_from_slice(&upl.to_be_bytes());
        let dfl: u16 = 0x4000; // 16384 bits
        hdr[4..6].copy_from_slice(&dfl.to_be_bytes());
        hdr[6] = 0x3C; // GCS: SYNC=0x00-0xB8 for protocol signalling
        let syncd: u16 = 0x0000;
        hdr[7..9].copy_from_slice(&syncd.to_be_bytes());
        hdr[9] = crc8(&hdr[..9]);

        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::Normal);
        assert_eq!(result.matype.ts_gs, TsGs::Gcs);
        assert_eq!(result.sync, 0x3C);
        assert_eq!(result.upl, upl);
    }

    #[test]
    fn parse_detects_nm_via_crc_xor_0() {
        // When crc8(init=0) XOR byte[9] == 0, mode is NM
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF0;
        hdr[1] = 0x00;
        hdr[2] = 0x07;
        hdr[3] = 0xD0; // UPL
        hdr[4] = 0xBC;
        hdr[5] = 0x00; // DFL
        hdr[6] = 0x47; // SYNC
        hdr[7] = 0x00;
        hdr[8] = 0x00; // SYNCD
        hdr[9] = crc8(&hdr[..9]); // CRC matches init=0x00

        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::Normal);
    }

    #[test]
    fn parse_rejects_crc_mismatch_in_both_modes() {
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF0;
        hdr[1] = 0x00;
        hdr[2] = 0x07;
        hdr[3] = 0xD0;
        hdr[4] = 0xBC;
        hdr[5] = 0x00;
        hdr[6] = 0x47;
        hdr[7] = 0x00;
        hdr[8] = 0x00;
        hdr[9] = 0xFF; // Wrong CRC

        let result = Bbheader::parse(&hdr);
        assert!(result.is_err());
    }

    #[test]
    fn parse_matype_extracts_ts_gs_enum_for_each_of_gfps_ts_gcs_gse() {
        for (ts_gs_val, expected) in [
            (0b00, TsGs::Gfps),
            (0b01, TsGs::Gcs),
            (0b10, TsGs::Gse),
            (0b11, TsGs::Ts),
        ] {
            let ma1 = (ts_gs_val << 6) | 0x30; // SIS=1, CCM=1, ISSYI=0, NPD=0, EXT=00
            let mut hdr = [0u8; BBHEADER_LEN];
            hdr[0] = ma1;
            hdr[1] = 0x00;
            hdr[2..9].copy_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
            hdr[9] = crc8(&hdr[..9]);
            let result = Bbheader::parse(&hdr).unwrap();
            assert_eq!(result.matype.ts_gs, expected, "ts_gs=0x{:02b}", ts_gs_val);
        }
    }

    #[test]
    fn parse_matype_extracts_sis_isi_on_multi_stream() {
        // sis = 0 → MIS, isi is MATYPE-2
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xD0; // TS/MIS/CCM -> not SIS
        hdr[1] = 0xAB; // ISI = 171
        hdr[2] = 0x07;
        hdr[3] = 0xD0;
        hdr[4] = 0xBC;
        hdr[5] = 0x00;
        hdr[6] = 0x47;
        hdr[7] = 0x00;
        hdr[8] = 0x00;
        hdr[9] = crc8(&hdr[..9]);

        let result = Bbheader::parse(&hdr).unwrap();
        assert!(!result.matype.sis);
        assert_eq!(result.matype.isi, 0xAB);
    }

    #[test]
    fn parse_matype_extracts_roll_off_2_bits_as_ext_for_s2_context() {
        // EXT = 0b11 in NM means reserved/S2X-low roll-off (0b00 is α0.35)
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF3; // TS/SIS/CCM, no ISSYI, no NPD, EXT=0b11
        hdr[1] = 0x00;
        hdr[2] = 0x07;
        hdr[3] = 0xD0;
        hdr[4] = 0xBC;
        hdr[5] = 0x00;
        hdr[6] = 0x47;
        hdr[7] = 0x00;
        hdr[8] = 0x00;
        hdr[9] = crc8(&hdr[..9]);

        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.matype.ext, 0b11);
    }

    #[test]
    fn serialize_nm_produces_expected_bytes() {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 188 * 8,
            sync: 0x47,
            dfl: 48328,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let buf = hdr.serialize();

        let parsed = Bbheader::parse(&buf).unwrap();
        assert_eq!(parsed.matype.ts_gs, TsGs::Ts);
        assert!(parsed.matype.sis);
        assert!(parsed.matype.ccm);
        assert_eq!(parsed.upl, 188 * 8);
        assert_eq!(parsed.sync, 0x47);
        assert_eq!(parsed.dfl, 48328);
        assert_eq!(parsed.syncd, 0);
        assert_eq!(parsed.mode, Mode::Normal);
    }

    #[test]
    fn serialize_round_trip_nm_ts_preserves_every_field() {
        let orig = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 1504,
            sync: 0x47,
            dfl: 48328,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let buf = orig.serialize();
        let parsed = Bbheader::parse(&buf).unwrap();
        assert_eq!(orig.matype.ts_gs, parsed.matype.ts_gs);
        assert_eq!(orig.matype.sis, parsed.matype.sis);
        assert_eq!(orig.matype.ccm, parsed.matype.ccm);
        assert_eq!(orig.matype.issyi, parsed.matype.issyi);
        assert_eq!(orig.matype.npd, parsed.matype.npd);
        assert_eq!(orig.matype.ext, parsed.matype.ext);
        assert_eq!(orig.matype.isi, parsed.matype.isi);
        assert_eq!(orig.upl, parsed.upl);
        assert_eq!(orig.sync, parsed.sync);
        assert_eq!(orig.dfl, parsed.dfl);
        assert_eq!(orig.syncd, parsed.syncd);
        assert_eq!(orig.mode, parsed.mode);
    }

    #[test]
    fn serialize_round_trip_nm_gcs() {
        let orig = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Gcs,
                sis: true,
                ccm: false,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0x00,
            dfl: 16384,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let buf = orig.serialize();
        let parsed = Bbheader::parse(&buf).unwrap();
        assert_eq!(orig.matype.ts_gs, parsed.matype.ts_gs);
        assert_eq!(orig.matype.sis, parsed.matype.sis);
        assert_eq!(orig.matype.ccm, parsed.matype.ccm);
        assert_eq!(orig.dfl, parsed.dfl);
        assert_eq!(orig.syncd, parsed.syncd);
        assert_eq!(orig.mode, parsed.mode);
    }

    #[test]
    fn serialize_crc8_always_matches_bytes_0_to_8() {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Gse,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0xFF,
            dfl: 32768,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let buf = hdr.serialize();
        let computed = crc8(&buf[..9]);
        assert_eq!(computed ^ buf[9], 0); // XOR with MODE must give 0 for NM
        assert_eq!(buf[9], computed); // MODE=0 means they must be equal
    }

    #[test]
    fn parse_detects_hem_via_crc_xor_1() {
        // Real DVB-T2 BBFRAME header from Rai T2-MI. Mode=HEM is detected by crc8(init=0) XOR stored = 1.
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::HighEfficiency);
    }

    #[test]
    fn parse_hem_extracts_matype_dfl_syncd() {
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::HighEfficiency);
        assert_eq!(result.matype.ts_gs, TsGs::Ts);
        assert!(result.matype.sis);
        assert!(result.matype.ccm);
        assert!(result.matype.issyi);
        assert!(!result.matype.npd);
        assert_eq!(result.matype.ext, 0);
        assert_eq!(result.dfl, 48328);
        assert_eq!(result.syncd, 0x0350);
    }

    #[test]
    fn parse_hem_preserves_three_issy_bytes() {
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        let result = Bbheader::parse(&hdr).unwrap();
        let issy = result.issy_in_header.unwrap();
        // ISSY in HEM: bytes[2..4] = ISSY_2MSB, byte[6] = ISSY_1LSB
        assert_eq!(issy, [0xa4, 0x28, 0xe2]);
    }

    #[test]
    fn issy_accessor_decodes_hem_iscr_long() {
        // Real HEM fixture: ISSY bytes [0xa4, 0x28, 0xe2]
        // byte0=0xa4, bit7=1 (long form), bit6=0 (ISCR long)
        // payload = (0xa4 & 0x3F)<<16 | 0x28<<8 | 0xe2 = 0x2428e2
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        let result = Bbheader::parse(&hdr).unwrap();
        let issy = result.issy().expect("ISSY decode from HEM header");
        assert_eq!(issy, Issy::IscrLong(0x0024_28E2));
    }

    #[test]
    fn issy_accessor_returns_none_for_nm() {
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF0;
        hdr[1] = 0x00;
        hdr[2] = 0x07;
        hdr[3] = 0xD0;
        hdr[4] = 0xBC;
        hdr[5] = 0x00;
        hdr[6] = 0x47;
        hdr[7] = 0x00;
        hdr[8] = 0x00;
        hdr[9] = crc8(&hdr[..9]);
        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.mode, Mode::Normal);
        assert!(result.issy().is_none());
    }

    #[test]
    fn issy_accessor_falls_back_to_short_form() {
        // HEM with ISSY bytes [0x7A, 0xBC, 0x00]: bit7=0 → short form.
        // decode_issy_long fails, falls back to decode_issy_short([0x7A, 0xBC])
        // → IscrShort(0x7ABC)
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0,
            dfl: 50000,
            syncd: 100,
            mode: Mode::HighEfficiency,
            issy_in_header: Some([0x7A, 0xBC, 0x00]),
        };
        let issy = hdr.issy().expect("short-form ISSY fallback");
        assert_eq!(issy, Issy::IscrShort(0x7ABC));
    }

    #[test]
    fn parse_hem_leaves_upl_bits_as_zero_and_sync_as_zero() {
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        let result = Bbheader::parse(&hdr).unwrap();
        assert_eq!(result.upl, 0);
        assert_eq!(result.sync, 0);
    }

    #[test]
    fn parse_hem_rejects_when_mode_xor_not_0_or_1() {
        // Create a header where crc8^byte[9] gives 2 (reserved)
        let mut hdr = [0u8; BBHEADER_LEN];
        hdr[0] = 0xF0;
        hdr[1] = 0x00;
        hdr[2] = 0x00;
        hdr[3] = 0x00;
        hdr[4] = 0x00;
        hdr[5] = 0x00;
        hdr[6] = 0x00;
        hdr[7] = 0x00;
        hdr[8] = 0x00;
        hdr[9] = crc8(&hdr[..9]) ^ 0x02; // XOR with reserved value 2
        assert!(Bbheader::parse(&hdr).is_err());
    }

    #[test]
    fn parse_same_bytes_different_mode_byte_produces_different_bbheader() {
        // Two headers that differ only in byte[9] (CRC-8 MODE byte)
        let mut hdr1 = [0xF8, 0x00, 0x00, 0x00, 0xBC, 0xC8, 0x00, 0x03, 0x50, 0x00];
        hdr1[9] = crc8(&hdr1[..9]); // NM
        let mut hdr2 = hdr1;
        hdr2[9] ^= 0x01; // HEM

        let result1 = Bbheader::parse(&hdr1).unwrap();
        let result2 = Bbheader::parse(&hdr2).unwrap();
        assert_eq!(result1.mode, Mode::Normal);
        assert_eq!(result2.mode, Mode::HighEfficiency);
    }

    #[test]
    fn serialize_hem_round_trip() {
        let orig = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,  // not used in HEM
            sync: 0, // not used in HEM
            dfl: 48328,
            syncd: 848,
            mode: Mode::HighEfficiency,
            issy_in_header: Some([0xA4, 0x28, 0xE2]),
        };
        let buf = orig.serialize();
        let parsed = Bbheader::parse(&buf).unwrap();
        assert_eq!(orig.mode, parsed.mode);
        assert_eq!(orig.matype.ts_gs, parsed.matype.ts_gs);
        assert_eq!(orig.dfl, parsed.dfl);
        assert_eq!(orig.syncd, parsed.syncd);
        assert_eq!(orig.issy_in_header, parsed.issy_in_header);
    }

    #[test]
    fn serialize_hem_sets_crc_xor_mode_byte_correctly() {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0x05,
            },
            upl: 0,
            sync: 0,
            dfl: 48000,
            syncd: 0,
            mode: Mode::HighEfficiency,
            issy_in_header: Some([0x00, 0x00, 0x00]),
        };
        let buf = hdr.serialize();
        let computed = crc8(&buf[..9]);
        // MODE=1: stored = computed XOR 1
        assert_eq!(buf[9], computed ^ 1);
    }

    #[test]
    fn serialize_hem_with_issy_bytes_zero_writes_expected_layout() {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0,
            dfl: 50000,
            syncd: 100,
            mode: Mode::HighEfficiency,
            issy_in_header: Some([0x00, 0x00, 0x00]),
        };
        let buf = hdr.serialize();
        let parsed = Bbheader::parse(&buf).unwrap();
        assert_eq!(parsed.mode, Mode::HighEfficiency);
        assert_eq!(parsed.issy_in_header, Some([0x00, 0x00, 0x00]));
        assert_eq!(parsed.dfl, 50000);
        assert_eq!(parsed.syncd, 100);
    }

    #[test]
    fn parse_valid_dvbt2_hem_bbframe_rai() {
        // Real DVB-T2 BBFRAME header from Rai T2-MI (12606V, ISI 5, PLP 0).
        let hdr: [u8; BBHEADER_LEN] = [0xf8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50, 0x1f];
        assert_eq!(Bbheader::parse(&hdr).unwrap().dfl, 48328);
    }

    #[test]
    fn exhaustive_tsgs_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = TsGs::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 4, "expected 4 matched variants");
    }

    #[test]
    fn exhaustive_mode_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = Mode::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 2, "expected 2 matched variants");
    }

    #[test]
    fn trait_parse_and_serialize_round_trip() {
        let orig = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Gse,
                sis: true,
                ccm: true,
                issyi: true,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0xFF,
            dfl: 32768,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let v = <Bbheader as Serialize>::to_bytes(&orig);
        let parsed = <Bbheader as Parse>::parse(&v).unwrap();
        assert_eq!(orig.matype, parsed.matype);
        assert_eq!(orig.upl, parsed.upl);
        assert_eq!(orig.sync, parsed.sync);
        assert_eq!(orig.dfl, parsed.dfl);
        assert_eq!(orig.syncd, parsed.syncd);
        assert_eq!(orig.mode, parsed.mode);
    }

    #[test]
    fn serialize_into_hem_no_issy_is_deterministic_regardless_of_buffer_content() {
        // BUG 1 regression: HEM + issy_in_header=None leaves buf[2], buf[3], buf[6]
        // untouched. A pre-filled (0xFF) buffer must produce the same bytes as
        // to_bytes() which zero-inits.
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0,
            dfl: 48000,
            syncd: 256,
            mode: Mode::HighEfficiency,
            issy_in_header: None,
        };

        // Reference: to_bytes() zero-inits so those bytes come out 0.
        let clean = <Bbheader as Serialize>::to_bytes(&hdr);

        // Dirty buffer pre-filled with 0xFF — buf[2], buf[3], buf[6] would keep
        // 0xFF if the else-branch doesn't explicitly zero them.
        let mut dirty = [0xFFu8; BBHEADER_LEN];
        hdr.serialize_into(&mut dirty).unwrap();

        assert_eq!(
            clean.as_slice(),
            dirty.as_slice(),
            "serialize_into into dirty buffer must produce identical bytes to to_bytes()"
        );

        // Also verify re-parsing succeeds with correct fields.
        // In HEM the parser always fills issy_in_header — zeros here because we
        // set None (no ISSY) in the struct, which serialises the ISSY bytes as 0.
        let parsed = Bbheader::parse(&dirty).unwrap();
        assert_eq!(parsed.mode, Mode::HighEfficiency);
        assert_eq!(parsed.issy_in_header, Some([0, 0, 0]));
        assert_eq!(parsed.dfl, 48000);
        assert_eq!(parsed.syncd, 256);
    }

    #[test]
    fn serialize_into_rejects_buffer_too_small() {
        let hdr = Bbheader {
            matype: Matype {
                ts_gs: TsGs::Ts,
                sis: true,
                ccm: true,
                issyi: false,
                npd: false,
                ext: 0,
                isi: 0x00,
            },
            upl: 0,
            sync: 0x47,
            dfl: 0,
            syncd: 0,
            mode: Mode::Normal,
            issy_in_header: None,
        };
        let mut small = [0u8; BBHEADER_LEN - 1];
        let err = hdr.serialize_into(&mut small).unwrap_err();
        assert_eq!(
            err,
            Error::OutputBufferTooSmall {
                need: BBHEADER_LEN,
                have: small.len(),
            }
        );
    }
}
