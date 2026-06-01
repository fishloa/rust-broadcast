//! BBHEADER (Base-Band Header) parser and builder.
//!
//! Supports both Normal Mode (NM) and High Efficiency Mode (HEM)
//! per EN 302 755 v1.4.1 §5.1.7.

use num_enum::TryFromPrimitive;

use crate::crc::{crc8, crc8_with_init, CRC8_INIT_DVB_T2};
use crate::error::Error;

/// Total bytes in a BBHEADER.
pub const BBHEADER_LEN: usize = 10;
/// Maximum valid DFL in bits for DVB-S2 (normal FECFRAME is 64800, short is 16200).
pub const DFL_MAX_BITS: u16 = 64800;

/// Input stream format as described by the TS/GS field (MATYPE-1 bits [7:6]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
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
#[repr(u8)]
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
/// - MATYPE-1 (byte 0): TS/GS, SIS/MIS, CCM/ACM, ISSYI, NPD, EXT[1:0]
/// - MATYPE-2 (byte 1): ISI (0-255) or reserved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Bbheader {
    /// Parse a 10-byte BBHEADER, detecting NM vs HEM automatically.
    ///
    /// Mode detection per EN 302 755 §5.1.7:
    /// `mode = crc8(bytes[0..9]) ^ bytes[9]` (0 = NM, 1 = HEM).
    /// Values other than 0 or 1 return `Error::InvalidMode`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < BBHEADER_LEN {
            return Err(Error::BufferTooShort {
                need: BBHEADER_LEN,
                have: bytes.len(),
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

        // Mode detection: crc8(init=0x00) XOR stored CRC-8 = MODE field
        let computed_crc = crc8(&bytes[..9]);
        let mode_val = computed_crc ^ crc_stored;
        let mode = Mode::try_from(mode_val)?;

        let (upl, sync, issy_in_header) = match mode {
            Mode::Normal => {
                let crc_expected = computed_crc;
                if crc_expected != crc_stored {
                    // In NM, MODE=0, so CRC-8 should match directly
                    return Err(Error::Crc8Mismatch {
                        computed: crc_expected,
                        stored: crc_stored,
                    });
                }
                (u16::from_be_bytes([bytes[2], bytes[3]]), bytes[6], None)
            }
            Mode::HighEfficiency => {
                // In HEM, bytes[2..4] are ISSY_2MSB, byte[6] is ISSY_1LSB
                // UPL and SYNC are repurposed for ISSY
                let issy_in_header = Some([bytes[2], bytes[3], bytes[6]]);
                // Verify with DVB-T2 CRC init
                let computed_crc_t2 = crc8_with_init(&bytes[..9], CRC8_INIT_DVB_T2);
                if computed_crc_t2 != crc_stored {
                    return Err(Error::Crc8Mismatch {
                        computed: computed_crc_t2,
                        stored: crc_stored,
                    });
                }
                (0, 0, issy_in_header)
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

    /// Serialize the BBHEADER back to its 10-byte wire format.
    pub fn serialize(&self) -> [u8; BBHEADER_LEN] {
        let mut buf = [0u8; BBHEADER_LEN];
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
                    // byte 4-5 = DFL
                    let dfl = self.dfl.to_be_bytes();
                    buf[4] = dfl[0];
                    buf[5] = dfl[1];
                    buf[6] = issy[2];
                    // byte 7-8 = SYNCD
                    let syncd = self.syncd.to_be_bytes();
                    buf[7] = syncd[0];
                    buf[8] = syncd[1];
                } else {
                    // HEM without ISSY — zero the ISSY positions
                    let dfl = self.dfl.to_be_bytes();
                    buf[4] = dfl[0];
                    buf[5] = dfl[1];
                    let syncd = self.syncd.to_be_bytes();
                    buf[7] = syncd[0];
                    buf[8] = syncd[1];
                }
            }
        }

        // CRC-8 = crc8(bytes[0..9], init=0x00) XOR MODE
        let computed = crc8(&buf[..9]);
        buf[9] = computed ^ (self.mode as u8);

        buf
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
        // EXT = 0b11 in NM means roll-off α=0.35 for DVB-S2
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
    fn parse_valid_dvbt2_bbframe_with_init_b5() {
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
}
