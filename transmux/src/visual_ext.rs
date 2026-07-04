//! Visual sample-entry extension boxes — ISO/IEC 14496-12:2015 §12.1.4–5.
//!
//! These boxes are optional children of `VisualSampleEntry` (avc1, hvc1, etc.)
//! and carry pixel aspect ratio, clean aperture, and colour information.
//!
//! # Types
//!
//! | Box  | Four-CC | Spec section | Description |
//! |------|---------|-------------|-------------|
//! | [`PixelAspectRatioBox`] | `pasp` | §12.1.4.2 | Pixel aspect ratio (hSpacing / vSpacing) |
//! | [`CleanApertureBox`] | `clap` | §12.1.4.2 | Clean aperture dimensions + offsets |
//! | [`ColourInformationBox`] | `colr` | §12.1.5.2 | Colour info (nclx / rICC / prof) |
//!
//! All types implement [`Parse`] + [`Serialize`] (round-trip symmetric).
//! `no_std` + `alloc`.

use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// PixelAspectRatioBox — pasp (§12.1.4.2)
// ---------------------------------------------------------------------------

/// Pixel Aspect Ratio Box (`pasp`) — ISO/IEC 14496-12:2015 §12.1.4.2.
///
/// Carries the relative width and height of a pixel. Only the ratio matters;
/// the units are unspecified. Both fields must be positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PixelAspectRatioBox {
    pub h_spacing: u32,
    pub v_spacing: u32,
}

impl<'a> Parse<'a> for PixelAspectRatioBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "pasp body",
            });
        }
        let h_spacing = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let v_spacing = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Ok(Self {
            h_spacing,
            v_spacing,
        })
    }
}

impl Serialize for PixelAspectRatioBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        8
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 8 {
            return Err(Error::OutputBufferTooSmall {
                need: 8,
                have: buf.len(),
            });
        }
        buf[..4].copy_from_slice(&self.h_spacing.to_be_bytes());
        buf[4..8].copy_from_slice(&self.v_spacing.to_be_bytes());
        Ok(8)
    }
}

// ---------------------------------------------------------------------------
// CleanApertureBox — clap (§12.1.4.2)
// ---------------------------------------------------------------------------

/// Clean Aperture Box (`clap`) — ISO/IEC 14496-12:2015 §12.1.4.2.
///
/// Defines the clean aperture width, height, and offset as fractions (N/D).
/// For horizOff and vertOff, D must be positive and N may be positive or negative.
/// For cleanApertureWidth and cleanApertureHeight, both N and D must be positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CleanApertureBox {
    pub clean_aperture_width_n: u32,
    pub clean_aperture_width_d: u32,
    pub clean_aperture_height_n: u32,
    pub clean_aperture_height_d: u32,
    pub horiz_off_n: u32,
    pub horiz_off_d: u32,
    pub vert_off_n: u32,
    pub vert_off_d: u32,
}

impl<'a> Parse<'a> for CleanApertureBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 32 {
            return Err(Error::BufferTooShort {
                need: 32,
                have: bytes.len(),
                what: "clap body",
            });
        }
        let clean_aperture_width_n = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let clean_aperture_width_d = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let clean_aperture_height_n =
            u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let clean_aperture_height_d =
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let horiz_off_n = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let horiz_off_d = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        let vert_off_n = u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let vert_off_d = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
        Ok(Self {
            clean_aperture_width_n,
            clean_aperture_width_d,
            clean_aperture_height_n,
            clean_aperture_height_d,
            horiz_off_n,
            horiz_off_d,
            vert_off_n,
            vert_off_d,
        })
    }
}

impl Serialize for CleanApertureBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        32
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 32 {
            return Err(Error::OutputBufferTooSmall {
                need: 32,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&self.clean_aperture_width_n.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.clean_aperture_width_d.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.clean_aperture_height_n.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.clean_aperture_height_d.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.horiz_off_n.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.horiz_off_d.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.vert_off_n.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.vert_off_d.to_be_bytes());
        Ok(32)
    }
}

// ---------------------------------------------------------------------------
// ColourInformationBox — colr (§12.1.5.2)
// ---------------------------------------------------------------------------

/// Colour type indicator inside [`ColourInformationBox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ColourType {
    Nclx,
    RIcc,
    Prof,
    Unknown([u8; 4]),
}

impl ColourType {
    /// Spec token for this `colour_type` FourCC (`"reserved"` for an
    /// unrecognised code — issue #204 convention).
    pub fn name(&self) -> &'static str {
        match self {
            ColourType::Nclx => "nclx",
            ColourType::RIcc => "rICC",
            ColourType::Prof => "prof",
            ColourType::Unknown(_) => "reserved",
        }
    }
}

// Hand-written rather than `impl_spec_display!` because the catch-all carries
// a 4-byte FourCC (not a single reserved byte the macro's `Reserved(u8)` form
// can format) — still losslessly renders the raw code, per the same
// convention.
impl core::fmt::Display for ColourType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ColourType::Unknown(code) => {
                write!(
                    f,
                    "{}(0x{:02X}{:02X}{:02X}{:02X})",
                    self.name(),
                    code[0],
                    code[1],
                    code[2],
                    code[3]
                )
            }
            other => f.write_str(other.name()),
        }
    }
}

/// On-screen colour parameters for colour_type 'nclx' — ISO/IEC 14496-12:2015 §12.1.5.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NclxColourInfo {
    pub colour_primaries: u16,
    pub transfer_characteristics: u16,
    pub matrix_coefficients: u16,
    pub full_range_flag: bool,
}

/// Colour Information Box (`colr`) — ISO/IEC 14496-12:2015 §12.1.5.2.
///
/// Carries colour type info: `nclx` (on-screen), `rICC` (restricted ICC),
/// `prof` (unrestricted ICC), or unknown (opaque payload).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ColourInformationBox {
    pub colour_type: [u8; 4],
    pub nclx: Option<NclxColourInfo>,
    pub icc_profile: Vec<u8>,
}

impl<'a> Parse<'a> for ColourInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: bytes.len(),
                what: "colr body",
            });
        }
        let colour_type = [bytes[0], bytes[1], bytes[2], bytes[3]];
        let rest = &bytes[4..];

        match &colour_type {
            b"nclx" => {
                if rest.len() < 7 {
                    return Err(Error::BufferTooShort {
                        need: 7,
                        have: rest.len(),
                        what: "colr nclx params",
                    });
                }
                let colour_primaries = u16::from_be_bytes([rest[0], rest[1]]);
                let transfer_characteristics = u16::from_be_bytes([rest[2], rest[3]]);
                let matrix_coefficients = u16::from_be_bytes([rest[4], rest[5]]);
                let full_range_flag = (rest[6] >> 7) != 0;
                Ok(Self {
                    colour_type,
                    nclx: Some(NclxColourInfo {
                        colour_primaries,
                        transfer_characteristics,
                        matrix_coefficients,
                        full_range_flag,
                    }),
                    icc_profile: Vec::new(),
                })
            }
            b"rICC" | b"prof" => Ok(Self {
                colour_type,
                nclx: None,
                icc_profile: rest.to_vec(),
            }),
            _ => Ok(Self {
                colour_type,
                nclx: None,
                icc_profile: rest.to_vec(),
            }),
        }
    }
}

impl Serialize for ColourInformationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = 4;
        match &self.colour_type {
            b"nclx" => {
                n += 7;
            }
            _ => {
                n += self.icc_profile.len();
            }
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&self.colour_type);
        c += 4;
        match &self.colour_type {
            b"nclx" => {
                if let Some(nclx) = &self.nclx {
                    buf[c..c + 2].copy_from_slice(&nclx.colour_primaries.to_be_bytes());
                    c += 2;
                    buf[c..c + 2].copy_from_slice(&nclx.transfer_characteristics.to_be_bytes());
                    c += 2;
                    buf[c..c + 2].copy_from_slice(&nclx.matrix_coefficients.to_be_bytes());
                    c += 2;
                    buf[c] = (nclx.full_range_flag as u8) << 7;
                    c += 1;
                }
            }
            _ => {
                buf[c..c + self.icc_profile.len()].copy_from_slice(&self.icc_profile);
                c += self.icc_profile.len();
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    // ---- pasp ------------------------------------------------------------

    #[test]
    fn pasp_round_trip() {
        let body = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]; // 1:1
        let pasp = PixelAspectRatioBox::parse(&body).unwrap();
        assert_eq!(pasp.h_spacing, 1);
        assert_eq!(pasp.v_spacing, 1);

        let bytes = pasp.to_bytes();
        assert_eq!(&bytes, &body);
    }

    #[test]
    fn pasp_mutate_proves_no_passthrough() {
        let body = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01];
        let pasp = PixelAspectRatioBox::parse(&body).unwrap();

        let pasp2 = PixelAspectRatioBox {
            h_spacing: 4,
            v_spacing: 3,
        };
        assert_ne!(pasp.to_bytes(), pasp2.to_bytes());
    }

    // ---- clap ------------------------------------------------------------

    #[test]
    fn clap_round_trip() {
        let body = [
            0x00, 0x00, 0x07, 0x80, // widthN = 1920
            0x00, 0x00, 0x00, 0x01, // widthD = 1
            0x00, 0x00, 0x04, 0x38, // heightN = 1080
            0x00, 0x00, 0x00, 0x01, // heightD = 1
            0x00, 0x00, 0x00, 0x00, // horizOffN = 0
            0x00, 0x00, 0x00, 0x01, // horizOffD = 1
            0x00, 0x00, 0x00, 0x00, // vertOffN = 0
            0x00, 0x00, 0x00, 0x01, // vertOffD = 1
        ];
        let clap = CleanApertureBox::parse(&body).unwrap();
        assert_eq!(clap.clean_aperture_width_n, 1920);
        assert_eq!(clap.clean_aperture_height_n, 1080);

        let bytes = clap.to_bytes();
        assert_eq!(&bytes, &body);
    }

    #[test]
    fn clap_mutate_proves_no_passthrough() {
        let body = [
            0x00, 0x00, 0x07, 0x80, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x04, 0x38, 0x00, 0x00,
            0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x01,
        ];
        let clap = CleanApertureBox::parse(&body).unwrap();

        let clap2 = CleanApertureBox {
            clean_aperture_width_n: 1280,
            ..clap
        };
        assert_ne!(clap.to_bytes(), clap2.to_bytes());
    }

    // ---- colr (nclx) -----------------------------------------------------

    /// Oracle: colr body from colr_hdr.mp4 (ffmpeg-generated BT.2020 HDR)
    const COLR_NCLX_ORACLE: [u8; 11] = [
        0x6E, 0x63, 0x6C, 0x78, // 'nclx'
        0x00, 0x02, // colour_primaries = 2 (unspecified)
        0x00, 0x02, // transfer_characteristics = 2 (unspecified)
        0x00, 0x09, // matrix_coefficients = 9 (BT.2020 non-constant)
        0x00, // full_range_flag=0, reserved=0
    ];

    #[test]
    fn colr_nclx_oracle_round_trip() {
        let colr = ColourInformationBox::parse(&COLR_NCLX_ORACLE).unwrap();
        assert_eq!(&colr.colour_type, b"nclx");

        let nclx = colr.nclx.as_ref().unwrap();
        assert_eq!(nclx.colour_primaries, 2);
        assert_eq!(nclx.transfer_characteristics, 2);
        assert_eq!(nclx.matrix_coefficients, 9);
        assert!(!nclx.full_range_flag);

        let bytes = colr.to_bytes();
        assert_eq!(&bytes, &COLR_NCLX_ORACLE);
    }

    #[test]
    fn colr_nclx_mutate_proves_no_passthrough() {
        let colr = ColourInformationBox::parse(&COLR_NCLX_ORACLE).unwrap();
        let mut colr2 = colr.clone();
        colr2.nclx.as_mut().unwrap().matrix_coefficients = 1;
        assert_ne!(colr.to_bytes(), colr2.to_bytes());
    }

    #[test]
    fn colr_nclx_full_range_round_trip() {
        let body = [
            0x6E, 0x63, 0x6C, 0x78, // 'nclx'
            0x00, 0x01, // bt709
            0x00, 0x01, // bt709
            0x00, 0x01, // bt709
            0x80, // full_range_flag=1
        ];
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert!(colr.nclx.as_ref().unwrap().full_range_flag);
        assert_eq!(colr.to_bytes(), &body);
    }

    // ---- colr (opaque profiles) ------------------------------------------

    #[test]
    fn colr_ricc_opaque_round_trip() {
        let body = [b'r', b'I', b'C', b'C', 1, 2, 3, 4, 5];
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert_eq!(colr.icc_profile, &[1, 2, 3, 4, 5]);
        assert_eq!(colr.to_bytes(), &body);
    }

    #[test]
    fn colr_prof_opaque_round_trip() {
        let body = [b'p', b'r', b'o', b'f', 0xAA, 0xBB];
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert_eq!(colr.icc_profile, &[0xAA, 0xBB]);
        assert_eq!(colr.to_bytes(), &body);
    }
}
