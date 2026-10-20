//! AFD and bar data parsing for H.264/H.265 T.35 SEI payloads
//! (ETSI TS 101 154 Annex B, SMPTE ST 2016-1).

use crate::ir::{ActiveFormatDescription, BarData, UserDataCarriage};
use crate::nal::NalCodec;
use crate::nal::sei_messages;
use alloc::vec::Vec;

pub const DTG1_USER_IDENTIFIER: u32 = 0x4454_4731;
pub const GA94_USER_IDENTIFIER: u32 = 0x4741_3934;
pub const ITU_T_T35_COUNTRY_CODE_USA: u8 = 0xB5;
pub const ATSC_T35_PROVIDER_CODE: u16 = 0x0031;
pub const BAR_DATA_USER_DATA_TYPE_CODE: u8 = 0x06;
pub const SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35: u32 = 4;

/// Parsed picture-level user data from one access unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureUserData {
    pub afd_present: bool,
    pub afd: Option<ActiveFormatDescription>,
    pub bar_present: bool,
    pub bar_data: Option<BarData>,
}

/// Parse an `afd_data()` structure from a T.35 payload (starting at the
/// country code). Returns `Some(Some(afd))` when an AFD message is present
/// with `active_format_flag == 1`, `Some(None)` when present with the flag
/// cleared, and `None` when the payload is not a DTG1 AFD message or is too
/// short.
pub fn afd_from_t35_payload(payload: &[u8]) -> Option<Option<ActiveFormatDescription>> {
    if payload.len() < 7 {
        return None;
    }
    if payload[0] != ITU_T_T35_COUNTRY_CODE_USA {
        return None;
    }
    let provider = u16::from_be_bytes([payload[1], payload[2]]);
    if provider != ATSC_T35_PROVIDER_CODE {
        return None;
    }
    let user_id = u32::from_be_bytes([payload[3], payload[4], payload[5], payload[6]]);
    if user_id != DTG1_USER_IDENTIFIER {
        return None;
    }
    let rest = &payload[7..];
    if rest.is_empty() {
        return None;
    }
    let active_format_flag = rest[0] & 0x40 != 0;
    if !active_format_flag {
        return Some(None);
    }
    if rest.len() < 2 {
        return None;
    }
    Some(Some(ActiveFormatDescription::from_code(rest[1] & 0x0F)))
}

/// Parse a `bar_data()` structure from a T.35 payload. Returns `Some` when a
/// GA94 bar-data message is present and at least one bar flag is set (with
/// enough bytes for the signalled fields); `None` otherwise.
pub fn bar_data_from_t35_payload(payload: &[u8]) -> Option<BarData> {
    if payload.len() < 9 {
        return None;
    }
    if payload[0] != ITU_T_T35_COUNTRY_CODE_USA {
        return None;
    }
    let provider = u16::from_be_bytes([payload[1], payload[2]]);
    if provider != ATSC_T35_PROVIDER_CODE {
        return None;
    }
    let user_id = u32::from_be_bytes([payload[3], payload[4], payload[5], payload[6]]);
    if user_id != GA94_USER_IDENTIFIER {
        return None;
    }
    if payload[7] != BAR_DATA_USER_DATA_TYPE_CODE {
        return None;
    }
    let flags = payload[8];
    let top = flags & 0x80 != 0;
    let bottom = flags & 0x40 != 0;
    let left = flags & 0x20 != 0;
    let right = flags & 0x10 != 0;
    if !top && !bottom && !left && !right {
        return Some(BarData {
            top_bar_end_line: None,
            bottom_bar_start_line: None,
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        });
    }
    let mut pos = 9usize;
    let mut bd = BarData {
        top_bar_end_line: None,
        bottom_bar_start_line: None,
        left_bar_end_pixel: None,
        right_bar_start_pixel: None,
    };
    if top {
        if pos + 2 > payload.len() {
            return None;
        }
        bd.top_bar_end_line = Some(u16::from_be_bytes([payload[pos], payload[pos + 1]]) & 0x3FFF);
        pos += 2;
    }
    if bottom {
        if pos + 2 > payload.len() {
            return None;
        }
        bd.bottom_bar_start_line =
            Some(u16::from_be_bytes([payload[pos], payload[pos + 1]]) & 0x3FFF);
        pos += 2;
    }
    if left {
        if pos + 2 > payload.len() {
            return None;
        }
        bd.left_bar_end_pixel = Some(u16::from_be_bytes([payload[pos], payload[pos + 1]]) & 0x3FFF);
        pos += 2;
    }
    if right {
        if pos + 2 > payload.len() {
            return None;
        }
        bd.right_bar_start_pixel =
            Some(u16::from_be_bytes([payload[pos], payload[pos + 1]]) & 0x3FFF);
    }
    Some(bd)
}

/// Parse all T.35 SEI messages in an Annex B access unit.
pub fn picture_user_data(codec: NalCodec, au: &[u8]) -> PictureUserData {
    let mut out = PictureUserData::default();
    let mut afd_seen = false;
    let mut bar_seen = false;
    let nals: Vec<&[u8]> = crate::annexb::iter_annexb_nals(au).collect();
    // If no Annex B start codes, treat whole input as one NAL.
    let nals = if nals.is_empty() {
        alloc::vec![au]
    } else {
        nals
    };
    for nal in nals {
        let sei_nal = match codec {
            NalCodec::Avc => crate::nal::nal_unit_type(codec, nal) == Some(6),
            NalCodec::Hevc => {
                let t = crate::nal::nal_unit_type(codec, nal);
                t == Some(39) // prefix only
            }
            _ => false,
        };
        if !sei_nal {
            continue;
        }
        let body = match codec {
            NalCodec::Avc => &nal[1..],
            NalCodec::Hevc => &nal[2..],
            _ => continue,
        };
        for (pt, payload) in sei_messages(body) {
            if pt != SEI_PAYLOAD_TYPE_USER_DATA_REGISTERED_ITU_T_T35 {
                continue;
            }
            if !afd_seen && let Some(afd) = afd_from_t35_payload(&payload) {
                out.afd_present = true;
                out.afd = afd;
                afd_seen = true;
            }
            if !bar_seen && let Some(bd) = bar_data_from_t35_payload(&payload) {
                // bar message present even if all flags zero; mark present
                out.bar_present = true;
                out.bar_data = Some(bd);
                bar_seen = true;
            }
        }
    }
    // If bar message had all-zero flags, we still marked present with all None.
    // Normalize: bar_present should reflect whether a bar_data message was seen.
    // (already set)
    if out.bar_present && out.bar_data.is_none() {
        out.bar_data = Some(BarData {
            top_bar_end_line: None,
            bottom_bar_start_line: None,
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        });
    }
    out
}

/// Carriage for a codec.
pub fn carriage_for(codec: NalCodec) -> UserDataCarriage {
    match codec {
        NalCodec::Avc => UserDataCarriage::AvcSei,
        NalCodec::Hevc => UserDataCarriage::HevcPrefixSei,
        _ => UserDataCarriage::Mpeg2PictureUserData,
    }
}
