//! `apdu_tag` — the 3-byte ASN.1 application-object tag — ETSI EN 50221 §8.8.2,
//! Table 58 + Figure 16 (PDF pp. 56-57).
//!
//! Every public `apdu_tag` is coded on three bytes beginning `0x9F`: the second
//! byte has its MSB set, the third byte has its MSB clear. We carry the tag as a
//! 24-bit value in the low three bytes of a `u32` and provide named constants
//! for every Table 58 entry (the named-constant policy: no magic tag bytes).

/// The fixed first byte of every public `apdu_tag` (Figure 16).
pub const APDU_TAG_PREFIX: u8 = 0x9F;

/// A 3-byte `apdu_tag`, held as a 24-bit value in the low three bytes of a `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ApduTag(u32);

impl ApduTag {
    /// Build a tag from its three wire bytes (b1 must be `0x9F` for a valid
    /// public tag, but any value is accepted so private tags round-trip).
    #[must_use]
    pub const fn from_bytes(b1: u8, b2: u8, b3: u8) -> Self {
        Self(((b1 as u32) << 16) | ((b2 as u32) << 8) | b3 as u32)
    }

    /// Build a tag from a raw 24-bit value (the low three bytes of `v`).
    #[must_use]
    pub const fn from_u24(v: u32) -> Self {
        Self(v & 0x00FF_FFFF)
    }

    /// The tag as a 24-bit integer (low three bytes of the returned `u32`).
    #[must_use]
    pub const fn as_u24(self) -> u32 {
        self.0
    }

    /// The three wire bytes, MSB first.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 3] {
        [(self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8]
    }

    /// Diagnostic name from Table 58, or `"unknown"` for an unallocated tag.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PROFILE_ENQ => "profile_enq",
            PROFILE => "profile",
            PROFILE_CHANGE => "profile_change",
            APPLICATION_INFO_ENQ => "application_info_enq",
            APPLICATION_INFO => "application_info",
            ENTER_MENU => "enter_menu",
            CA_INFO_ENQ => "ca_info_enq",
            CA_INFO => "ca_info",
            CA_PMT => "ca_pmt",
            CA_PMT_REPLY => "ca_pmt_reply",
            DATE_TIME_ENQ => "date_time_enq",
            DATE_TIME => "date_time",
            CLOSE_MMI => "close_mmi",
            TUNE => "tune",
            REPLACE => "replace",
            CLEAR_REPLACE => "clear_replace",
            ASK_RELEASE => "ask_release",
            DISPLAY_CONTROL => "display_control",
            DISPLAY_REPLY => "display_reply",
            TEXT_LAST => "text_last",
            TEXT_MORE => "text_more",
            KEYPAD_CONTROL => "keypad_control",
            KEYPRESS => "keypress",
            ENQ => "enq",
            ANSW => "answ",
            MENU_LAST => "menu_last",
            MENU_MORE => "menu_more",
            MENU_ANSW => "menu_answ",
            LIST_LAST => "list_last",
            LIST_MORE => "list_more",
            SUBTITLE_SEGMENT_LAST => "subtitle_segment_last",
            SUBTITLE_SEGMENT_MORE => "subtitle_segment_more",
            DISPLAY_MESSAGE => "display_message",
            SCENE_END_MARK => "scene_end_mark",
            SCENE_DONE => "scene_done",
            SCENE_CONTROL => "scene_control",
            SUBTITLE_DOWNLOAD_LAST => "subtitle_download_last",
            SUBTITLE_DOWNLOAD_MORE => "subtitle_download_more",
            FLUSH_DOWNLOAD => "flush_download",
            DOWNLOAD_REPLY => "download_reply",
            COMMS_CMD => "comms_cmd",
            CONNECTION_DESCRIPTOR => "connection_descriptor",
            COMMS_REPLY => "comms_reply",
            COMMS_SEND_LAST => "comms_send_last",
            COMMS_SEND_MORE => "comms_send_more",
            COMMS_RCV_LAST => "comms_rcv_last",
            COMMS_RCV_MORE => "comms_rcv_more",
            _ => "unknown",
        }
    }
}

impl core::fmt::Display for ApduTag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [a, b, c] = self.to_bytes();
        match self.name() {
            "unknown" => write!(f, "apdu_tag(0x{a:02X}{b:02X}{c:02X})"),
            n => write!(f, "{n}(0x{a:02X}{b:02X}{c:02X})"),
        }
    }
}

// --- Resource Manager (Table 58) ---
/// `Tprofile_enq` = `9F 80 10`.
pub const PROFILE_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x10);
/// `Tprofile` (profile reply) = `9F 80 11`.
pub const PROFILE: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x11);
/// `Tprofile_change` = `9F 80 12`.
pub const PROFILE_CHANGE: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x12);

// --- Application Information ---
/// `Tapplication_info_enq` = `9F 80 20`.
pub const APPLICATION_INFO_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x20);
/// `Tapplication_info` = `9F 80 21`.
pub const APPLICATION_INFO: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x21);
/// `Tenter_menu` = `9F 80 22`.
pub const ENTER_MENU: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x22);

// --- CA Support ---
/// `Tca_info_enq` = `9F 80 30`.
pub const CA_INFO_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x30);
/// `Tca_info` = `9F 80 31`.
pub const CA_INFO: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x31);
/// `Tca_pmt` = `9F 80 32`.
pub const CA_PMT: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x32);
/// `Tca_pmt_reply` = `9F 80 33`.
pub const CA_PMT_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x33);

// --- Date-Time ---
/// `Tdate_time_enq` = `9F 84 40`.
pub const DATE_TIME_ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x40);
/// `Tdate_time` = `9F 84 41`.
pub const DATE_TIME: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x41);

// --- Host Control (Table 58) ---
/// `Ttune` = `9F 84 00`.
pub const TUNE: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x00);
/// `Treplace` = `9F 84 01`.
pub const REPLACE: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x01);
/// `Tclear_replace` = `9F 84 02`.
pub const CLEAR_REPLACE: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x02);
/// `Task_release` = `9F 84 03`.
pub const ASK_RELEASE: ApduTag = ApduTag::from_bytes(0x9F, 0x84, 0x03);

// --- MMI ---
/// `Tclose_mmi` = `9F 88 00`.
pub const CLOSE_MMI: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x00);
/// `Tdisplay_control` = `9F 88 01`.
pub const DISPLAY_CONTROL: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x01);
/// `Tdisplay_reply` = `9F 88 02`.
pub const DISPLAY_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x02);
/// `Ttext-last` = `9F 88 03`.
pub const TEXT_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x03);
/// `Ttext-more` = `9F 88 04`.
pub const TEXT_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x04);
/// `Tkeypad_control` = `9F 88 05`.
pub const KEYPAD_CONTROL: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x05);
/// `Tkeypress` = `9F 88 06`.
pub const KEYPRESS: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x06);
/// `Tenq` = `9F 88 07`.
pub const ENQ: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x07);
/// `Tansw` = `9F 88 08`.
pub const ANSW: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x08);
/// `Tmenu_last` = `9F 88 09`.
pub const MENU_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x09);
/// `Tmenu_more` = `9F 88 0A`.
pub const MENU_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0A);
/// `Tmenu_answ` = `9F 88 0B`.
pub const MENU_ANSW: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0B);
/// `Tlist_last` = `9F 88 0C`.
pub const LIST_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0C);
/// `Tlist_more` = `9F 88 0D`.
pub const LIST_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0D);
/// `Tsubtitle_segment_last` = `9F 88 0E`.
pub const SUBTITLE_SEGMENT_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0E);
/// `Tsubtitle_segment_more` = `9F 88 0F`.
pub const SUBTITLE_SEGMENT_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x0F);
/// `Tdisplay_message` = `9F 88 10`.
pub const DISPLAY_MESSAGE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x10);
/// `Tscene_end_mark` = `9F 88 11`.
pub const SCENE_END_MARK: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x11);
/// `Tscene_done` = `9F 88 12`.
pub const SCENE_DONE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x12);
/// `Tscene_control` = `9F 88 13`.
pub const SCENE_CONTROL: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x13);
/// `Tsubtitle_download_last` = `9F 88 14`.
pub const SUBTITLE_DOWNLOAD_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x14);
/// `Tsubtitle_download_more` = `9F 88 15`.
pub const SUBTITLE_DOWNLOAD_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x15);
/// `Tflush_download` = `9F 88 16`.
pub const FLUSH_DOWNLOAD: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x16);
/// `Tdownload_reply` = `9F 88 17`.
pub const DOWNLOAD_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x88, 0x17);

// --- Low-Speed Communications (Table 58) ---
/// `Tcomms_cmd` = `9F 8C 00`.
pub const COMMS_CMD: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x00);
/// `Tconnection_descriptor` = `9F 8C 01`.
pub const CONNECTION_DESCRIPTOR: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x01);
/// `Tcomms_reply` = `9F 8C 02`.
pub const COMMS_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x02);
/// `Tcomms_send_last` = `9F 8C 03`.
pub const COMMS_SEND_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x03);
/// `Tcomms_send_more` = `9F 8C 04`.
pub const COMMS_SEND_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x04);
/// `Tcomms_rcv_last` = `9F 8C 05`.
pub const COMMS_RCV_LAST: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x05);
/// `Tcomms_rcv_more` = `9F 8C 06`.
pub const COMMS_RCV_MORE: ApduTag = ApduTag::from_bytes(0x9F, 0x8C, 0x06);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn byte_round_trip() {
        let t = ApduTag::from_bytes(0x9F, 0x80, 0x32);
        assert_eq!(t.to_bytes(), [0x9F, 0x80, 0x32]);
        assert_eq!(t.as_u24(), 0x009F_8032);
        assert_eq!(ApduTag::from_u24(0x9F_8032), t);
        assert_eq!(t, CA_PMT);
    }

    #[test]
    fn names_match_table_58() {
        assert_eq!(CA_PMT.name(), "ca_pmt");
        assert_eq!(CA_PMT_REPLY.name(), "ca_pmt_reply");
        assert_eq!(PROFILE_ENQ.name(), "profile_enq");
        assert_eq!(DATE_TIME.name(), "date_time");
        assert_eq!(ApduTag::from_bytes(0x9F, 0x99, 0x99).name(), "unknown");
    }

    #[test]
    fn display_is_lossless() {
        assert_eq!(format!("{CA_PMT}"), "ca_pmt(0x9F8032)");
        assert_eq!(
            format!("{}", ApduTag::from_bytes(0x9F, 0x99, 0x99)),
            "apdu_tag(0x9F9999)"
        );
    }
}
