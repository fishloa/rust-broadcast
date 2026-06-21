//! `data_unit_id` interpretation — ETSI EN 301 775 §4.4.2, Table 3.
//!
//! The 8-bit `data_unit_id` field identifies the kind of each data unit in the
//! PES data field, for `data_identifier` in `0x10`–`0x1F` or `0x99`–`0x9B`.
//! Coded per Table 3 (the authoritative value table).
//!
//! ⚠ Table 1's parse branch routes `0x02`, `0x03`, `0xC0`, **and `0xC1`** to
//! `txt_data_field()`, but Table 3 marks `0xC1` as *reserved → discard*. The
//! spec is internally inconsistent; this crate treats Table 3 as authoritative,
//! so `0xC1` is [`DataUnitId::Reserved`] (see `docs/vbi.md`).

/// `data_unit_id` value: EBU Teletext non-subtitle data (`0x02`, Table 3).
pub const ID_EBU_TELETEXT_NON_SUBTITLE: u8 = 0x02;
/// `data_unit_id` value: EBU Teletext subtitle data (`0x03`, Table 3).
pub const ID_EBU_TELETEXT_SUBTITLE: u8 = 0x03;
/// `data_unit_id` value: Inverted Teletext (`0xC0`, Table 3).
pub const ID_INVERTED_TELETEXT: u8 = 0xC0;
/// `data_unit_id` value: VPS (`0xC3`, Table 3).
pub const ID_VPS: u8 = 0xC3;
/// `data_unit_id` value: WSS (`0xC4`, Table 3).
pub const ID_WSS: u8 = 0xC4;
/// `data_unit_id` value: Closed Captioning (`0xC5`, Table 3).
pub const ID_CLOSED_CAPTIONING: u8 = 0xC5;
/// `data_unit_id` value: monochrome 4:2:2 samples (`0xC6`, Table 3).
pub const ID_MONOCHROME_422_SAMPLES: u8 = 0xC6;
/// `data_unit_id` value: stuffing (`0xFF`, Table 3).
pub const ID_STUFFING: u8 = 0xFF;

/// A decoded `data_unit_id` (ETSI EN 301 775 §4.4.2, Table 3).
///
/// The named variants carry the typed payloads this crate decodes; everything
/// else falls into [`DataUnitId::Reserved`] or [`DataUnitId::UserDefined`],
/// preserving the raw byte. Per Table 3 the spec action for the latter two is
/// "discard".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DataUnitId {
    /// `0x02` — EBU Teletext non-subtitle data.
    EbuTeletextNonSubtitle,
    /// `0x03` — EBU Teletext subtitle data.
    EbuTeletextSubtitle,
    /// `0xC0` — Inverted Teletext.
    InvertedTeletext,
    /// `0xC3` — VPS (Video Programme System).
    Vps,
    /// `0xC4` — WSS (Wide Screen Signalling).
    Wss,
    /// `0xC5` — Closed Captioning (line 21, EIA-608 Rev A).
    ClosedCaptioning,
    /// `0xC6` — monochrome 4:2:2 luminance samples.
    Monochrome422Samples,
    /// `0xFF` — stuffing (no data field).
    Stuffing,
    /// `0x00`–`0x01`, `0x04`–`0x7F`, `0xC1`, `0xC2`, `0xC7`–`0xFE` — reserved
    /// for future use (Table 3: discard). Carries the raw `data_unit_id`.
    Reserved(u8),
    /// `0x80`–`0xBF` — user defined (Table 3: discard). Carries the raw
    /// `data_unit_id`.
    UserDefined(u8),
}

impl DataUnitId {
    /// Decode a raw 8-bit `data_unit_id` per Table 3.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            ID_EBU_TELETEXT_NON_SUBTITLE => DataUnitId::EbuTeletextNonSubtitle,
            ID_EBU_TELETEXT_SUBTITLE => DataUnitId::EbuTeletextSubtitle,
            ID_INVERTED_TELETEXT => DataUnitId::InvertedTeletext,
            ID_VPS => DataUnitId::Vps,
            ID_WSS => DataUnitId::Wss,
            ID_CLOSED_CAPTIONING => DataUnitId::ClosedCaptioning,
            ID_MONOCHROME_422_SAMPLES => DataUnitId::Monochrome422Samples,
            ID_STUFFING => DataUnitId::Stuffing,
            // 0x80–0xBF is the user-defined range; everything else not named
            // above is reserved (incl. 0xC1, 0xC2, 0xC7–0xFE).
            0x80..=0xBF => DataUnitId::UserDefined(raw),
            other => DataUnitId::Reserved(other),
        }
    }

    /// Encode back to the raw 8-bit wire value.
    pub fn to_u8(self) -> u8 {
        match self {
            DataUnitId::EbuTeletextNonSubtitle => ID_EBU_TELETEXT_NON_SUBTITLE,
            DataUnitId::EbuTeletextSubtitle => ID_EBU_TELETEXT_SUBTITLE,
            DataUnitId::InvertedTeletext => ID_INVERTED_TELETEXT,
            DataUnitId::Vps => ID_VPS,
            DataUnitId::Wss => ID_WSS,
            DataUnitId::ClosedCaptioning => ID_CLOSED_CAPTIONING,
            DataUnitId::Monochrome422Samples => ID_MONOCHROME_422_SAMPLES,
            DataUnitId::Stuffing => ID_STUFFING,
            DataUnitId::Reserved(v) => v,
            DataUnitId::UserDefined(v) => v,
        }
    }

    /// Spec label for this `data_unit_id` (Table 3).
    pub fn name(&self) -> &'static str {
        match self {
            DataUnitId::EbuTeletextNonSubtitle => "EBU Teletext non-subtitle data",
            DataUnitId::EbuTeletextSubtitle => "EBU Teletext subtitle data",
            DataUnitId::InvertedTeletext => "Inverted Teletext",
            DataUnitId::Vps => "VPS",
            DataUnitId::Wss => "WSS",
            DataUnitId::ClosedCaptioning => "Closed Captioning",
            DataUnitId::Monochrome422Samples => "monochrome 4:2:2 samples",
            DataUnitId::Stuffing => "stuffing",
            DataUnitId::Reserved(_) => "reserved",
            DataUnitId::UserDefined(_) => "user defined",
        }
    }
}

dvb_common::impl_spec_display!(DataUnitId, Reserved, UserDefined);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn all_u8_round_trip() {
        for raw in 0u16..=0xFF {
            let raw = raw as u8;
            assert_eq!(DataUnitId::from_u8(raw).to_u8(), raw, "raw={raw:#04X}");
        }
    }

    #[test]
    fn c1_is_reserved_not_teletext() {
        // ⚠ Table 1 vs Table 3 conflict: Table 3 wins — 0xC1 is reserved.
        assert_eq!(DataUnitId::from_u8(0xC1), DataUnitId::Reserved(0xC1));
        assert_eq!(DataUnitId::from_u8(0xC2), DataUnitId::Reserved(0xC2));
    }

    #[test]
    fn ranges() {
        assert_eq!(DataUnitId::from_u8(0x00), DataUnitId::Reserved(0x00));
        assert_eq!(DataUnitId::from_u8(0x7F), DataUnitId::Reserved(0x7F));
        assert_eq!(DataUnitId::from_u8(0x80), DataUnitId::UserDefined(0x80));
        assert_eq!(DataUnitId::from_u8(0xBF), DataUnitId::UserDefined(0xBF));
        assert_eq!(DataUnitId::from_u8(0xC7), DataUnitId::Reserved(0xC7));
        assert_eq!(DataUnitId::from_u8(0xFE), DataUnitId::Reserved(0xFE));
    }

    #[test]
    fn display_is_lossless_for_byte_bearing() {
        assert_eq!(DataUnitId::Reserved(0xC1).to_string(), "reserved(0xC1)");
        assert_eq!(
            DataUnitId::UserDefined(0x90).to_string(),
            "user defined(0x90)"
        );
        assert_eq!(DataUnitId::Vps.to_string(), "VPS");
    }
}
