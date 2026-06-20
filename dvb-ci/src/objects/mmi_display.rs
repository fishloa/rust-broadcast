//! Low-Level / Display / Keypad / Subtitle / Download MMI objects —
//! ETSI EN 50221 §8.6.2-§8.6.4, Tables 34-45 (PDF pp. 36-45).
//!
//! - `display_control` (`9F 88 01`, Table 34) — cmd + optional MMI_mode.
//! - `display_reply` (`9F 88 02`, Table 35) — branch-dependent reply body.
//! - `keypad_control` (`9F 88 05`, Table 36) — cmd + optional key_code list.
//! - `keypress` (`9F 88 06`, Table 37) — one key_code.
//! - `subtitle_segment` (`9F 88 0E` last / `9F 88 0F` more, Table 38).
//! - `display_message` (`9F 88 10`, Table 39) — display_message_id.
//! - `scene_end_mark` (`9F 88 11`, Table 40) — flags + scene_tag.
//! - `scene_done_message` (`9F 88 12`, Table 41) — flags + scene_tag.
//! - `scene_control` (`9F 88 13`, Table 42) — flags + scene_tag.
//! - `subtitle_download` (`9F 88 14` last / `9F 88 15` more, Table 43).
//! - `flush_download` (`9F 88 16`, Table 44) — header only.
//! - `download_reply` (`9F 88 17`, Table 45) — object_id + download_reply_id.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// `display_control_cmd` values (Table 34, p. 37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DisplayControlCmd {
    /// `01` — enter the MMI mode indicated by the MMI_mode byte.
    SetMmiMode,
    /// `02` — request the display character-table list.
    GetDisplayCharacterTableList,
    /// `03` — request the input character-table list.
    GetInputCharacterTableList,
    /// `04` — request the overlay-graphics profile.
    GetOverlayGraphicsCharacteristics,
    /// `05` — request the full-screen-graphics profile.
    GetFullScreenGraphicsCharacteristics,
    /// Any other value (reserved).
    Reserved(u8),
}

impl DisplayControlCmd {
    /// Decode a `display_control_cmd` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::SetMmiMode,
            0x02 => Self::GetDisplayCharacterTableList,
            0x03 => Self::GetInputCharacterTableList,
            0x04 => Self::GetOverlayGraphicsCharacteristics,
            0x05 => Self::GetFullScreenGraphicsCharacteristics,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::SetMmiMode => 0x01,
            Self::GetDisplayCharacterTableList => 0x02,
            Self::GetInputCharacterTableList => 0x03,
            Self::GetOverlayGraphicsCharacteristics => 0x04,
            Self::GetFullScreenGraphicsCharacteristics => 0x05,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SetMmiMode => "set_mmi_mode",
            Self::GetDisplayCharacterTableList => "get_display_character_table_list",
            Self::GetInputCharacterTableList => "get_input_character_table_list",
            Self::GetOverlayGraphicsCharacteristics => "get_overlay_graphics_characteristics",
            Self::GetFullScreenGraphicsCharacteristics => {
                "get_full-screen_graphics_characteristics"
            }
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DisplayControlCmd, Reserved);

/// `mmi_mode` values (Table 34, p. 37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MmiMode {
    /// `01` — open a high-level MMI session.
    HighLevel,
    /// `02` — open a low-level overlay-graphics session.
    LowLevelOverlayGraphics,
    /// `03` — open a low-level full-screen-graphics session.
    LowLevelFullScreenGraphics,
    /// Any other value (reserved).
    Reserved(u8),
}

impl MmiMode {
    /// Decode an `mmi_mode` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::HighLevel,
            0x02 => Self::LowLevelOverlayGraphics,
            0x03 => Self::LowLevelFullScreenGraphics,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::HighLevel => 0x01,
            Self::LowLevelOverlayGraphics => 0x02,
            Self::LowLevelFullScreenGraphics => 0x03,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::HighLevel => "high_level",
            Self::LowLevelOverlayGraphics => "low_level_overlay_graphics",
            Self::LowLevelFullScreenGraphics => "low_level_full_screen_graphics",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(MmiMode, Reserved);

/// `display_control()` object (Table 34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisplayControl {
    /// `display_control_cmd`.
    pub cmd: DisplayControlCmd,
    /// `MMI_mode` — present only when `cmd == set_mmi_mode`.
    pub mmi_mode: Option<MmiMode>,
}

impl<'a> Parse<'a> for DisplayControl {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DISPLAY_CONTROL, "display_control")?;
        let cmd_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "display_control cmd",
        })?;
        let cmd = DisplayControlCmd::from_u8(cmd_byte);
        let mmi_mode = if cmd == DisplayControlCmd::SetMmiMode {
            if body.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: body.len(),
                    what: "display_control mmi_mode",
                });
            }
            Some(MmiMode::from_u8(body[1]))
        } else {
            None
        };
        Ok(Self { cmd, mmi_mode })
    }
}

impl Serialize for DisplayControl {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1 + usize::from(self.mmi_mode.is_some()))
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + usize::from(self.mmi_mode.is_some());
        let mut pos = super::write_apdu_header(tag::DISPLAY_CONTROL, body_len, buf)?;
        buf[pos] = self.cmd.to_u8();
        pos += 1;
        if let Some(m) = self.mmi_mode {
            buf[pos] = m.to_u8();
            pos += 1;
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for DisplayControl {
    const TAG: ApduTag = tag::DISPLAY_CONTROL;
    const NAME: &'static str = "DISPLAY_CONTROL";
}

/// `display_reply_id` values (Table 35, p. 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DisplayReplyId {
    /// `01` — acknowledge of the selected MMI mode.
    MmiModeAck,
    /// `02` — list of display character tables.
    ListDisplayCharacterTables,
    /// `03` — list of input character tables.
    ListInputCharacterTables,
    /// `04` — overlay-graphics characteristics.
    ListGraphicOverlayCharacteristics,
    /// `05` — full-screen-graphics characteristics.
    ListFullScreenGraphicCharacteristics,
    /// `F0` — unknown display_control_cmd.
    UnknownDisplayControlCmd,
    /// `F1` — unknown mmi_mode.
    UnknownMmiMode,
    /// `F2` — unknown character table.
    UnknownCharacterTable,
    /// Any other value (reserved).
    Reserved(u8),
}

impl DisplayReplyId {
    /// Decode a `display_reply_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::MmiModeAck,
            0x02 => Self::ListDisplayCharacterTables,
            0x03 => Self::ListInputCharacterTables,
            0x04 => Self::ListGraphicOverlayCharacteristics,
            0x05 => Self::ListFullScreenGraphicCharacteristics,
            0xF0 => Self::UnknownDisplayControlCmd,
            0xF1 => Self::UnknownMmiMode,
            0xF2 => Self::UnknownCharacterTable,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::MmiModeAck => 0x01,
            Self::ListDisplayCharacterTables => 0x02,
            Self::ListInputCharacterTables => 0x03,
            Self::ListGraphicOverlayCharacteristics => 0x04,
            Self::ListFullScreenGraphicCharacteristics => 0x05,
            Self::UnknownDisplayControlCmd => 0xF0,
            Self::UnknownMmiMode => 0xF1,
            Self::UnknownCharacterTable => 0xF2,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MmiModeAck => "mmi_mode_ack",
            Self::ListDisplayCharacterTables => "list_display_character_tables",
            Self::ListInputCharacterTables => "list_input_character_tables",
            Self::ListGraphicOverlayCharacteristics => "list_graphic_overlay_characteristics",
            Self::ListFullScreenGraphicCharacteristics => {
                "list_full_screen_graphic_characteristics"
            }
            Self::UnknownDisplayControlCmd => "unknown display_control_cmd",
            Self::UnknownMmiMode => "unknown_mmi_mode",
            Self::UnknownCharacterTable => "unknown_character_table",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DisplayReplyId, Reserved);

/// One pixel-depth entry in a graphics-characteristics display reply (Table 35).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PixelDepth {
    /// 3-bit `display_depth` (coded as region_level_of_compatibility in `[9]`).
    pub display_depth: u8,
    /// 3-bit `pixels_per_byte` (0 = 8-bit-deep display special case).
    pub pixels_per_byte: u8,
    /// 8-bit `region_overhead` (×16 = pixel reduction per added region).
    pub region_overhead: u8,
}

/// The graphics-characteristics profile body of a [`DisplayReply`] (Table 35,
/// for `list_graphic_overlay_characteristics` / `list_full_screen_…`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GraphicsCharacteristics {
    /// 16-bit `display_horizontal_size`.
    pub display_horizontal_size: u16,
    /// 16-bit `display_vertical_size`.
    pub display_vertical_size: u16,
    /// 4-bit `aspect_ratio_information` (ISO/IEC 13818-2 coding).
    pub aspect_ratio_information: u8,
    /// 3-bit `graphics_relation_to_video`.
    pub graphics_relation_to_video: u8,
    /// `multiple_depths` flag.
    pub multiple_depths: bool,
    /// 12-bit `display_bytes` (×256 = display memory bytes).
    pub display_bytes: u16,
    /// 8-bit `composition_buffer_bytes` (×256).
    pub composition_buffer_bytes: u8,
    /// 8-bit `object_cache_bytes` (×4096).
    pub object_cache_bytes: u8,
    /// The per-pixel-depth entries (`number_pixel_depths` = `depths.len()`).
    pub depths: Vec<PixelDepth>,
}

// Fixed part of the graphics body: h(2)+v(2)+packed(1)+display/comp/obj/n(4).
const GFX_FIXED: usize = 9;

/// The branch-dependent body of a [`DisplayReply`] (Table 35).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DisplayReplyBody {
    /// `list_graphic_overlay_characteristics` / `list_full_screen_…`.
    Graphics(GraphicsCharacteristics),
    /// `list_display_character_tables` / `list_input_character_tables` — the
    /// `character_table_byte` list.
    CharacterTables(Vec<u8>),
    /// `mmi_mode_ack` — the acknowledged mode.
    MmiModeAck(MmiMode),
    /// No branch body (`unknown_*` replies and reserved ids).
    None,
}

/// `display_reply()` object (Table 35).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisplayReply {
    /// `display_reply_id`.
    pub reply_id: DisplayReplyId,
    /// The branch-dependent body keyed by `reply_id`.
    pub body: DisplayReplyBody,
}

impl DisplayReply {
    fn parse_graphics(b: &[u8]) -> Result<GraphicsCharacteristics> {
        if b.len() < GFX_FIXED {
            return Err(Error::BufferTooShort {
                need: GFX_FIXED,
                have: b.len(),
                what: "display_reply graphics",
            });
        }
        let display_horizontal_size = u16::from_be_bytes([b[0], b[1]]);
        let display_vertical_size = u16::from_be_bytes([b[2], b[3]]);
        let aspect_ratio_information = (b[4] >> 4) & 0x0F;
        let graphics_relation_to_video = (b[4] >> 1) & 0x07;
        let multiple_depths = (b[4] & 0x01) != 0;
        // display_bytes(12) | composition_buffer_bytes(8) | object_cache_bytes(8)
        // | number_pixel_depths(4), packed across b[5..9].
        let display_bytes = (((b[5] as u16) << 4) | ((b[6] >> 4) as u16)) & 0x0FFF;
        let composition_buffer_bytes = ((b[6] & 0x0F) << 4) | (b[7] >> 4);
        let object_cache_bytes = ((b[7] & 0x0F) << 4) | (b[8] >> 4);
        let number_pixel_depths = (b[8] & 0x0F) as usize;
        let mut depths = Vec::with_capacity(number_pixel_depths);
        let mut pos = GFX_FIXED;
        for _ in 0..number_pixel_depths {
            if pos + 2 > b.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: b.len(),
                    what: "display_reply pixel_depth",
                });
            }
            depths.push(PixelDepth {
                display_depth: (b[pos] >> 5) & 0x07,
                pixels_per_byte: (b[pos] >> 2) & 0x07,
                region_overhead: b[pos + 1],
            });
            pos += 2;
        }
        Ok(GraphicsCharacteristics {
            display_horizontal_size,
            display_vertical_size,
            aspect_ratio_information,
            graphics_relation_to_video,
            multiple_depths,
            display_bytes,
            composition_buffer_bytes,
            object_cache_bytes,
            depths,
        })
    }

    fn write_graphics(g: &GraphicsCharacteristics, buf: &mut [u8]) -> usize {
        buf[0..2].copy_from_slice(&g.display_horizontal_size.to_be_bytes());
        buf[2..4].copy_from_slice(&g.display_vertical_size.to_be_bytes());
        buf[4] = ((g.aspect_ratio_information & 0x0F) << 4)
            | ((g.graphics_relation_to_video & 0x07) << 1)
            | u8::from(g.multiple_depths);
        let n = g.depths.len() as u8 & 0x0F;
        buf[5] = (g.display_bytes >> 4) as u8;
        buf[6] = (((g.display_bytes & 0x0F) as u8) << 4) | (g.composition_buffer_bytes >> 4);
        buf[7] = ((g.composition_buffer_bytes & 0x0F) << 4) | (g.object_cache_bytes >> 4);
        buf[8] = ((g.object_cache_bytes & 0x0F) << 4) | n;
        let mut pos = GFX_FIXED;
        for d in &g.depths {
            buf[pos] = ((d.display_depth & 0x07) << 5) | ((d.pixels_per_byte & 0x07) << 2);
            buf[pos + 1] = d.region_overhead;
            pos += 2;
        }
        pos
    }

    fn body_len(&self) -> usize {
        1 + match &self.body {
            DisplayReplyBody::Graphics(g) => GFX_FIXED + g.depths.len() * 2,
            DisplayReplyBody::CharacterTables(t) => t.len(),
            DisplayReplyBody::MmiModeAck(_) => 1,
            DisplayReplyBody::None => 0,
        }
    }
}

impl<'a> Parse<'a> for DisplayReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DISPLAY_REPLY, "display_reply")?;
        let id_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "display_reply id",
        })?;
        let reply_id = DisplayReplyId::from_u8(id_byte);
        let rest = &body[1..];
        let body = match reply_id {
            DisplayReplyId::ListGraphicOverlayCharacteristics
            | DisplayReplyId::ListFullScreenGraphicCharacteristics => {
                DisplayReplyBody::Graphics(Self::parse_graphics(rest)?)
            }
            DisplayReplyId::ListDisplayCharacterTables
            | DisplayReplyId::ListInputCharacterTables => {
                DisplayReplyBody::CharacterTables(rest.to_vec())
            }
            DisplayReplyId::MmiModeAck => {
                let m = *rest.first().ok_or(Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "display_reply mmi_mode_ack",
                })?;
                DisplayReplyBody::MmiModeAck(MmiMode::from_u8(m))
            }
            _ => DisplayReplyBody::None,
        };
        Ok(Self { reply_id, body })
    }
}

impl Serialize for DisplayReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let mut pos = super::write_apdu_header(tag::DISPLAY_REPLY, body_len, buf)?;
        buf[pos] = self.reply_id.to_u8();
        pos += 1;
        match &self.body {
            DisplayReplyBody::Graphics(g) => {
                pos += Self::write_graphics(g, &mut buf[pos..]);
            }
            DisplayReplyBody::CharacterTables(t) => {
                buf[pos..pos + t.len()].copy_from_slice(t);
                pos += t.len();
            }
            DisplayReplyBody::MmiModeAck(m) => {
                buf[pos] = m.to_u8();
                pos += 1;
            }
            DisplayReplyBody::None => {}
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for DisplayReply {
    const TAG: ApduTag = tag::DISPLAY_REPLY;
    const NAME: &'static str = "DISPLAY_REPLY";
}

/// `keypad_control_cmd` values (Table 36, p. 41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum KeypadControlCmd {
    /// `01` — intercept all keypresses.
    InterceptAllKeypresses,
    /// `02` — ignore all keypresses.
    IgnoreAllKeypresses,
    /// `03` — intercept the listed keypresses.
    InterceptSelectedKeypress,
    /// `04` — ignore the listed keypresses.
    IgnoreSelectedKeypress,
    /// `05` — reject one keypress.
    RejectKeypress,
    /// Any other value (reserved).
    Reserved(u8),
}

impl KeypadControlCmd {
    /// Decode a `keypad_control_cmd` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::InterceptAllKeypresses,
            0x02 => Self::IgnoreAllKeypresses,
            0x03 => Self::InterceptSelectedKeypress,
            0x04 => Self::IgnoreSelectedKeypress,
            0x05 => Self::RejectKeypress,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::InterceptAllKeypresses => 0x01,
            Self::IgnoreAllKeypresses => 0x02,
            Self::InterceptSelectedKeypress => 0x03,
            Self::IgnoreSelectedKeypress => 0x04,
            Self::RejectKeypress => 0x05,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InterceptAllKeypresses => "intercept_all_keypresses",
            Self::IgnoreAllKeypresses => "ignore_all_keypresses",
            Self::InterceptSelectedKeypress => "intercept_selected_keypress",
            Self::IgnoreSelectedKeypress => "ignore_selected_keypress",
            Self::RejectKeypress => "reject_keypress",
            Self::Reserved(_) => "reserved",
        }
    }
    /// `true` for the commands that carry a `key_code` list/value.
    fn carries_key_codes(self) -> bool {
        matches!(
            self,
            Self::InterceptSelectedKeypress | Self::IgnoreSelectedKeypress | Self::RejectKeypress
        )
    }
}
dvb_common::impl_spec_display!(KeypadControlCmd, Reserved);

/// `keypad_control()` object (Table 36).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeypadControl {
    /// `keypad_control_cmd`.
    pub cmd: KeypadControlCmd,
    /// The `key_code` list — non-empty only for the select/reject commands (a
    /// single code for `reject_keypress`).
    pub key_codes: Vec<u8>,
}

impl<'a> Parse<'a> for KeypadControl {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::KEYPAD_CONTROL, "keypad_control")?;
        let cmd_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "keypad_control cmd",
        })?;
        let cmd = KeypadControlCmd::from_u8(cmd_byte);
        let key_codes = if cmd.carries_key_codes() {
            body[1..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self { cmd, key_codes })
    }
}

impl Serialize for KeypadControl {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let codes = if self.cmd.carries_key_codes() {
            self.key_codes.len()
        } else {
            0
        };
        super::apdu_len(1 + codes)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let with_codes = self.cmd.carries_key_codes();
        let codes = if with_codes { self.key_codes.len() } else { 0 };
        let mut pos = super::write_apdu_header(tag::KEYPAD_CONTROL, 1 + codes, buf)?;
        buf[pos] = self.cmd.to_u8();
        pos += 1;
        if with_codes {
            buf[pos..pos + self.key_codes.len()].copy_from_slice(&self.key_codes);
            pos += self.key_codes.len();
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for KeypadControl {
    const TAG: ApduTag = tag::KEYPAD_CONTROL;
    const NAME: &'static str = "KEYPAD_CONTROL";
}

/// `keypress()` object (Table 37): one `key_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Keypress {
    /// `key_code` (Table of key codes, p. 41).
    pub key_code: u8,
}

impl<'a> Parse<'a> for Keypress {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::KEYPRESS, "keypress")?;
        let key_code = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "keypress key_code",
        })?;
        Ok(Self { key_code })
    }
}

impl Serialize for Keypress {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::KEYPRESS, 1, buf)?;
        buf[pos] = self.key_code;
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Keypress {
    const TAG: ApduTag = tag::KEYPRESS;
    const NAME: &'static str = "KEYPRESS";
}

/// `subtitle_segment()` object (Table 38): a verbatim DVB_Subtitling_segment.
///
/// The `_last` (`9F 88 0E`) / `_more` (`9F 88 0F`) tags share this body;
/// [`SubtitleSegment::more`] selects which tag is written and is set from the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubtitleSegment<'a> {
    /// `true` = `subtitle_segment_more`; `false` = `subtitle_segment_last`.
    pub more: bool,
    /// The DVB_Subtitling_segment bytes (reference `[9]`), verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub segment: &'a [u8],
}

impl<'a> SubtitleSegment<'a> {
    /// The `apdu_tag` for this object given [`SubtitleSegment::more`].
    #[must_use]
    pub fn tag(&self) -> ApduTag {
        if self.more {
            tag::SUBTITLE_SEGMENT_MORE
        } else {
            tag::SUBTITLE_SEGMENT_LAST
        }
    }
}

impl<'a> Parse<'a> for SubtitleSegment<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "subtitle_segment tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let (expected, more) = match t {
            tag::SUBTITLE_SEGMENT_MORE => (tag::SUBTITLE_SEGMENT_MORE, true),
            _ => (tag::SUBTITLE_SEGMENT_LAST, false),
        };
        let body = super::parse_apdu_header(bytes, expected, "subtitle_segment")?;
        Ok(Self {
            more,
            segment: body,
        })
    }
}

impl Serialize for SubtitleSegment<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.segment.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(self.tag(), self.segment.len(), buf)?;
        buf[pos..pos + self.segment.len()].copy_from_slice(self.segment);
        pos += self.segment.len();
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for SubtitleSegment<'a> {
    const TAG: ApduTag = tag::SUBTITLE_SEGMENT_LAST;
    const NAME: &'static str = "SUBTITLE_SEGMENT";
}

/// `display_message_id` values (Table 39, p. 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DisplayMessageId {
    /// `00` — Display OK.
    DisplayOk,
    /// `01` — Display Error.
    DisplayError,
    /// `02` — Display out of memory.
    DisplayOutOfMemory,
    /// `03` — DVB Subtitling syntax error.
    DvbSubtitlingSyntaxError,
    /// `04` — Undefined region referenced.
    UndefinedRegionReferenced,
    /// `05` — Undefined CLUT referenced.
    UndefinedClutReferenced,
    /// `06` — Undefined object referenced.
    UndefinedObjectReferenced,
    /// `07` — Object incompatible with region.
    ObjectIncompatibleWithRegion,
    /// `08` — Unknown character referenced.
    UnknownCharacterReferenced,
    /// `09` — Display characteristics changed.
    DisplayCharacteristicsChanged,
    /// Any other value (reserved).
    Reserved(u8),
}

impl DisplayMessageId {
    /// Decode a `display_message_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::DisplayOk,
            0x01 => Self::DisplayError,
            0x02 => Self::DisplayOutOfMemory,
            0x03 => Self::DvbSubtitlingSyntaxError,
            0x04 => Self::UndefinedRegionReferenced,
            0x05 => Self::UndefinedClutReferenced,
            0x06 => Self::UndefinedObjectReferenced,
            0x07 => Self::ObjectIncompatibleWithRegion,
            0x08 => Self::UnknownCharacterReferenced,
            0x09 => Self::DisplayCharacteristicsChanged,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::DisplayOk => 0x00,
            Self::DisplayError => 0x01,
            Self::DisplayOutOfMemory => 0x02,
            Self::DvbSubtitlingSyntaxError => 0x03,
            Self::UndefinedRegionReferenced => 0x04,
            Self::UndefinedClutReferenced => 0x05,
            Self::UndefinedObjectReferenced => 0x06,
            Self::ObjectIncompatibleWithRegion => 0x07,
            Self::UnknownCharacterReferenced => 0x08,
            Self::DisplayCharacteristicsChanged => 0x09,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::DisplayOk => "Display OK",
            Self::DisplayError => "Display Error",
            Self::DisplayOutOfMemory => "Display out of memory",
            Self::DvbSubtitlingSyntaxError => "DVB Subtitling syntax error",
            Self::UndefinedRegionReferenced => "Undefined region referenced",
            Self::UndefinedClutReferenced => "Undefined CLUT referenced",
            Self::UndefinedObjectReferenced => "Undefined object referenced",
            Self::ObjectIncompatibleWithRegion => "Object incompatible with region",
            Self::UnknownCharacterReferenced => "Unknown character referenced",
            Self::DisplayCharacteristicsChanged => "Display characteristics changed",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DisplayMessageId, Reserved);

/// `display_message()` object (Table 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DisplayMessage {
    /// `display_message_id`.
    pub message_id: DisplayMessageId,
}

impl<'a> Parse<'a> for DisplayMessage {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DISPLAY_MESSAGE, "display_message")?;
        let message_id = DisplayMessageId::from_u8(*body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "display_message id",
        })?);
        Ok(Self { message_id })
    }
}

impl Serialize for DisplayMessage {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::DISPLAY_MESSAGE, 1, buf)?;
        buf[pos] = self.message_id.to_u8();
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for DisplayMessage {
    const TAG: ApduTag = tag::DISPLAY_MESSAGE;
    const NAME: &'static str = "DISPLAY_MESSAGE";
}

/// `scene_end_mark()` object (Table 40): subtitle display-set delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SceneEndMark {
    /// `decoder_continue_flag`.
    pub decoder_continue_flag: bool,
    /// `scene_reveal_flag`.
    pub scene_reveal_flag: bool,
    /// `send_scene_done` — instruct the decoder to send a scene done APDU.
    pub send_scene_done: bool,
    /// 4-bit `scene_tag`.
    pub scene_tag: u8,
}

impl<'a> Parse<'a> for SceneEndMark {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::SCENE_END_MARK, "scene_end_mark")?;
        let b = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "scene_end_mark",
        })?;
        Ok(Self {
            decoder_continue_flag: (b & 0x80) != 0,
            scene_reveal_flag: (b & 0x40) != 0,
            send_scene_done: (b & 0x20) != 0,
            // bit `[4]` reserved, scene_tag in `[3:0]`.
            scene_tag: b & 0x0F,
        })
    }
}

impl Serialize for SceneEndMark {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::SCENE_END_MARK, 1, buf)?;
        // decoder_continue(1) reveal(1) send_done(1) reserved(1)=1 scene_tag(4).
        buf[pos] = (u8::from(self.decoder_continue_flag) << 7)
            | (u8::from(self.scene_reveal_flag) << 6)
            | (u8::from(self.send_scene_done) << 5)
            | 0x10
            | (self.scene_tag & 0x0F);
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for SceneEndMark {
    const TAG: ApduTag = tag::SCENE_END_MARK;
    const NAME: &'static str = "SCENE_END_MARK";
}

/// `scene_done_message()` object (Table 41): decoder's display-set completion.
///
/// Per Table 41 the reserved field here is **2 bits** (vs. 1 bit in
/// `scene_end_mark`), keeping the body byte-aligned without `send_scene_done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SceneDoneMessage {
    /// `decoder_continue_flag` — duplicates the causing scene end mark.
    pub decoder_continue_flag: bool,
    /// `scene_reveal_flag` — duplicates the causing scene end mark.
    pub scene_reveal_flag: bool,
    /// 4-bit `scene_tag` — duplicates the causing scene end mark.
    pub scene_tag: u8,
}

impl<'a> Parse<'a> for SceneDoneMessage {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::SCENE_DONE, "scene_done_message")?;
        let b = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "scene_done_message",
        })?;
        Ok(Self {
            decoder_continue_flag: (b & 0x80) != 0,
            scene_reveal_flag: (b & 0x40) != 0,
            // bits `[5:4]` reserved, scene_tag in `[3:0]`.
            scene_tag: b & 0x0F,
        })
    }
}

impl Serialize for SceneDoneMessage {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::SCENE_DONE, 1, buf)?;
        // decoder_continue(1) reveal(1) reserved(2)=11 scene_tag(4).
        buf[pos] = (u8::from(self.decoder_continue_flag) << 7)
            | (u8::from(self.scene_reveal_flag) << 6)
            | 0x30
            | (self.scene_tag & 0x0F);
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for SceneDoneMessage {
    const TAG: ApduTag = tag::SCENE_DONE;
    const NAME: &'static str = "SCENE_DONE";
}

/// `scene_control()` object (Table 42): same layout as `scene_done_message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SceneControl {
    /// `decoder_continue_flag`.
    pub decoder_continue_flag: bool,
    /// `scene_reveal_flag`.
    pub scene_reveal_flag: bool,
    /// 4-bit `scene_tag` — the scene end mark being operated upon.
    pub scene_tag: u8,
}

impl<'a> Parse<'a> for SceneControl {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::SCENE_CONTROL, "scene_control")?;
        let b = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "scene_control",
        })?;
        Ok(Self {
            decoder_continue_flag: (b & 0x80) != 0,
            scene_reveal_flag: (b & 0x40) != 0,
            scene_tag: b & 0x0F,
        })
    }
}

impl Serialize for SceneControl {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::SCENE_CONTROL, 1, buf)?;
        // decoder_continue(1) reveal(1) reserved(2)=11 scene_tag(4).
        buf[pos] = (u8::from(self.decoder_continue_flag) << 7)
            | (u8::from(self.scene_reveal_flag) << 6)
            | 0x30
            | (self.scene_tag & 0x0F);
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for SceneControl {
    const TAG: ApduTag = tag::SCENE_CONTROL;
    const NAME: &'static str = "SCENE_CONTROL";
}

/// `subtitle_download()` object (Table 43): identical format to
/// `subtitle_segment` but constrained to object-data segments.
///
/// The `_last` (`9F 88 14`) / `_more` (`9F 88 15`) tags share this body;
/// [`SubtitleDownload::more`] selects which tag is written and is set from the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubtitleDownload<'a> {
    /// `true` = `subtitle_download_more`; `false` = `subtitle_download_last`.
    pub more: bool,
    /// The DVB_Subtitling_segment (object-data) bytes, verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub segment: &'a [u8],
}

impl<'a> SubtitleDownload<'a> {
    /// The `apdu_tag` for this object given [`SubtitleDownload::more`].
    #[must_use]
    pub fn tag(&self) -> ApduTag {
        if self.more {
            tag::SUBTITLE_DOWNLOAD_MORE
        } else {
            tag::SUBTITLE_DOWNLOAD_LAST
        }
    }
}

impl<'a> Parse<'a> for SubtitleDownload<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "subtitle_download tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let (expected, more) = match t {
            tag::SUBTITLE_DOWNLOAD_MORE => (tag::SUBTITLE_DOWNLOAD_MORE, true),
            _ => (tag::SUBTITLE_DOWNLOAD_LAST, false),
        };
        let body = super::parse_apdu_header(bytes, expected, "subtitle_download")?;
        Ok(Self {
            more,
            segment: body,
        })
    }
}

impl Serialize for SubtitleDownload<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.segment.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(self.tag(), self.segment.len(), buf)?;
        buf[pos..pos + self.segment.len()].copy_from_slice(self.segment);
        pos += self.segment.len();
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for SubtitleDownload<'a> {
    const TAG: ApduTag = tag::SUBTITLE_DOWNLOAD_LAST;
    const NAME: &'static str = "SUBTITLE_DOWNLOAD";
}

/// `flush_download()` object (Table 44): header-only cache-purge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlushDownload;

impl<'a> Parse<'a> for FlushDownload {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::FLUSH_DOWNLOAD, "flush_download")?;
        Ok(Self)
    }
}

impl Serialize for FlushDownload {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::FLUSH_DOWNLOAD, buf)
    }
}

impl<'a> ApduDef<'a> for FlushDownload {
    const TAG: ApduTag = tag::FLUSH_DOWNLOAD;
    const NAME: &'static str = "FLUSH_DOWNLOAD";
}

/// `download_reply_id` values (Table 45, p. 45).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DownloadReplyId {
    /// `00` — Download OK.
    DownloadOk,
    /// `01` — Not an object data segment (`object_id` should be `0xFFFF`).
    NotAnObjectDataSegment,
    /// `02` — Memory exhausted.
    MemoryExhausted,
    /// Any other value (reserved).
    Reserved(u8),
}

impl DownloadReplyId {
    /// Decode a `download_reply_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::DownloadOk,
            0x01 => Self::NotAnObjectDataSegment,
            0x02 => Self::MemoryExhausted,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::DownloadOk => 0x00,
            Self::NotAnObjectDataSegment => 0x01,
            Self::MemoryExhausted => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::DownloadOk => "Download OK",
            Self::NotAnObjectDataSegment => "Not an object data segment",
            Self::MemoryExhausted => "Memory exhausted",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DownloadReplyId, Reserved);

/// `download_reply()` object (Table 45): object_id + download_reply_id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DownloadReply {
    /// `object_id` — the offending object (`0xFFFF` for "not an object segment").
    pub object_id: u16,
    /// `download_reply_id`.
    pub reply_id: DownloadReplyId,
}

// object_id(2) + download_reply_id(1).
const DOWNLOAD_REPLY_BODY: usize = 3;

impl<'a> Parse<'a> for DownloadReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::DOWNLOAD_REPLY, "download_reply")?;
        if body.len() < DOWNLOAD_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: DOWNLOAD_REPLY_BODY,
                have: body.len(),
                what: "download_reply",
            });
        }
        Ok(Self {
            object_id: u16::from_be_bytes([body[0], body[1]]),
            reply_id: DownloadReplyId::from_u8(body[2]),
        })
    }
}

impl Serialize for DownloadReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(DOWNLOAD_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::DOWNLOAD_REPLY, DOWNLOAD_REPLY_BODY, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.object_id.to_be_bytes());
        buf[pos + 2] = self.reply_id.to_u8();
        pos += DOWNLOAD_REPLY_BODY;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for DownloadReply {
    const TAG: ApduTag = tag::DOWNLOAD_REPLY;
    const NAME: &'static str = "DOWNLOAD_REPLY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_control_set_mmi_mode_and_query() {
        let dc = DisplayControl {
            cmd: DisplayControlCmd::SetMmiMode,
            mmi_mode: Some(MmiMode::HighLevel),
        };
        let bytes = dc.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x01, 0x02, 0x01, 0x01]);
        assert_eq!(DisplayControl::parse(&bytes).unwrap(), dc);

        let q = DisplayControl {
            cmd: DisplayControlCmd::GetDisplayCharacterTableList,
            mmi_mode: None,
        };
        let qb = q.to_bytes();
        assert_eq!(qb, [0x9F, 0x88, 0x01, 0x01, 0x02]);
        assert_eq!(DisplayControl::parse(&qb).unwrap(), q);

        let mut other = dc;
        other.mmi_mode = Some(MmiMode::LowLevelFullScreenGraphics);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn display_reply_graphics_round_trips_and_bites() {
        let g = GraphicsCharacteristics {
            display_horizontal_size: 720,
            display_vertical_size: 576,
            aspect_ratio_information: 3,
            graphics_relation_to_video: 0b111,
            multiple_depths: true,
            display_bytes: 0x0ABC,
            composition_buffer_bytes: 0x12,
            object_cache_bytes: 0x34,
            depths: alloc::vec![
                PixelDepth {
                    display_depth: 2,
                    pixels_per_byte: 4,
                    region_overhead: 0x10,
                },
                PixelDepth {
                    display_depth: 7,
                    pixels_per_byte: 1,
                    region_overhead: 0x20,
                },
            ],
        };
        let dr = DisplayReply {
            reply_id: DisplayReplyId::ListGraphicOverlayCharacteristics,
            body: DisplayReplyBody::Graphics(g),
        };
        let bytes = dr.to_bytes();
        let parsed = DisplayReply::parse(&bytes).unwrap();
        assert_eq!(parsed, dr);
        if let DisplayReplyBody::Graphics(gg) = &parsed.body {
            assert_eq!(gg.depths.len(), 2);
            assert_eq!(gg.display_bytes, 0x0ABC);
            assert_eq!(gg.composition_buffer_bytes, 0x12);
            assert_eq!(gg.object_cache_bytes, 0x34);
        } else {
            panic!("expected graphics");
        }

        // bite: change one depth entry.
        let mut other = dr.clone();
        if let DisplayReplyBody::Graphics(gg) = &mut other.body {
            gg.depths[0].region_overhead = 0xFF;
        }
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn display_reply_char_tables_and_ack() {
        let ct = DisplayReply {
            reply_id: DisplayReplyId::ListDisplayCharacterTables,
            body: DisplayReplyBody::CharacterTables(alloc::vec![0x00, 0x01, 0x02]),
        };
        let bytes = ct.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x02, 0x04, 0x02, 0x00, 0x01, 0x02]);
        assert_eq!(DisplayReply::parse(&bytes).unwrap(), ct);

        let ack = DisplayReply {
            reply_id: DisplayReplyId::MmiModeAck,
            body: DisplayReplyBody::MmiModeAck(MmiMode::HighLevel),
        };
        let ab = ack.to_bytes();
        assert_eq!(ab, [0x9F, 0x88, 0x02, 0x02, 0x01, 0x01]);
        assert_eq!(DisplayReply::parse(&ab).unwrap(), ack);

        let unknown = DisplayReply {
            reply_id: DisplayReplyId::UnknownMmiMode,
            body: DisplayReplyBody::None,
        };
        let ub = unknown.to_bytes();
        assert_eq!(ub, [0x9F, 0x88, 0x02, 0x01, 0xF1]);
        assert_eq!(DisplayReply::parse(&ub).unwrap(), unknown);
    }

    #[test]
    fn keypad_control_multi_key_round_trips_and_bites() {
        let kc = KeypadControl {
            cmd: KeypadControlCmd::InterceptSelectedKeypress,
            key_codes: alloc::vec![0x00, 0x01, 0x0A], // 0, 1, menu
        };
        let bytes = kc.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x05, 0x04, 0x03, 0x00, 0x01, 0x0A]);
        let parsed = KeypadControl::parse(&bytes).unwrap();
        assert_eq!(parsed, kc);
        assert_eq!(parsed.key_codes.len(), 3);

        // intercept_all carries no codes.
        let all = KeypadControl {
            cmd: KeypadControlCmd::InterceptAllKeypresses,
            key_codes: Vec::new(),
        };
        let ab = all.to_bytes();
        assert_eq!(ab, [0x9F, 0x88, 0x05, 0x01, 0x01]);
        assert_eq!(KeypadControl::parse(&ab).unwrap(), all);

        // bite.
        let mut other = kc.clone();
        other.key_codes[1] = 0x09;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn keypress_round_trips_and_bites() {
        let k = Keypress { key_code: 0x0A };
        let bytes = k.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x06, 0x01, 0x0A]);
        assert_eq!(Keypress::parse(&bytes).unwrap(), k);
        let other = Keypress { key_code: 0x0B };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn subtitle_segment_more_bites() {
        let s = SubtitleSegment {
            more: false,
            segment: &[0x0F, 0x10, 0x00, 0x01, 0x00, 0x05],
        };
        let bytes = s.to_bytes();
        assert_eq!(bytes[2], 0x0E);
        assert_eq!(SubtitleSegment::parse(&bytes).unwrap(), s);
        let mut more = s.clone();
        more.more = true;
        let mb = more.to_bytes();
        assert_eq!(mb[2], 0x0F);
        assert_ne!(bytes, mb);
        assert_eq!(SubtitleSegment::parse(&mb).unwrap(), more);
    }

    #[test]
    fn display_message_round_trips_and_bites() {
        let m = DisplayMessage {
            message_id: DisplayMessageId::DvbSubtitlingSyntaxError,
        };
        let bytes = m.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x10, 0x01, 0x03]);
        assert_eq!(DisplayMessage::parse(&bytes).unwrap(), m);
        assert_eq!(m.message_id.name(), "DVB Subtitling syntax error");
        let other = DisplayMessage {
            message_id: DisplayMessageId::DisplayOk,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn scene_end_mark_round_trips_and_bites() {
        let s = SceneEndMark {
            decoder_continue_flag: true,
            scene_reveal_flag: false,
            send_scene_done: true,
            scene_tag: 0x05,
        };
        let bytes = s.to_bytes();
        // 1_0_1_1_0101 = 0xB5
        assert_eq!(bytes, [0x9F, 0x88, 0x11, 0x01, 0xB5]);
        assert_eq!(SceneEndMark::parse(&bytes).unwrap(), s);
        let mut other = s;
        other.scene_tag = 0x06;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn scene_done_message_two_bit_reserved() {
        let s = SceneDoneMessage {
            decoder_continue_flag: false,
            scene_reveal_flag: true,
            scene_tag: 0x0A,
        };
        let bytes = s.to_bytes();
        // 0_1_11_1010 = 0x7A
        assert_eq!(bytes, [0x9F, 0x88, 0x12, 0x01, 0x7A]);
        assert_eq!(SceneDoneMessage::parse(&bytes).unwrap(), s);
        let mut other = s;
        other.decoder_continue_flag = true;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn scene_control_round_trips() {
        let s = SceneControl {
            decoder_continue_flag: true,
            scene_reveal_flag: true,
            scene_tag: 0x03,
        };
        let bytes = s.to_bytes();
        // 1_1_11_0011 = 0xF3
        assert_eq!(bytes, [0x9F, 0x88, 0x13, 0x01, 0xF3]);
        assert_eq!(SceneControl::parse(&bytes).unwrap(), s);
    }

    #[test]
    fn subtitle_download_more_bites() {
        let s = SubtitleDownload {
            more: true,
            segment: &[0x0F, 0x13, 0x00, 0x01],
        };
        let bytes = s.to_bytes();
        assert_eq!(bytes[2], 0x15);
        assert_eq!(SubtitleDownload::parse(&bytes).unwrap(), s);
        let mut last = s.clone();
        last.more = false;
        let lb = last.to_bytes();
        assert_eq!(lb[2], 0x14);
        assert_ne!(bytes, lb);
    }

    #[test]
    fn flush_download_round_trips() {
        let bytes = FlushDownload.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x16, 0x00]);
        assert_eq!(FlushDownload::parse(&bytes).unwrap(), FlushDownload);
    }

    #[test]
    fn download_reply_round_trips_and_bites() {
        let d = DownloadReply {
            object_id: 0xFFFF,
            reply_id: DownloadReplyId::NotAnObjectDataSegment,
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x17, 0x03, 0xFF, 0xFF, 0x01]);
        assert_eq!(DownloadReply::parse(&bytes).unwrap(), d);
        let mut other = d;
        other.object_id = 0x0001;
        assert_ne!(bytes, other.to_bytes());
    }
}
