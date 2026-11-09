//! Visual sample-entry extension boxes — ISO/IEC 14496-12:2026 §12.1.4–5.
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
use broadcast_common::error::InvalidValue;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// PixelAspectRatioBox — pasp (§12.1.4.2)
// ---------------------------------------------------------------------------

/// Pixel Aspect Ratio Box (`pasp`) — ISO/IEC 14496-12:2026 §12.1.4.2.
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
        need_in(bytes, 8, "pasp body")?;
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
        need_out(buf, 8)?;
        buf[..4].copy_from_slice(&self.h_spacing.to_be_bytes());
        buf[4..8].copy_from_slice(&self.v_spacing.to_be_bytes());
        Ok(8)
    }
}

// ---------------------------------------------------------------------------
// CleanApertureBox — clap (§12.1.4.2)
// ---------------------------------------------------------------------------

/// Clean Aperture Box (`clap`) — ISO/IEC 14496-12:2026 §12.1.4.2.
///
/// The clean aperture width, height and centre offsets as fractions N/D; all
/// eight fields are `signed int(32)`. For `horizOff` and `vertOff` D shall be
/// strictly positive and N may be negative; for the width and height both N and
/// D shall be strictly positive. Parsing keeps whatever is on the wire (it
/// round-trips); [`Self::aperture_within`] is where the constraints apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CleanApertureBox {
    pub clean_aperture_width_n: i32,
    pub clean_aperture_width_d: i32,
    pub clean_aperture_height_n: i32,
    pub clean_aperture_height_d: i32,
    pub horiz_off_n: i32,
    pub horiz_off_d: i32,
    pub vert_off_n: i32,
    pub vert_off_d: i32,
}

/// The `/ 2` of `pcX = horizOff + (width - 1)/2` and the clean aperture edges
/// `pcX ± (cleanApertureWidth - 1)/2` (ISO/IEC 14496-12:2026 §12.1.4.1).
const HALF: i128 = 2;

/// One axis of [`CleanApertureBox::aperture_within`]: the first sample and the
/// length of the aperture inside a frame of `frame` samples.
///
/// The aperture's first sample is `pc - (size - 1)/2` with
/// `pc = off + (frame - 1)/2`, which is `off + (frame - size)/2`; the `- 1`
/// terms cancel. `None` when a constraint of the box is broken, when the size
/// or the first sample is not a whole number of samples, or when the aperture
/// leaves the frame.
fn aperture_axis(size: (i32, i32), offset: (i32, i32), frame: u32) -> Option<(u32, u32)> {
    let (size_n, size_d) = (i128::from(size.0), i128::from(size.1));
    let (off_n, off_d) = (i128::from(offset.0), i128::from(offset.1));
    if size_n <= 0 || size_d <= 0 || off_d <= 0 || size_n % size_d != 0 {
        return None;
    }
    let size = size_n / size_d;
    let frame = i128::from(frame);
    let first_num = HALF * off_n + off_d * (frame - size);
    let first_den = HALF * off_d;
    if first_num % first_den != 0 {
        return None;
    }
    let first = first_num / first_den;
    if first < 0 || first + size > frame {
        return None;
    }
    Some((u32::try_from(first).ok()?, u32::try_from(size).ok()?))
}

impl CleanApertureBox {
    /// The aperture as `(left, top, width, height)` in samples of a decoded
    /// picture `frame_width` x `frame_height` (the sample entry's `width` and
    /// `height`, §12.1.4.1), or `None` when it cannot be a rectangle of whole
    /// samples inside that picture: a constraint of the box is broken
    /// (non-positive width, height or denominator), the width, height or an
    /// edge is fractional (centre/offset arithmetic gives half samples for an
    /// odd width), or the aperture leaves the picture.
    pub fn aperture_within(
        &self,
        frame_width: u32,
        frame_height: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        let (left, width) = aperture_axis(
            (self.clean_aperture_width_n, self.clean_aperture_width_d),
            (self.horiz_off_n, self.horiz_off_d),
            frame_width,
        )?;
        let (top, height) = aperture_axis(
            (self.clean_aperture_height_n, self.clean_aperture_height_d),
            (self.vert_off_n, self.vert_off_d),
            frame_height,
        )?;
        Some((left, top, width, height))
    }
}

impl<'a> Parse<'a> for CleanApertureBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, 32, "clap body")?;
        let field = |at: usize| {
            i32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        Ok(Self {
            clean_aperture_width_n: field(0),
            clean_aperture_width_d: field(4),
            clean_aperture_height_n: field(8),
            clean_aperture_height_d: field(12),
            horiz_off_n: field(16),
            horiz_off_d: field(20),
            vert_off_n: field(24),
            vert_off_d: field(28),
        })
    }
}

impl Serialize for CleanApertureBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        32
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, 32)?;
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
#[non_exhaustive]
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

/// Length of the `nclx` body after the 4-byte `colour_type` — ISO/IEC
/// 14496-12:2026 §12.1.5.2: `colour_primaries(16)`,
/// `transfer_characteristics(16)`, `matrix_coefficients(16)`,
/// `full_range_flag(1)` + `reserved(7)`.
const NCLX_PARAMS_LEN: usize = 7;

/// On-screen colour parameters for colour_type 'nclx' — ISO/IEC 14496-12:2026 §12.1.5.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NclxColourInfo {
    pub colour_primaries: u16,
    pub transfer_characteristics: u16,
    pub matrix_coefficients: u16,
    pub full_range_flag: bool,
    /// The 7 `reserved` bits of the final byte, kept verbatim. The spec says
    /// they are zero, but preserving them keeps a `colr` byte-identical on a
    /// round trip instead of silently rewriting a sloppy writer's padding.
    pub reserved: u8,
}

/// Colour Information Box (`colr`) — ISO/IEC 14496-12:2026 §12.1.5.2.
///
/// Carries colour type info: `nclx` (on-screen), `rICC` (restricted ICC),
/// `prof` (unrestricted ICC), or unknown (opaque payload).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ColourInformationBox {
    pub colour_type: [u8; 4],
    pub nclx: Option<NclxColourInfo>,
    /// Every byte after the type-specific body. For `rICC`/`prof`/unknown that
    /// is the ICC profile or opaque payload; for `nclx` it is trailing padding
    /// some writers emit past the 7 spec'd bytes (`NCLX_PARAMS_LEN`), kept so
    /// the box round-trips byte-identically.
    pub icc_profile: Vec<u8>,
}

impl<'a> Parse<'a> for ColourInformationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, 4, "colr body")?;
        let colour_type = [bytes[0], bytes[1], bytes[2], bytes[3]];
        let rest = &bytes[4..];

        match &colour_type {
            b"nclx" => {
                need_in(rest, NCLX_PARAMS_LEN, "colr nclx params")?;
                let colour_primaries = u16::from_be_bytes([rest[0], rest[1]]);
                let transfer_characteristics = u16::from_be_bytes([rest[2], rest[3]]);
                let matrix_coefficients = u16::from_be_bytes([rest[4], rest[5]]);
                let full_range_flag = (rest[6] >> 7) != 0;
                let reserved = rest[6] & 0x7F;
                Ok(Self {
                    colour_type,
                    nclx: Some(NclxColourInfo {
                        colour_primaries,
                        transfer_characteristics,
                        matrix_coefficients,
                        full_range_flag,
                        reserved,
                    }),
                    // Some writers pad an `nclx` body past the 7 spec'd bytes.
                    // Keep them verbatim so the box re-serializes byte-identically
                    // (r04-W39); the spec defines no other field here.
                    icc_profile: rest[NCLX_PARAMS_LEN..].to_vec(),
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
        // 4-byte `colour_type`, then either the 7 spec'd `nclx` bytes plus any
        // trailing pad, or the opaque payload.
        match &self.colour_type {
            b"nclx" => 4 + NCLX_PARAMS_LEN + self.icc_profile.len(),
            _ => 4 + self.icc_profile.len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&self.colour_type);
        c += 4;
        if &self.colour_type == b"nclx" {
            // `serialized_len` always budgets the 7 `nclx` bytes for a `nclx`
            // colour type, so a missing `nclx` must be an error — otherwise the
            // box is written 7 bytes short of its declared length (r04-W39).
            let Some(nclx) = &self.nclx else {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "colr nclx",
                    value: 0,
                    reason: "colour_type is nclx but no nclx params are set",
                }));
            };
            buf[c..c + 2].copy_from_slice(&nclx.colour_primaries.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&nclx.transfer_characteristics.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&nclx.matrix_coefficients.to_be_bytes());
            c += 2;
            buf[c] = ((nclx.full_range_flag as u8) << 7)
                | broadcast_common::len::fit_mask(nclx.reserved, 0x7F, "colr.nclx.reserved")?;
            c += 1;
        }
        // Trailing payload: the pad after an `nclx` body, or an ICC/opaque one.
        buf[c..c + self.icc_profile.len()].copy_from_slice(&self.icc_profile);
        c += self.icc_profile.len();
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use broadcast_common::error::InvalidValue;

    use super::*;
    use broadcast_common::Serialize;

    // ---- pasp ------------------------------------------------------------

    #[test]
    fn pasp_round_trip() {
        let body = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]; // 1:1
        let pasp = PixelAspectRatioBox::parse(&body).unwrap();
        assert_eq!(pasp.h_spacing, 1);
        assert_eq!(pasp.v_spacing, 1);

        let bytes = pasp.try_to_bytes().unwrap();
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
        assert_ne!(pasp.try_to_bytes().unwrap(), pasp2.try_to_bytes().unwrap());
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

        let bytes = clap.try_to_bytes().unwrap();
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
        assert_ne!(clap.try_to_bytes().unwrap(), clap2.try_to_bytes().unwrap());
    }

    #[test]
    fn clap_fields_are_signed() {
        let body = [
            0x00, 0x00, 0x07, 0x70, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x04, 0x30, 0x00, 0x00,
            0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFB, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFE,
            0x00, 0x00, 0x00, 0x01,
        ];
        let clap = CleanApertureBox::parse(&body).unwrap();
        assert_eq!(clap.horiz_off_n, -5);
        assert_eq!(clap.vert_off_n, -2);
        assert_eq!(clap.try_to_bytes().unwrap(), body);
        assert_eq!(clap.aperture_within(1920, 1080), Some((3, 2, 1904, 1072)));
    }

    #[test]
    fn clap_aperture_arithmetic() {
        let clap = |w: (i32, i32), h: (i32, i32), x: (i32, i32), y: (i32, i32)| CleanApertureBox {
            clean_aperture_width_n: w.0,
            clean_aperture_width_d: w.1,
            clean_aperture_height_n: h.0,
            clean_aperture_height_d: h.1,
            horiz_off_n: x.0,
            horiz_off_d: x.1,
            vert_off_n: y.0,
            vert_off_d: y.1,
        };
        let centred = clap((100, 1), (50, 1), (0, 1), (0, 1));
        assert_eq!(centred.aperture_within(100, 50), Some((0, 0, 100, 50)));
        assert_eq!(centred.aperture_within(200, 100), Some((50, 25, 100, 50)));
        assert_eq!(
            centred.aperture_within(99, 50),
            None,
            "wider than the frame"
        );
        assert_eq!(
            clap((101, 1), (50, 1), (0, 1), (0, 1)).aperture_within(100, 50),
            None
        );
        assert_eq!(
            clap((99, 1), (50, 1), (0, 1), (0, 1)).aperture_within(100, 50),
            None,
            "half-sample edge"
        );
        assert_eq!(
            clap((100, 1), (50, 1), (i32::MIN, 1), (0, 1)).aperture_within(100, 50),
            None
        );
        assert_eq!(
            clap((i32::MAX, 1), (50, 1), (0, 1), (0, 1)).aperture_within(100, 50),
            None
        );
        assert_eq!(
            clap((100, 1), (50, 1), (i32::MAX, i32::MAX), (0, 1)).aperture_within(200, 50),
            Some((51, 0, 100, 50)),
            "offset N/D = 1 with extreme operands does not overflow"
        );
    }

    // ---- colr (nclx) -----------------------------------------------------

    /// Oracle: colr body from colr-hdr.mp4 (ffmpeg-generated BT.2020 HDR)
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

        let bytes = colr.try_to_bytes().unwrap();
        assert_eq!(&bytes, &COLR_NCLX_ORACLE);
    }

    #[test]
    fn colr_nclx_mutate_proves_no_passthrough() {
        let colr = ColourInformationBox::parse(&COLR_NCLX_ORACLE).unwrap();
        let mut colr2 = colr.clone();
        colr2.nclx.as_mut().unwrap().matrix_coefficients = 1;
        assert_ne!(colr.try_to_bytes().unwrap(), colr2.try_to_bytes().unwrap());
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
        assert_eq!(colr.try_to_bytes().unwrap(), &body);
    }

    // ---- colr (opaque profiles) ------------------------------------------

    #[test]
    fn colr_ricc_opaque_round_trip() {
        let body = [b'r', b'I', b'C', b'C', 1, 2, 3, 4, 5];
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert_eq!(colr.icc_profile, &[1, 2, 3, 4, 5]);
        assert_eq!(colr.try_to_bytes().unwrap(), &body);
    }

    #[test]
    fn colr_prof_opaque_round_trip() {
        let body = [b'p', b'r', b'o', b'f', 0xAA, 0xBB];
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert_eq!(colr.icc_profile, &[0xAA, 0xBB]);
        assert_eq!(colr.try_to_bytes().unwrap(), &body);
    }

    /// r04-W39: the 7 `reserved` bits of the `nclx` flag byte are preserved, so
    /// a `colr` written with non-zero padding round-trips byte-identically
    /// instead of being silently rewritten.
    #[test]
    fn colr_nclx_reserved_bits_are_preserved() {
        let body = [
            0x6E, 0x63, 0x6C, 0x78, // 'nclx'
            0x00, 0x01, // bt709 primaries
            0x00, 0x01, // bt709 transfer
            0x00, 0x01, // bt709 matrix
            0xBF, // full_range_flag=1, reserved=0x3F (non-zero padding)
        ];
        let colr = ColourInformationBox::parse(&body).unwrap();
        let nclx = colr.nclx.as_ref().unwrap();
        assert!(nclx.full_range_flag);
        assert_eq!(nclx.reserved, 0x3F, "reserved bits kept, not zeroed");
        assert_eq!(
            colr.try_to_bytes().unwrap(),
            body,
            "reserved bits must round-trip"
        );

        // The spec's zero form still writes zero.
        let zero = [
            0x6E, 0x63, 0x6C, 0x78, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x80,
        ];
        let z = ColourInformationBox::parse(&zero).unwrap();
        assert_eq!(z.nclx.as_ref().unwrap().reserved, 0);
        assert_eq!(z.try_to_bytes().unwrap(), zero);
    }

    /// r04-W39: a `nclx` body with trailing pad bytes must keep them, so the
    /// box re-serializes byte-identically. Unfixed, the extras were dropped and
    /// the output was shorter than the input.
    #[test]
    fn colr_nclx_trailing_bytes_are_preserved() {
        let mut body = COLR_NCLX_ORACLE.to_vec();
        body.extend_from_slice(&[0xDE, 0xAD]);
        let colr = ColourInformationBox::parse(&body).unwrap();
        assert_eq!(colr.colour_type, *b"nclx");
        assert_eq!(
            colr.icc_profile.as_slice(),
            &[0xDE, 0xAD],
            "trailing pad kept"
        );
        assert_eq!(colr.serialized_len(), body.len());
        assert_eq!(colr.try_to_bytes().unwrap(), body);
    }

    /// r04-W39: `serialized_len` always budgets the 7 `nclx` bytes for a `nclx`
    /// colour type, so serializing one with no `nclx` params must error rather
    /// than write 7 bytes short of its declared length and return 4.
    #[test]
    fn colr_nclx_without_params_errors() {
        let colr = ColourInformationBox {
            colour_type: *b"nclx",
            nclx: None,
            icc_profile: Vec::new(),
        };
        assert_eq!(colr.serialized_len(), 4 + NCLX_PARAMS_LEN);
        let mut buf = vec![0u8; colr.serialized_len()];
        let err = colr.serialize_into(&mut buf).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue(InvalidValue {
                    field: "colr nclx",
                    ..
                })
            ),
            "expected InvalidValue for the missing nclx params, got {err:?}"
        );
        // The opaque types are unaffected and still round-trip.
        let prof = ColourInformationBox {
            colour_type: *b"prof",
            nclx: None,
            icc_profile: vec![1, 2, 3],
        };
        assert_eq!(prof.serialized_len(), 7);
        assert_eq!(prof.try_to_bytes().unwrap().len(), 7);
    }
}
