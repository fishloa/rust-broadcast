//! CEA-608 (line-21) caption decode — ANSI/CTA-608-E.
//!
//! Decodes the line-21 two-byte control / character codes that the
//! [`crate::CcData`] carriage demuxes (`cc_type` 0 = field 1, 1 = field 2) into a
//! caption screen. Implements the control-code state machine of
//! `cc-data/docs/decode/cea608-decode.md`:
//!
//! - pop-on (RCL/EOC), roll-up (RU2/RU3/RU4 + CR), paint-on (RDC) modes,
//! - Preamble Address Codes (row + indent + colour/italics/underline),
//! - mid-row colour/italics codes, tab offsets,
//! - the standard (Table 50), special (Table 49) and extended Western-European
//!   (Tables 5–10, automatic-backspace) character sets,
//! - the four data channels CC1–CC4, control-code doubling, and field-2 XDS
//!   detect-and-skip.
//!
//! Bytes carry odd parity in b7; this decoder strips parity (masks b7) to the
//! 7-bit value before classifying.

use crate::cc_data::{CcTriplet, CcType};
use alloc::string::String;
use alloc::vec::Vec;

/// Foreground colour of a line-21 caption cell (CTA-608-E Tables 51/53).
///
/// Returned by [`Cea608StyledChar::color`]; corresponds to the 3-bit colour
/// index in PAC / mid-row code tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Cea608Color {
    /// White (default).
    #[default]
    White,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Cyan.
    Cyan,
    /// Red.
    Red,
    /// Yellow.
    Yellow,
    /// Magenta.
    Magenta,
}

impl Cea608Color {
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Cyan => "cyan",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Magenta => "magenta",
        }
    }

    /// From the 3-bit colour index used in PAC and mid-row tables (Tables 51/53).
    #[must_use]
    pub(super) fn from_idx(idx: u8) -> Self {
        match idx & 0x07 {
            0 => Self::White,
            1 => Self::Green,
            2 => Self::Blue,
            3 => Self::Cyan,
            4 => Self::Red,
            5 => Self::Yellow,
            6 => Self::Magenta,
            _ => Self::White,
        }
    }
}
broadcast_common::impl_spec_display!(Cea608Color);

/// Number of caption rows on a line-21 screen (§3.2.2).
const SCREEN_ROWS: usize = 15;
/// Number of caption columns on a line-21 screen.
const SCREEN_COLS: usize = 32;

// ── Misc-control 2nd-byte values (Table 52, data-channel-1 column) ────────────
const MC_RCL: u8 = 0x20;
const MC_BS: u8 = 0x21;
const MC_DER: u8 = 0x24;
const MC_RU2: u8 = 0x25;
const MC_RU3: u8 = 0x26;
const MC_RU4: u8 = 0x27;
const MC_FON: u8 = 0x28;
const MC_RDC: u8 = 0x29;
const MC_TR: u8 = 0x2A;
const MC_RTD: u8 = 0x2B;
const MC_EDM: u8 = 0x2C;
const MC_CR: u8 = 0x2D;
const MC_ENM: u8 = 0x2E;
const MC_EOC: u8 = 0x2F;

// ── Tab offsets (Table 52): first byte 0x17/0x1F, 2nd byte 0x21–0x23 ──────────
const TAB_FIRST_C1: u8 = 0x17;
const TAB_FIRST_C2: u8 = 0x1F;
const TAB1: u8 = 0x21;
const TAB3: u8 = 0x23;

/// The caption mode (§6.1, §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Cea608Mode {
    /// No mode selected yet.
    #[default]
    None,
    /// Pop-on (RCL → load to back buffer → EOC flips).
    PopOn,
    /// Roll-up with the given number of rows (2/3/4).
    RollUp(u8),
    /// Paint-on (RDC → paint to displayed memory).
    PaintOn,
    /// Text mode (TR/RTD).
    Text,
}

impl Cea608Mode {
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PopOn => "pop_on",
            Self::RollUp(_) => "roll_up",
            Self::PaintOn => "paint_on",
            Self::Text => "text",
        }
    }
}
broadcast_common::impl_spec_display!(Cea608Mode);

/// A line-21 caption data channel (Table 1, §4.1): the eight CC/Text logical
/// services keyed by field + data-channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Cea608Channel {
    /// CC1 — field 1, data channel 1 (primary).
    Cc1,
    /// CC2 — field 1, data channel 2.
    Cc2,
    /// CC3 — field 2, data channel 1 (secondary).
    Cc3,
    /// CC4 — field 2, data channel 2.
    Cc4,
}

impl Cea608Channel {
    /// Index 0–3 into the decoder's per-channel state.
    #[must_use]
    fn index(self) -> usize {
        match self {
            Self::Cc1 => 0,
            Self::Cc2 => 1,
            Self::Cc3 => 2,
            Self::Cc4 => 3,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cc1 => "cc1",
            Self::Cc2 => "cc2",
            Self::Cc3 => "cc3",
            Self::Cc4 => "cc4",
        }
    }
}
broadcast_common::impl_spec_display!(Cea608Channel);

/// A styled character cell on a 608 caption screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Cea608StyledChar {
    /// The glyph.
    pub ch: char,
    /// Underline attribute.
    pub underline: bool,
    /// Italics attribute.
    pub italics: bool,
    /// Foreground colour (CTA-608-E Tables 51/53).
    pub color: Cea608Color,
}

impl Default for Cea608StyledChar {
    fn default() -> Self {
        Cea608StyledChar {
            ch: ' ',
            underline: false,
            italics: false,
            color: Cea608Color::White,
        }
    }
}

/// One row of a 608 caption screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Cea608Row {
    /// The styled cells (sparse: only populated cells present, indexed by column).
    cells: Vec<(usize, Cea608StyledChar)>,
}

impl Cea608Row {
    fn set(&mut self, col: usize, c: Cea608StyledChar) {
        if let Some(slot) = self.cells.iter_mut().find(|(i, _)| *i == col) {
            slot.1 = c;
        } else {
            self.cells.push((col, c));
        }
    }
    fn clear_from(&mut self, col: usize) {
        self.cells.retain(|(i, _)| *i < col);
    }
    fn remove(&mut self, col: usize) {
        self.cells.retain(|(i, _)| *i != col);
    }
    /// The row's text (cells sorted by column, gaps filled with spaces).
    #[must_use]
    pub fn text(&self) -> String {
        let mut sorted = self.cells.clone();
        sorted.sort_by_key(|(i, _)| *i);
        let mut out = String::new();
        let mut last = None;
        for (i, c) in sorted {
            if let Some(prev) = last {
                for _ in (prev + 1)..i {
                    out.push(' ');
                }
            }
            out.push(c.ch);
            last = Some(i);
        }
        out
    }
    /// Styled cells in column order.
    #[must_use]
    pub fn styled_cells(&self) -> Vec<(usize, Cea608StyledChar)> {
        let mut sorted = self.cells.clone();
        sorted.sort_by_key(|(i, _)| *i);
        sorted
    }
}

/// A 608 caption screen — `SCREEN_ROWS` rows of styled cells.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Cea608Screen {
    rows: [Cea608Row; SCREEN_ROWS],
}

impl Cea608Screen {
    /// The screen text: non-empty rows joined with `\n`, trailing spaces trimmed.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            let line = row.text();
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(trimmed);
        }
        out
    }
    /// All rows (including empty ones).
    #[must_use]
    pub fn rows(&self) -> &[Cea608Row; SCREEN_ROWS] {
        &self.rows
    }
}

/// Current pen attribute set by a PAC / mid-row code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pen {
    underline: bool,
    italics: bool,
    color: Cea608Color,
}

impl Default for Pen {
    fn default() -> Self {
        Pen {
            underline: false,
            italics: false,
            color: Cea608Color::White,
        }
    }
}

/// Per-channel caption state (displayed + non-displayed memory, mode, cursor).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ChannelState {
    displayed: Cea608Screen,
    nondisplayed: Cea608Screen,
    mode: Cea608Mode,
    rollup_rows: u8,
    cursor_row: usize,
    cursor_col: usize,
    pen: Pen,
}

impl ChannelState {
    /// The active memory for the current mode (pop-on writes to non-displayed;
    /// roll-up / paint-on write to displayed).
    fn active_mut(&mut self) -> &mut Cea608Screen {
        match self.mode {
            Cea608Mode::PopOn => &mut self.nondisplayed,
            _ => &mut self.displayed,
        }
    }
}

/// CEA-608 (line-21) caption decoder.
///
/// Feed it [`CcTriplet`]s (or raw byte pairs) of the 608 fields; read decoded
/// text per channel via [`channel_text`](Cea608Decoder::channel_text) /
/// [`screen`](Cea608Decoder::screen).
///
/// ```
/// use cc_data::decode::{Cea608Decoder, Cea608Channel};
/// let mut dec = Cea608Decoder::new();
/// // RCL, PAC row 15, "HI", EOC — a pop-on caption on CC1 (field 1).
/// dec.push_pair(false, 0x14, 0x20); // RCL (field 1 → CC1)
/// dec.push_pair(false, 0x14, 0x20); // doubled control — ignored
/// dec.push_pair(false, 0x14, 0x70); // PAC row 15 indent 0 (field 1)
/// dec.push_pair(false, b'H', b'I');
/// dec.push_pair(false, 0x14, 0x2F); // EOC → flip
/// assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cea608Decoder {
    channels: [ChannelState; 4],
    /// The control pair last acted on, per field, for doubling suppression.
    last_control: [Option<(u8, u8)>; 2],
    xds_active: bool,
}

impl Default for Cea608Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Cea608Decoder {
    /// A new decoder.
    #[must_use]
    pub fn new() -> Self {
        Cea608Decoder {
            channels: Default::default(),
            last_control: [None, None],
            xds_active: false,
        }
    }

    /// Feed the decoder the 608 (line-21) triplets of a [`crate::CcData`].
    pub fn push_triplets<'a, I>(&mut self, triplets: I)
    where
        I: IntoIterator<Item = &'a CcTriplet>,
    {
        for t in triplets {
            if !t.cc_valid {
                continue;
            }
            let field2 = match t.cc_type {
                CcType::Ntsc608Field1 => false,
                CcType::Ntsc608Field2 => true,
                _ => continue,
            };
            self.process_pair(field2, t.cc_data_1, t.cc_data_2);
        }
    }

    /// Feed one raw 608 byte pair. `field2 = false` ⇒ field 1 (`cc_type` 0);
    /// `field2 = true` ⇒ field 2 (`cc_type` 1). Bytes may carry parity in b7.
    pub fn push_pair(&mut self, field2: bool, b1: u8, b2: u8) {
        self.process_pair(field2, b1, b2);
    }

    fn process_pair(&mut self, field2: bool, raw1: u8, raw2: u8) {
        let b1 = raw1 & 0x7F;
        let b2 = raw2 & 0x7F;
        let field_idx = usize::from(field2);

        // Null filler (0x00 0x00): nothing to do.
        if b1 == 0x00 && b2 == 0x00 {
            return;
        }

        // XDS (field 2 only): a control pair with first byte 0x01–0x0F begins or
        // continues an XDS sub-packet; 0x0F + checksum ends it. Detect + skip the
        // whole span (we never decode XDS into captions).
        if field2 && (0x01..=0x0F).contains(&b1) {
            // 0x0F = End control (followed by a checksum byte); otherwise
            // Start/Continue. Either way this pair is XDS, not caption.
            self.xds_active = b1 != 0x0F;
            return;
        }
        // Any pair while an XDS sub-packet is open (field 2) is XDS payload
        // (informational chars 0x20–0x7F or null) until the End control closes it.
        if field2 && self.xds_active {
            return;
        }

        // Control codes: first byte 0x10–0x1F.
        if (0x10..=0x1F).contains(&b1) {
            // Doubling: an identical control pair immediately repeated is one cmd.
            if self.last_control[field_idx] == Some((b1, b2)) {
                self.last_control[field_idx] = None; // consume the doubled copy
                return;
            }
            self.last_control[field_idx] = Some((b1, b2));
            self.xds_active = false;
            self.handle_control(field2, b1, b2);
            return;
        }

        // Displayable characters: first byte 0x20–0x7F.
        self.last_control[field_idx] = None;
        self.xds_active = false;
        // Standard chars are not channel-tagged; route to the field's last-used
        // channel (default CC1 for field 1, CC3 for field 2).
        let ch = self.default_channel(field2);
        if b1 >= 0x20 {
            self.put_char(ch, Self::standard_char(b1));
        }
        if b2 >= 0x20 {
            self.put_char(ch, Self::standard_char(b2));
        }
    }

    /// The control-code handler. `b1` 0x10–0x1F, `b2` 0x20–0x7F (7-bit).
    fn handle_control(&mut self, field2: bool, b1: u8, b2: u8) {
        // Determine data channel from the first byte. For field 1: 0x10–0x17 =
        // C1, 0x18–0x1F = C2. For field 2: same first-byte split selects C1/C2
        // but maps to CC3/CC4.
        let c2 = b1 >= 0x18;
        let base1 = if c2 { b1 - 0x08 } else { b1 }; // fold C2 onto C1 range
        let ch = self.channel_for(field2, c2);

        // Misc control: first byte 0x14 (C1) / 0x15 (field-2 offset of 0x14).
        // After folding C2→C1 (base1) we compare on the C1 first byte.
        if base1 == 0x14 && (0x20..=0x2F).contains(&b2) {
            self.misc_control(ch, b2);
            return;
        }
        // Tab offsets: first byte 0x17 (C1) / 0x1F (C2), 2nd 0x21–0x23.
        if (b1 == TAB_FIRST_C1 || b1 == TAB_FIRST_C2) && (TAB1..=TAB3).contains(&b2) {
            let n = (b2 - TAB1 + 1) as usize;
            let st = &mut self.channels[ch.index()];
            st.cursor_col = (st.cursor_col + n).min(SCREEN_COLS - 1);
            return;
        }
        // Mid-row codes: first byte 0x11 (C1) / 0x19 (C2), 2nd 0x20–0x2F.
        if base1 == 0x11 && (0x20..=0x2F).contains(&b2) {
            self.mid_row(ch, b2);
            return;
        }
        // Special characters: first byte 0x11/0x19, 2nd 0x30–0x3F.
        if base1 == 0x11 && (0x30..=0x3F).contains(&b2) {
            self.put_char(ch, Self::special_char(b2));
            return;
        }
        // Extended chars block 1: first byte 0x12/0x1A, 2nd 0x20–0x3F.
        if base1 == 0x12 && (0x20..=0x3F).contains(&b2) {
            self.put_extended(ch, Self::extended_char_block1(b2));
            return;
        }
        // Extended chars block 2: first byte 0x13/0x1B, 2nd 0x20–0x3F.
        if base1 == 0x13 && (0x20..=0x3F).contains(&b2) {
            self.put_extended(ch, Self::extended_char_block2(b2));
            return;
        }
        // PAC: 2nd byte 0x40–0x7F (any control first byte).
        if (0x40..=0x7F).contains(&b2) {
            self.pac(field2, ch, b1, b2);
        }
        // Background/foreground attr codes (0x10/0x18 2nd 0x20–0x2F) — ignored
        // for text extraction.
    }

    /// Miscellaneous control codes (RCL/RU2-4/RDC/CR/EDM/ENM/EOC/DER/BS …).
    fn misc_control(&mut self, ch: Cea608Channel, code: u8) {
        let st = &mut self.channels[ch.index()];
        match code {
            MC_RCL => {
                st.mode = Cea608Mode::PopOn;
                st.nondisplayed = Cea608Screen::default();
                st.cursor_row = 0;
                st.cursor_col = 0;
            }
            MC_RU2 | MC_RU3 | MC_RU4 => {
                let rows = match code {
                    MC_RU2 => 2,
                    MC_RU3 => 3,
                    _ => 4,
                };
                st.mode = Cea608Mode::RollUp(rows);
                st.rollup_rows = rows;
                // base row defaults to bottom (row 15 → index 14)
                st.cursor_row = SCREEN_ROWS - 1;
                st.cursor_col = 0;
                st.pen = Pen::default();
            }
            MC_RDC => {
                st.mode = Cea608Mode::PaintOn;
            }
            MC_CR => {
                self.carriage_return(ch);
            }
            MC_EDM => {
                st.displayed = Cea608Screen::default();
            }
            MC_ENM => {
                st.nondisplayed = Cea608Screen::default();
            }
            MC_EOC => {
                // flip non-displayed ↔ displayed (pop-on)
                core::mem::swap(&mut st.displayed, &mut st.nondisplayed);
            }
            MC_BS if st.cursor_col > 0 => {
                st.cursor_col -= 1;
                let row = st.cursor_row.min(SCREEN_ROWS - 1);
                let col = st.cursor_col;
                st.active_mut().rows[row].remove(col);
            }
            MC_DER => {
                let row = st.cursor_row.min(SCREEN_ROWS - 1);
                let col = st.cursor_col;
                st.active_mut().rows[row].clear_from(col);
            }
            MC_TR => {
                st.mode = Cea608Mode::Text;
                st.displayed = Cea608Screen::default();
                st.cursor_row = 0;
                st.cursor_col = 0;
            }
            MC_RTD => {
                st.mode = Cea608Mode::Text;
            }
            MC_FON => {}
            _ => {}
        }
    }

    /// Carriage Return in roll-up: roll the window up one row, clear the base row.
    fn carriage_return(&mut self, ch: Cea608Channel) {
        let st = &mut self.channels[ch.index()];
        if let Cea608Mode::RollUp(rows) = st.mode {
            let base = st.cursor_row.min(SCREEN_ROWS - 1);
            let rows = rows as usize;
            let top = base.saturating_sub(rows - 1);
            // Move each row up by one within [top..=base].
            for r in top..base {
                st.displayed.rows[r] = st.displayed.rows[r + 1].clone();
            }
            st.displayed.rows[base] = Cea608Row::default();
            st.cursor_col = 0;
        } else {
            // In other modes, CR is uncommon; just move cursor down.
            if st.cursor_row + 1 < SCREEN_ROWS {
                st.cursor_row += 1;
            }
            st.cursor_col = 0;
        }
    }

    /// Mid-row code (Table 51): sets colour/italics + underline from the cursor.
    fn mid_row(&mut self, ch: Cea608Channel, b2: u8) {
        let idx = (b2 - 0x20) as usize; // 0x20..0x2F → 0..15
        let underline = idx & 0x01 != 0;
        let color_idx = (idx >> 1) as u8;
        let (color, italics) = if color_idx <= 6 {
            (Cea608Color::from_idx(color_idx), false)
        } else {
            (Cea608Color::White, true) // 0x2E/0x2F = italics
        };
        let st = &mut self.channels[ch.index()];
        st.pen = Pen {
            underline,
            italics,
            color,
        };
        // mid-row occupies one cell (a space carrying the attribute)
        self.put_char(ch, ' ');
    }

    /// Preamble Address Code: set row + indent / colour + underline (Table 53).
    fn pac(&mut self, _field2: bool, ch: Cea608Channel, b1: u8, b2: u8) {
        // Fold the C2 first byte (0x18–0x1F) onto the C1 first byte (0x10–0x17).
        let f = if b1 >= 0x18 { b1 - 0x08 } else { b1 };
        // Row group from the first byte (Table 53 top block, C1 column).
        let row_pair = match f {
            0x11 => 1,  // rows 1–2
            0x12 => 3,  // rows 3–4
            0x15 => 5,  // rows 5–6
            0x16 => 7,  // rows 7–8
            0x17 => 9,  // rows 9–10
            0x10 => 11, // row 11
            0x13 => 12, // rows 12–13
            0x14 => 14, // rows 14–15
            _ => 1,
        };
        // Second byte high bit picks the row within the pair (0x40–0x5F = first
        // of pair, 0x60–0x7F = second).
        let second_of_pair = b2 >= 0x60;
        let row = if f == 0x10 {
            11 // row 11 has no pair partner
        } else if second_of_pair {
            row_pair + 1
        } else {
            row_pair
        };
        let row = row.clamp(1, SCREEN_ROWS); // 1-based
        let attr = b2 & 0x1F; // low 5 bits (drop the col-A/B high nibble)
        let underline = attr & 0x01 != 0;

        let (color, italics, indent) = if attr >= 0x10 {
            // Indent codes 0x10..0x1F → indent 0..28 in steps of 4, colour white.
            let indent = ((attr - 0x10) >> 1) as usize * 4;
            (Cea608Color::White, false, indent)
        } else {
            // Colour/italics codes 0x00..0x0F.
            let color_idx = attr >> 1;
            let (c, it) = if color_idx <= 6 {
                (Cea608Color::from_idx(color_idx), false)
            } else {
                (Cea608Color::White, true)
            };
            (c, it, 0)
        };

        let st = &mut self.channels[ch.index()];
        st.cursor_row = (row - 1).min(SCREEN_ROWS - 1);
        st.cursor_col = indent.min(SCREEN_COLS - 1);
        st.pen = Pen {
            underline,
            italics,
            color,
        };
    }

    /// Put a standard / special character at the cursor, advancing it.
    fn put_char(&mut self, ch: Cea608Channel, c: char) {
        let st = &mut self.channels[ch.index()];
        let row = st.cursor_row.min(SCREEN_ROWS - 1);
        let col = st.cursor_col;
        if col >= SCREEN_COLS {
            return;
        }
        let pen = st.pen;
        let styled = Cea608StyledChar {
            ch: c,
            underline: pen.underline,
            italics: pen.italics,
            color: pen.color,
        };
        st.active_mut().rows[row].set(col, styled);
        st.cursor_col = (col + 1).min(SCREEN_COLS);
    }

    /// Put an extended char: automatic backspace first (erase the fallback char
    /// the provider sent), then the glyph.
    fn put_extended(&mut self, ch: Cea608Channel, c: char) {
        {
            let st = &mut self.channels[ch.index()];
            if st.cursor_col > 0 {
                st.cursor_col -= 1;
                let row = st.cursor_row.min(SCREEN_ROWS - 1);
                let col = st.cursor_col;
                st.active_mut().rows[row].remove(col);
            }
        }
        self.put_char(ch, c);
    }

    /// Resolve the data channel for a control pair.
    fn channel_for(&self, field2: bool, c2: bool) -> Cea608Channel {
        match (field2, c2) {
            (false, false) => Cea608Channel::Cc1,
            (false, true) => Cea608Channel::Cc2,
            (true, false) => Cea608Channel::Cc3,
            (true, true) => Cea608Channel::Cc4,
        }
    }

    /// The default channel for routing displayable chars on a field.
    fn default_channel(&self, field2: bool) -> Cea608Channel {
        if field2 {
            Cea608Channel::Cc3
        } else {
            Cea608Channel::Cc1
        }
    }

    /// The displayed screen for a channel.
    #[must_use]
    pub fn screen(&self, channel: Cea608Channel) -> &Cea608Screen {
        &self.channels[channel.index()].displayed
    }

    /// The current mode for a channel.
    #[must_use]
    pub fn mode(&self, channel: Cea608Channel) -> Cea608Mode {
        self.channels[channel.index()].mode
    }

    /// The decoded displayed text for a channel.
    #[must_use]
    pub fn channel_text(&self, channel: Cea608Channel) -> String {
        self.channels[channel.index()].displayed.text()
    }

    // ── Character tables ──────────────────────────────────────────────────────

    /// Standard character set (Table 50): 7-bit `0x20`–`0x7F` → glyph.
    fn standard_char(b: u8) -> char {
        match b {
            0x2A => '\u{00E1}', // á
            0x5C => '\u{00E9}', // é
            0x5E => '\u{00ED}', // í
            0x5F => '\u{00F3}', // ó
            0x60 => '\u{00FA}', // ú
            0x7B => '\u{00E7}', // ç
            0x7C => '\u{00F7}', // ÷
            0x7D => '\u{00D1}', // Ñ
            0x7E => '\u{00F1}', // ñ
            0x7F => '\u{25A0}', // ■
            _ => char::from(b), // ASCII otherwise
        }
    }

    /// Special characters (Table 49): 2nd byte 0x30–0x3F.
    fn special_char(b2: u8) -> char {
        match b2 {
            0x30 => '\u{00AE}', // ®
            0x31 => '\u{00B0}', // °
            0x32 => '\u{00BD}', // ½
            0x33 => '\u{00BF}', // ¿
            0x34 => '\u{2122}', // ™
            0x35 => '\u{00A2}', // ¢
            0x36 => '\u{00A3}', // £
            0x37 => '\u{266A}', // ♪
            0x38 => '\u{00E0}', // à
            0x39 => ' ',        // transparent space
            0x3A => '\u{00E8}', // è
            0x3B => '\u{00E2}', // â
            0x3C => '\u{00EA}', // ê
            0x3D => '\u{00EE}', // î
            0x3E => '\u{00F4}', // ô
            0x3F => '\u{00FB}', // û
            _ => '?',
        }
    }

    /// Extended Western-European block 1 (first byte 0x12/0x1A; Tables 5–7).
    fn extended_char_block1(b2: u8) -> char {
        match b2 {
            // Spanish (Table 5)
            0x20 => '\u{00C1}', // Á
            0x21 => '\u{00C9}', // É
            0x22 => '\u{00D3}', // Ó
            0x23 => '\u{00DA}', // Ú
            0x24 => '\u{00DC}', // Ü
            0x25 => '\u{00FC}', // ü
            0x26 => '\u{2018}', // ‘
            0x27 => '\u{00A1}', // ¡
            // Misc (Table 6)
            0x28 => '*',
            0x29 => '\'',
            0x2A => '\u{2014}', // —
            0x2B => '\u{00A9}', // ©
            0x2C => '\u{2120}', // ℠
            0x2D => '\u{2022}', // •
            0x2E => '\u{201C}', // "
            0x2F => '\u{201D}', // "
            // French (Table 7)
            0x30 => '\u{00C0}', // À
            0x31 => '\u{00C2}', // Â
            0x32 => '\u{00C7}', // Ç
            0x33 => '\u{00C8}', // È
            0x34 => '\u{00CA}', // Ê
            0x35 => '\u{00CB}', // Ë
            0x36 => '\u{00EB}', // ë
            0x37 => '\u{00CE}', // Î
            0x38 => '\u{00CF}', // Ï
            0x39 => '\u{00EF}', // ï
            0x3A => '\u{00D4}', // Ô
            0x3B => '\u{00D9}', // Ù
            0x3C => '\u{00F9}', // ù
            0x3D => '\u{00DB}', // Û
            0x3E => '\u{00AB}', // «
            0x3F => '\u{00BB}', // »
            _ => '?',
        }
    }

    /// Extended Western-European block 2 (first byte 0x13/0x1B; Tables 8–10).
    fn extended_char_block2(b2: u8) -> char {
        match b2 {
            // Portuguese (Table 8)
            0x20 => '\u{00C3}', // Ã
            0x21 => '\u{00E3}', // ã
            0x22 => '\u{00CD}', // Í
            0x23 => '\u{00CC}', // Ì
            0x24 => '\u{00EC}', // ì
            0x25 => '\u{00D2}', // Ò
            0x26 => '\u{00F2}', // ò
            0x27 => '\u{00D5}', // Õ
            0x28 => '\u{00F5}', // õ
            0x29 => '{',
            0x2A => '}',
            0x2B => '\\',
            0x2C => '^',
            0x2D => '_',
            0x2E => '|',
            0x2F => '~',
            // German (Table 9)
            0x30 => '\u{00C4}', // Ä
            0x31 => '\u{00E4}', // ä
            0x32 => '\u{00D6}', // Ö
            0x33 => '\u{00F6}', // ö
            0x34 => '\u{00DF}', // ß
            0x35 => '\u{00A5}', // ¥
            0x36 => '\u{00A4}', // ¤
            0x37 => '|',
            // Danish (Table 10)
            0x38 => '\u{00C5}', // Å
            0x39 => '\u{00E5}', // å
            0x3A => '\u{00D8}', // Ø
            0x3B => '\u{00F8}', // ø
            0x3C => '\u{231C}', // ⌜
            0x3D => '\u{231D}', // ⌝
            0x3E => '\u{231E}', // ⌞
            0x3F => '\u{231F}', // ⌟
            _ => '?',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Add odd parity to a 7-bit value (so inputs look like real line-21 bytes).
    fn par(v: u8) -> u8 {
        let ones = (v & 0x7F).count_ones();
        if ones % 2 == 0 { v | 0x80 } else { v & 0x7F }
    }

    /// Pop-on caption: RCL, PAC row 15 indent 0, "HI", EOC → on-screen "HI".
    #[test]
    fn pop_on_caption() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x20)); // RCL
        dec.push_pair(false, par(0x14), par(0x70)); // PAC row 15 (col B), white indent0
        dec.push_pair(false, par(b'H'), par(b'I'));
        dec.push_pair(false, par(0x14), par(0x2F)); // EOC → flip to displayed
        assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");
        assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::PopOn);
    }

    /// Control-code doubling: a repeated RCL pair acts once.
    #[test]
    fn control_doubling() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x29)); // RDC
        dec.push_pair(false, par(0x14), par(0x29)); // doubled — ignored
        assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::PaintOn);
        // a non-doubled second RDC after a non-control acts again
        dec.push_pair(false, par(b'X'), par(0x00));
        dec.push_pair(false, par(0x14), par(0x2C)); // EDM
    }

    /// Roll-up across ≥2 rows: RU2, write line 1, CR, write line 2.
    #[test]
    fn roll_up_two_rows() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x25)); // RU2
        dec.push_pair(false, par(b'A'), par(b'B'));
        dec.push_pair(false, par(0x14), par(0x2D)); // CR
        dec.push_pair(false, par(b'C'), par(b'D'));
        let text = dec.channel_text(Cea608Channel::Cc1);
        assert!(text.contains("AB"), "got {text:?}");
        assert!(text.contains("CD"), "got {text:?}");
        // AB rolled up above CD
        let ab = text.find("AB").unwrap();
        let cd = text.find("CD").unwrap();
        assert!(ab < cd, "AB should be above CD: {text:?}");
    }

    /// Mid-row colour change places a styled space and changes the pen colour.
    #[test]
    fn mid_row_colour() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x20)); // RCL
        dec.push_pair(false, par(0x14), par(0x70)); // PAC row 15
        dec.push_pair(false, par(b'A'), par(0x00));
        dec.push_pair(false, par(0x11), par(0x22)); // mid-row Green
        dec.push_pair(false, par(b'B'), par(0x00));
        dec.push_pair(false, par(0x14), par(0x2F)); // EOC
        // Find the styled cells; "B" must be green.
        let screen = dec.screen(Cea608Channel::Cc1);
        let mut found_green_b = false;
        for row in screen.rows() {
            for (_, c) in row.styled_cells() {
                if c.ch == 'B' {
                    assert_eq!(c.color, Cea608Color::Green);
                    found_green_b = true;
                }
            }
        }
        assert!(found_green_b, "expected a green 'B'");
    }

    /// Special character (musical note) via 0x11 0x37.
    #[test]
    fn special_char_music_note() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x29)); // RDC (paint-on → displayed)
        dec.push_pair(false, par(0x11), par(0x37)); // ♪ on CC1
        assert!(dec.channel_text(Cea608Channel::Cc1).contains('\u{266A}'));
    }

    /// Extended char with automatic backspace: provider sends 'u' then ü code.
    #[test]
    fn extended_char_backspace() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x14), par(0x29)); // RDC
        dec.push_pair(false, par(b'u'), par(0x00)); // fallback 'u'
        dec.push_pair(false, par(0x12), par(0x25)); // extended ü (block1 0x25)
        let t = dec.channel_text(Cea608Channel::Cc1);
        assert_eq!(t, "\u{00FC}"); // just ü, the 'u' was backspaced
    }

    /// Channel routing: a CC2 control (0x1C…) writes to CC2, not CC1.
    #[test]
    fn channel_cc2() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, par(0x1C), par(0x29)); // RDC on data channel 2 → CC2
        assert_eq!(dec.mode(Cea608Channel::Cc2), Cea608Mode::PaintOn);
        assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::None);
    }

    /// Field 2 → CC3.
    #[test]
    fn field2_cc3() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(true, par(0x14), par(0x29)); // RDC field 2 ch1 → CC3
        assert_eq!(dec.mode(Cea608Channel::Cc3), Cea608Mode::PaintOn);
    }

    /// XDS on field 2 is detected and skipped (no caption output).
    #[test]
    fn xds_skipped() {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(true, par(0x01), par(0x02)); // XDS start (Current class)
        dec.push_pair(true, par(0x20), par(0x21)); // XDS informational
        dec.push_pair(true, par(0x0F), par(0x40)); // XDS end + checksum
        assert_eq!(dec.channel_text(Cea608Channel::Cc3), "");
    }

    #[test]
    fn standard_char_accents() {
        assert_eq!(Cea608Decoder::standard_char(0x2A), '\u{00E1}'); // á
        assert_eq!(Cea608Decoder::standard_char(b'A'), 'A');
        assert_eq!(Cea608Decoder::standard_char(0x7F), '\u{25A0}'); // ■
    }

    #[test]
    fn no_panic_on_arbitrary_input() {
        let mut dec = Cea608Decoder::new();
        let mut x: u32 = 0xDEAD_BEEF;
        for _ in 0..8192 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let b1 = (x >> 8) as u8;
            let b2 = (x >> 16) as u8;
            let f2 = (x & 1) != 0;
            dec.push_pair(f2, b1, b2);
        }
        // Also via triplets, including invalid + truncated patterns.
        let triplets = [
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Ntsc608Field1,
                cc_data_1: 0x14,
                cc_data_2: 0x2D,
            },
            CcTriplet {
                cc_valid: false,
                cc_type: CcType::Ntsc608Field2,
                cc_data_1: 0xFF,
                cc_data_2: 0xFF,
            },
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Ntsc608Field2,
                cc_data_1: 0x01,
                cc_data_2: 0x00,
            },
        ];
        dec.push_triplets(&triplets);
    }
}
