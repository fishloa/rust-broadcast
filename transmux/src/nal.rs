//! NAL-unit-type classification + IDR/IRAP keyframe detection for AVC/HEVC/VVC.
//!
//! One reusable helper the demuxers share instead of each hand-rolling keyframe
//! detection. Works on a single NAL unit (header already stripped of any
//! start-code prefix or length prefix) and on a whole access unit carried as an
//! **Annex B** byte stream (start code `00 00 01`) or a **4-byte length-prefixed**
//! buffer, reusing [`crate::annexb`].
//!
//! NAL header layouts and the keyframe/IRAP tables (all cited):
//!
//! - **H.264 / AVC** — ITU-T H.264 §7.3.1 (`nal_unit()`): 1-byte header,
//!   `nal_unit_type` = bits `[4:0]` of byte 0. Table 7-1: a coded slice of an
//!   **IDR** picture is type **5** (the AVC keyframe VCL type); a non-IDR coded
//!   slice is type 1.
//! - **H.265 / HEVC** — ITU-T H.265 §7.3.1.2 (`nal_unit_header()`): 2-byte
//!   header, `nal_unit_type` = bits `[6:1]` of byte 0, i.e. `(byte0 >> 1) & 0x3F`.
//!   Table 7-1: the **IRAP** VCL types are **16..=23** (BLA_W_LP … CRA_NUT,
//!   covering BLA / IDR_W_RADL=19 / IDR_N_LP=20 / CRA_NUT=21) — any of these
//!   starts a random-access point / keyframe.
//! - **H.266 / VVC** — ITU-T H.266 §7.3.1.2 (`nal_unit_header()`): 2-byte header,
//!   `nal_unit_type` = bits `[7:3]` of byte 1, i.e. `(byte1 >> 3) & 0x1F`.
//!   Table 5: the **IRAP** VCL types are **IDR_W_RADL = 7**, **IDR_N_LP = 8**,
//!   and **CRA_NUT = 9** — any of these starts a random-access point / keyframe.

use crate::annexb::{iter_annexb_nals, iter_length_prefixed_nals};

// ── AVC (ITU-T H.264 §7.3.1, Table 7-1) ─────────────────────────────────────

/// Mask for the H.264 5-bit `nal_unit_type` in header byte 0 (bits `[4:0]`).
const AVC_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 `nal_unit_type` for a coded slice of an **IDR** picture (Table 7-1).
const AVC_NAL_IDR: u8 = 5;

// ── HEVC (ITU-T H.265 §7.3.1.2, Table 7-1) ──────────────────────────────────

/// Right shift to reach the HEVC 6-bit `nal_unit_type` in header byte 0.
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
/// Mask for the HEVC 6-bit `nal_unit_type` after the shift.
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;
/// First HEVC IRAP `nal_unit_type` (`BLA_W_LP`) — Table 7-1.
const HEVC_IRAP_FIRST: u8 = 16;
/// Last HEVC IRAP `nal_unit_type` (`RSV_IRAP_VCL23`) — Table 7-1.
const HEVC_IRAP_LAST: u8 = 23;

// ── VVC (ITU-T H.266 §7.3.1.2, Table 5) ─────────────────────────────────────

/// Right shift to reach the VVC 5-bit `nal_unit_type` in header byte 1.
const VVC_NAL_TYPE_SHIFT: u8 = 3;
/// Mask for the VVC 5-bit `nal_unit_type` after the shift.
const VVC_NAL_TYPE_MASK: u8 = 0x1F;
/// VVC `IDR_W_RADL` `nal_unit_type` — Table 5.
const VVC_NAL_IDR_W_RADL: u8 = 7;
/// VVC `IDR_N_LP` `nal_unit_type` — Table 5.
const VVC_NAL_IDR_N_LP: u8 = 8;
/// VVC `CRA_NUT` `nal_unit_type` — Table 5.
const VVC_NAL_CRA: u8 = 9;

/// The video codec family a NAL unit belongs to, selecting the NAL-header layout
/// and keyframe/IRAP table used to classify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum NalCodec {
    /// H.264 / AVC — ITU-T H.264 (ISO/IEC 14496-10).
    Avc,
    /// H.265 / HEVC — ITU-T H.265 (ISO/IEC 23008-2).
    Hevc,
    /// H.266 / VVC — ITU-T H.266 (ISO/IEC 23090-3).
    Vvc,
}

impl NalCodec {
    /// The short spec token for this codec family.
    pub fn name(&self) -> &'static str {
        match self {
            NalCodec::Avc => "AVC",
            NalCodec::Hevc => "HEVC",
            NalCodec::Vvc => "VVC",
        }
    }

    /// Minimum NAL-header size in bytes: AVC has a 1-byte header, HEVC and VVC
    /// each have a 2-byte header.
    fn header_len(self) -> usize {
        match self {
            NalCodec::Avc => 1,
            NalCodec::Hevc | NalCodec::Vvc => 2,
        }
    }
}

broadcast_common::impl_spec_display!(NalCodec);

/// The raw `nal_unit_type` from a single NAL unit's header, or `None` if the
/// slice is shorter than that codec's NAL header.
///
/// `nal` must be one NAL unit with any start-code / length prefix already
/// removed (its first byte is the NAL header). Bit layout per
/// [module docs](crate::nal): AVC `byte0[4:0]`, HEVC `(byte0 >> 1) & 0x3F`,
/// VVC `(byte1 >> 3) & 0x1F`.
pub fn nal_unit_type(codec: NalCodec, nal: &[u8]) -> Option<u8> {
    if nal.len() < codec.header_len() {
        return None;
    }
    Some(match codec {
        NalCodec::Avc => nal[0] & AVC_NAL_TYPE_MASK,
        NalCodec::Hevc => (nal[0] >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK,
        NalCodec::Vvc => (nal[1] >> VVC_NAL_TYPE_SHIFT) & VVC_NAL_TYPE_MASK,
    })
}

/// Whether a single NAL unit is an IDR/IRAP VCL type (a random-access keyframe
/// slice) for `codec`.
///
/// - AVC: type 5 (IDR slice).
/// - HEVC: types 16..=23 (IRAP: BLA / IDR / CRA).
/// - VVC: types 7 (IDR_W_RADL), 8 (IDR_N_LP), 9 (CRA_NUT).
///
/// Returns `false` for a slice too short to carry a NAL header.
pub fn is_keyframe_nal(codec: NalCodec, nal: &[u8]) -> bool {
    match nal_unit_type(codec, nal) {
        None => false,
        Some(t) => match codec {
            NalCodec::Avc => t == AVC_NAL_IDR,
            NalCodec::Hevc => (HEVC_IRAP_FIRST..=HEVC_IRAP_LAST).contains(&t),
            NalCodec::Vvc => t == VVC_NAL_IDR_W_RADL || t == VVC_NAL_IDR_N_LP || t == VVC_NAL_CRA,
        },
    }
}

/// Whether an access unit contains any keyframe/IRAP VCL NAL unit for `codec`.
///
/// Iterates the NAL units of `au` — as a 4-byte **length-prefixed** buffer when
/// `length_prefixed` is `true`, otherwise as an **Annex B** byte stream — and
/// returns `true` if any is a keyframe per [`is_keyframe_nal`]. A malformed
/// length-prefixed buffer (a declared length running past the end) yields
/// `false` rather than an error, since "no detectable keyframe" is the safe
/// answer for keyframe gating.
pub fn access_unit_is_keyframe(codec: NalCodec, au: &[u8], length_prefixed: bool) -> bool {
    if length_prefixed {
        match iter_length_prefixed_nals(au) {
            Ok(nals) => nals.iter().any(|nal| is_keyframe_nal(codec, nal)),
            Err(_) => false,
        }
    } else {
        iter_annexb_nals(au).any(|nal| is_keyframe_nal(codec, nal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avc_nal_type_extraction() {
        // AVC SPS header byte: forbidden_zero(0) nal_ref_idc(11) type(00111=7).
        assert_eq!(nal_unit_type(NalCodec::Avc, &[0x67, 0x42]), Some(7));
        // AVC IDR slice: type 5 (00101), nal_ref_idc 11 → 0x65.
        assert_eq!(nal_unit_type(NalCodec::Avc, &[0x65, 0x88]), Some(5));
        assert!(is_keyframe_nal(NalCodec::Avc, &[0x65, 0x88]));
        assert!(!is_keyframe_nal(NalCodec::Avc, &[0x67, 0x42])); // SPS is not VCL keyframe
                                                                 // Non-IDR slice type 1 (0x41) is not a keyframe.
        assert!(!is_keyframe_nal(NalCodec::Avc, &[0x41, 0x9a]));
        // Empty slice → None / false.
        assert_eq!(nal_unit_type(NalCodec::Avc, &[]), None);
        assert!(!is_keyframe_nal(NalCodec::Avc, &[]));
    }

    #[test]
    fn hevc_nal_type_extraction() {
        // HEVC 2-byte header. type = (byte0 >> 1) & 0x3f.
        // SPS = 33 → byte0 = 33<<1 = 0x42.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x42, 0x01]), Some(33));
        assert!(!is_keyframe_nal(NalCodec::Hevc, &[0x42, 0x01]));
        // IDR_W_RADL = 19 → 0x26; IDR_N_LP = 20 → 0x28.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x26, 0x01]), Some(19));
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x28, 0x01]), Some(20));
        assert!(is_keyframe_nal(NalCodec::Hevc, &[0x26, 0x01]));
        assert!(is_keyframe_nal(NalCodec::Hevc, &[0x28, 0x01]));
        // TRAIL_R = 1 → 0x02 is not IRAP.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x02, 0x01]), Some(1));
        assert!(!is_keyframe_nal(NalCodec::Hevc, &[0x02, 0x01]));
        // A 1-byte slice is too short for HEVC's 2-byte header.
        assert_eq!(nal_unit_type(NalCodec::Hevc, &[0x26]), None);
    }

    #[test]
    fn vvc_nal_type_extraction() {
        // VVC 2-byte header. type = (byte1 >> 3) & 0x1f.
        // A real vvenc SPS NAL header pair is [0x00, byte1] with type SPS=15 →
        // byte1 = 15<<3 = 0x78.
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x78]), Some(15));
        assert!(!is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x78]));
        // IDR_W_RADL = 7 → byte1 = 7<<3 = 0x38; IDR_N_LP = 8 → 0x40; CRA = 9 → 0x48.
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x38]), Some(7));
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x40]), Some(8));
        assert_eq!(nal_unit_type(NalCodec::Vvc, &[0x00, 0x48]), Some(9));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x38]));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x40]));
        assert!(is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x48]));
        // TRAIL = 0 → byte1 = 0x00 is not IRAP.
        assert!(!is_keyframe_nal(NalCodec::Vvc, &[0x00, 0x00]));
    }

    #[test]
    fn access_unit_annexb_and_length_prefixed_agree() {
        // Annex B AU: SPS (67) + non-IDR slice (41) → no keyframe.
        let non_kf = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x41, 0x9a];
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &non_kf, false));
        // Annex B AU: SPS (67) + IDR slice (65) → keyframe.
        let kf = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x65, 0x88];
        assert!(access_unit_is_keyframe(NalCodec::Avc, &kf, false));

        // Length-prefixed form of the same AUs yields the same verdict.
        let lp_non_kf = crate::annexb::annexb_to_length_prefixed(&non_kf);
        let lp_kf = crate::annexb::annexb_to_length_prefixed(&kf);
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &lp_non_kf, true));
        assert!(access_unit_is_keyframe(NalCodec::Avc, &lp_kf, true));
    }

    #[test]
    fn malformed_length_prefixed_is_not_keyframe() {
        // Declares a 99-byte NAL in a 6-byte buffer.
        let lp = [0x00, 0x00, 0x00, 0x63, 0x65, 0x88];
        assert!(!access_unit_is_keyframe(NalCodec::Avc, &lp, true));
    }

    #[test]
    fn nal_codec_display() {
        assert_eq!(NalCodec::Avc.to_string(), "AVC");
        assert_eq!(NalCodec::Hevc.to_string(), "HEVC");
        assert_eq!(NalCodec::Vvc.to_string(), "VVC");
    }
}
