//! CEA-708 (DTVCC) caption decode — ANSI/CTA-708-E S-2023 §5–§8 + 47 CFR §79.102.
//!
//! Decode pipeline (`dvb-cc/docs/decode/cea708-decode.md`):
//! `cc_data` byte pairs → Caption Channel Packets (§5) → Service Blocks (§6) →
//! the C0/C1/G0/G1/G2/G3 command interpreter (§7/§8) driving the window + pen
//! model. Up to six services (47 CFR §79.102 (c)) are tracked; each service has
//! eight windows (DF0–DF7) and a current pen. Decoded window text is exposed.
//!
//! Decoder is panic-free on arbitrary input: short / over-length packets, bad
//! service blocks and truncated commands are ignored.

use crate::cc_data::{CcTriplet, CcType};
use crate::decode::screen::{
    Color, EdgeType, FontStyle, Justify, Opacity, PenOffset, PenSize, PrintDirection,
    ScrollDirection,
};
use alloc::string::String;
use alloc::vec::Vec;

// ── Service / window counts (§6.1, §8) ──────────────────────────────────────
/// Number of standard services tracked (47 CFR §79.102 (c): Caption Service #1–#6).
const NUM_SERVICES: usize = 6;
/// Windows per service (DF0–DF7).
const NUM_WINDOWS: usize = 8;
/// Maximum rows in a window (rc field, virtual rows − 1, max 11 → 12 rows).
const MAX_WINDOW_ROWS: usize = 12;
/// Maximum columns in a window (cc field, virtual cols − 1, max 41 → 42 cols).
const MAX_WINDOW_COLS: usize = 42;

// ── Packet layer (§5) ───────────────────────────────────────────────────────
/// `packet_size_code == 0` ⇒ 127 data bytes (§5.1).
const PACKET_SIZE_ZERO_DATA: usize = 127;

// ── Service block (§6.2) ──────────────────────────────────────────────────────
/// `service_number == 7` is the extended-service escape (§6.2.2).
const EXTENDED_SERVICE_ESCAPE: u8 = 7;

// ── C0 control codes (§7.1.4, Table 13) ───────────────────────────────────────
const C0_NUL: u8 = 0x00;
const C0_ETX: u8 = 0x03;
const C0_BS: u8 = 0x08;
const C0_FF: u8 = 0x0C;
const C0_CR: u8 = 0x0D;
const C0_HCR: u8 = 0x0E;
const C0_EXT1: u8 = 0x10;
const C0_P16: u8 = 0x18;

// ── C1 caption command opcodes (§7.1.5, Table 14) ─────────────────────────────
const C1_CW0: u8 = 0x80; // CW0..CW7 = 0x80..=0x87
const C1_CW7: u8 = 0x87;
const C1_CLW: u8 = 0x88;
const C1_DSW: u8 = 0x89;
const C1_HDW: u8 = 0x8A;
const C1_TGW: u8 = 0x8B;
const C1_DLW: u8 = 0x8C;
const C1_DLY: u8 = 0x8D;
const C1_DLC: u8 = 0x8E;
const C1_RST: u8 = 0x8F;
const C1_SPA: u8 = 0x90;
const C1_SPC: u8 = 0x91;
const C1_SPL: u8 = 0x92;
const C1_SWA: u8 = 0x97;
const C1_DF0: u8 = 0x98; // DF0..DF7 = 0x98..=0x9F
const C1_DF7: u8 = 0x9F;

// ── Code-space range boundaries (§7.1, Table 11) ──────────────────────────────
const C0_END: u8 = 0x1F;
const G0_START: u8 = 0x20;
const G0_END: u8 = 0x7F;
const C1_START: u8 = 0x80;
const C1_END: u8 = 0x9F;
const G1_START: u8 = 0xA0;

// ── G0 substitution (§7.1.6): 0x7F is the musical note, not DEL ───────────────
const G0_MUSIC_NOTE: u8 = 0x7F;

/// State of a window's display (§8.10.5 DisplayWindows / HideWindows / Toggle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum WindowState {
    /// Window defined but not yet shown (default after DefineWindow).
    #[default]
    Hidden,
    /// Window is being displayed.
    Visible,
}

impl WindowState {
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Visible => "visible",
        }
    }
}
dvb_common::impl_spec_display!(WindowState);

/// A decoded CEA-708 caption window (§8.4 window model).
///
/// Holds the window attributes set by `DefineWindow` / `SetWindowAttributes`,
/// the pen attributes set by `SetPenAttributes` / `SetPenColor`, and the painted
/// text grid. Only created when a `DefineWindow` for its ID is received.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Window {
    /// Whether the window is currently displayed.
    pub state: WindowState,
    /// Window priority, 0 (highest) – 7.
    pub priority: u8,
    /// Anchor point (0–8).
    pub anchor_point: u8,
    /// Anchor vertical coordinate.
    pub anchor_vertical: u8,
    /// Anchor horizontal coordinate.
    pub anchor_horizontal: u8,
    /// `true` if the anchor coordinates are relative (percent).
    pub relative_position: bool,
    /// Row count (virtual rows; = rc + 1).
    pub row_count: u8,
    /// Column count (virtual cols; = cc + 1).
    pub column_count: u8,
    /// Row lock.
    pub row_lock: bool,
    /// Column lock.
    pub column_lock: bool,
    /// Window-style preset ID (0–7) requested in the last DefineWindow.
    pub window_style: u8,
    /// Pen-style preset ID (0–7) requested in the last DefineWindow.
    pub pen_style: u8,
    /// Justification.
    pub justify: Justify,
    /// Print direction.
    pub print_direction: PrintDirection,
    /// Scroll direction.
    pub scroll_direction: ScrollDirection,
    /// Word wrap.
    pub word_wrap: bool,
    /// Window fill colour.
    pub fill_color: Color,
    /// Window fill opacity.
    pub fill_opacity: Opacity,
    /// Border colour.
    pub border_color: Color,
    /// Border type (0=none … 5=shadow-right).
    pub border_type: u8,
    /// Pen size.
    pub pen_size: PenSize,
    /// Pen offset (subscript/normal/superscript).
    pub pen_offset: PenOffset,
    /// Font style.
    pub font_style: FontStyle,
    /// Pen italics.
    pub italics: bool,
    /// Pen underline.
    pub underline: bool,
    /// Pen edge type.
    pub edge_type: EdgeType,
    /// Pen foreground colour.
    pub fg_color: Color,
    /// Pen foreground opacity.
    pub fg_opacity: Opacity,
    /// Pen background colour.
    pub bg_color: Color,
    /// Pen background opacity.
    pub bg_opacity: Opacity,
    /// Text grid, `row_count` rows of `column_count` chars (rows are `String`s).
    rows: Vec<String>,
    /// Current pen row.
    pen_row: usize,
    /// Current pen column.
    pen_col: usize,
}

impl Window {
    fn new() -> Self {
        Window {
            state: WindowState::Hidden,
            priority: 0,
            anchor_point: 0,
            anchor_vertical: 0,
            anchor_horizontal: 0,
            relative_position: false,
            row_count: 1,
            column_count: 1,
            row_lock: false,
            column_lock: false,
            window_style: 0,
            pen_style: 0,
            justify: Justify::Left,
            print_direction: PrintDirection::LeftToRight,
            scroll_direction: ScrollDirection::BottomToTop,
            word_wrap: false,
            fill_color: Color::BLACK,
            fill_opacity: Opacity::Solid,
            border_color: Color::BLACK,
            border_type: 0,
            pen_size: PenSize::Standard,
            pen_offset: PenOffset::Normal,
            font_style: FontStyle::Default,
            italics: false,
            underline: false,
            edge_type: EdgeType::None,
            fg_color: Color::WHITE,
            fg_opacity: Opacity::Solid,
            bg_color: Color::BLACK,
            bg_opacity: Opacity::Solid,
            rows: Vec::new(),
            pen_row: 0,
            pen_col: 0,
        }
    }

    fn ensure_grid(&mut self) {
        let rows = (self.row_count as usize).clamp(1, MAX_WINDOW_ROWS);
        if self.rows.len() != rows {
            self.rows = alloc::vec![String::new(); rows];
        }
    }

    fn clear_text(&mut self) {
        for r in &mut self.rows {
            r.clear();
        }
        self.pen_row = 0;
        self.pen_col = 0;
    }

    fn cols(&self) -> usize {
        (self.column_count as usize).clamp(1, MAX_WINDOW_COLS)
    }

    /// Append a character at the current pen position, advancing the pen.
    fn put_char(&mut self, ch: char) {
        self.ensure_grid();
        let cols = self.cols();
        if self.pen_row >= self.rows.len() {
            return;
        }
        // pad row out to pen_col with spaces
        let row = &mut self.rows[self.pen_row];
        while row.chars().count() < self.pen_col {
            row.push(' ');
        }
        if self.pen_col < cols {
            row.push(ch);
            self.pen_col += 1;
        }
    }

    /// Back Space (C0 BS).
    fn back_space(&mut self) {
        if self.pen_col > 0 {
            self.pen_col -= 1;
            if self.pen_row < self.rows.len() {
                let row = &mut self.rows[self.pen_row];
                let mut chars: Vec<char> = row.chars().collect();
                if self.pen_col < chars.len() {
                    chars.truncate(self.pen_col);
                    *row = chars.into_iter().collect();
                }
            }
        }
    }

    /// Carriage Return (C0 CR): start of next row; roll up if past the bottom.
    fn carriage_return(&mut self) {
        self.ensure_grid();
        self.pen_col = 0;
        if self.pen_row + 1 < self.rows.len() {
            self.pen_row += 1;
        } else if !self.rows.is_empty() {
            // roll up: drop the top row, append a blank at the bottom
            self.rows.remove(0);
            self.rows.push(String::new());
            self.pen_row = self.rows.len() - 1;
        }
    }

    /// Horizontal Carriage Return (C0 HCR): start of current row, erase the row.
    fn horizontal_cr(&mut self) {
        self.ensure_grid();
        if self.pen_row < self.rows.len() {
            self.rows[self.pen_row].clear();
        }
        self.pen_col = 0;
    }

    fn set_pen_location(&mut self, row: usize, col: usize) {
        self.ensure_grid();
        self.pen_row = row.min(self.rows.len().saturating_sub(1));
        self.pen_col = col.min(self.cols());
    }

    /// The window's visible text, rows joined with `\n`, trailing blank rows
    /// trimmed and per-row trailing spaces removed.
    #[must_use]
    pub fn text(&self) -> String {
        // Trim trailing per-row spaces; keep interior blank rows as newlines but
        // drop trailing blank rows.
        let mut lines: Vec<&str> = self.rows.iter().map(|r| r.trim_end()).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

/// One DTVCC service (§6.1): up to eight windows + a current-window pointer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct Service {
    windows: [Option<Window>; NUM_WINDOWS],
    /// Current window ID (0–7), or `None` when unknown.
    current_window: Option<usize>,
}

impl Service {
    fn reset(&mut self) {
        *self = Service::default();
    }

    fn current(&mut self) -> Option<&mut Window> {
        let id = self.current_window?;
        self.windows.get_mut(id)?.as_mut()
    }
}

/// CEA-708 (DTVCC) caption decoder.
///
/// Feed it [`CcTriplet`]s (or raw `cc_data` byte pairs) from the DTVCC stream
/// (`cc_type` 2/3); read decoded window text per service via
/// [`service_text`](Cea708Decoder::service_text) / [`windows`](Cea708Decoder::windows).
///
/// ```
/// use dvb_cc::decode::Cea708Decoder;
/// let mut dec = Cea708Decoder::new();
/// // A CCP (header + service-1 block) carrying the DefineWindow worked example
/// // for window 2: 0x9A 38 4A D1 8B 0F 11.
/// dec.push_packet(&[0x05, 0x27, 0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11]);
/// let w = &dec.windows(1)[2];
/// assert!(w.is_some());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Cea708Decoder {
    services: [Service; NUM_SERVICES],
    /// Accumulated CCP data for the in-progress packet.
    packet: Vec<u8>,
    /// Last sequence number seen (for discontinuity detection).
    last_seq: Option<u8>,
}

impl Default for Cea708Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Cea708Decoder {
    /// A new decoder with no services defined.
    #[must_use]
    pub fn new() -> Self {
        Cea708Decoder {
            services: Default::default(),
            packet: Vec::new(),
            last_seq: None,
        }
    }

    /// Reset every service (§8.9.5 packet-loss recovery / RST).
    pub fn reset(&mut self) {
        for s in &mut self.services {
            s.reset();
        }
        self.packet.clear();
        self.last_seq = None;
    }

    /// Feed the decoder the 708 (DTVCC) triplets of a [`crate::CcData`].
    ///
    /// A `cc_type == Dtvcc708Start` triplet begins a new Caption Channel Packet;
    /// `Dtvcc708Data` triplets continue it. Invalid triplets are skipped.
    pub fn push_triplets<'a, I>(&mut self, triplets: I)
    where
        I: IntoIterator<Item = &'a CcTriplet>,
    {
        for t in triplets {
            if !t.cc_valid {
                continue;
            }
            match t.cc_type {
                CcType::Dtvcc708Start => {
                    // a new CCP starts; flush any complete prior packet
                    self.flush_packet();
                    self.packet.clear();
                    self.packet.push(t.cc_data_1);
                    self.packet.push(t.cc_data_2);
                }
                CcType::Dtvcc708Data => {
                    self.packet.push(t.cc_data_1);
                    self.packet.push(t.cc_data_2);
                }
                _ => {}
            }
        }
        self.flush_packet();
    }

    /// Feed one complete Caption Channel Packet (the CCP header byte followed by
    /// its data bytes). Useful for testing / when packets are pre-assembled.
    pub fn push_packet(&mut self, ccp: &[u8]) {
        self.decode_packet(ccp);
    }

    /// Flush the accumulated packet buffer if it forms a complete CCP.
    fn flush_packet(&mut self) {
        if self.packet.is_empty() {
            return;
        }
        let packet = core::mem::take(&mut self.packet);
        self.decode_packet(&packet);
    }

    /// Decode a Caption Channel Packet (§5): header byte + service blocks.
    fn decode_packet(&mut self, ccp: &[u8]) {
        let Some((&header, rest)) = ccp.split_first() else {
            return;
        };
        let seq = (header >> 6) & 0x03;
        let size_code = header & 0x3F;
        let data_size = if size_code == 0 {
            PACKET_SIZE_ZERO_DATA
        } else {
            (size_code as usize) * 2 - 1
        };
        // discontinuity check (§5.1): non-consecutive seq ⇒ reset every service
        if let Some(prev) = self.last_seq {
            if seq != (prev + 1) & 0x03 {
                for s in &mut self.services {
                    s.reset();
                }
            }
        }
        self.last_seq = Some(seq);
        let end = data_size.min(rest.len());
        self.decode_service_blocks(&rest[..end]);
    }

    /// Walk the service blocks of a CCP (§6.2).
    fn decode_service_blocks(&mut self, mut data: &[u8]) {
        loop {
            let Some((&header, rest)) = data.split_first() else {
                return;
            };
            // Null Service Block Header (§6.2.3): all-zero ⇒ no more blocks.
            if header == 0 {
                return;
            }
            let mut service_number = u16::from((header >> 5) & 0x07);
            let block_size = (header & 0x1F) as usize;
            let mut body = rest;
            if service_number == u16::from(EXTENDED_SERVICE_ESCAPE) && block_size != 0 {
                // Extended Service Block Header (§6.2.2): 2nd byte low 6 bits.
                let Some((&ext, after)) = rest.split_first() else {
                    return;
                };
                service_number = u16::from(ext & 0x3F);
                body = after;
            }
            if block_size > body.len() {
                // truncated block — process what we have, then stop
                self.dispatch_service(service_number, body);
                return;
            }
            let (block, next) = body.split_at(block_size);
            self.dispatch_service(service_number, block);
            data = next;
        }
    }

    fn dispatch_service(&mut self, service_number: u16, block: &[u8]) {
        // We track standard services 1–6 (47 CFR §79.102 (c)).
        if service_number == 0 || service_number as usize > NUM_SERVICES {
            return;
        }
        let idx = service_number as usize - 1;
        Self::interpret(&mut self.services[idx], block);
    }

    /// The C0/C1/G0/G1/G2/G3 command interpreter (§7/§8) for one service block.
    fn interpret(service: &mut Service, block: &[u8]) {
        let mut i = 0usize;
        while i < block.len() {
            let b = block[i];
            let consumed = match b {
                0x00..=C0_END => Self::handle_c0(service, &block[i..]),
                G0_START..=G0_END => {
                    Self::put(service, Self::g0_char(b));
                    1
                }
                C1_START..=C1_END => Self::handle_c1(service, &block[i..]),
                G1_START..=0xFF => {
                    // G1 = ISO 8859-1 Latin-1: byte value is the code point.
                    Self::put(service, char::from(b));
                    1
                }
            };
            i += consumed.max(1);
        }
    }

    /// Handle a C0 control code (§7.1.4). Returns bytes consumed (≥1).
    fn handle_c0(service: &mut Service, data: &[u8]) -> usize {
        let b = data[0];
        match b {
            C0_NUL => 1,
            C0_ETX => 1,
            C0_BS => {
                if let Some(w) = service.current() {
                    w.back_space();
                }
                1
            }
            C0_FF => {
                if let Some(w) = service.current() {
                    w.clear_text();
                }
                1
            }
            C0_CR => {
                if let Some(w) = service.current() {
                    w.carriage_return();
                }
                1
            }
            C0_HCR => {
                if let Some(w) = service.current() {
                    w.horizontal_cr();
                }
                1
            }
            C0_EXT1 => Self::handle_ext1(service, data),
            C0_P16 => 3, // P16: command + 2 bytes (16-bit char addressing)
            // Undefined codes: 0x11–0x17 ⇒ 2 bytes; 0x19–0x1F ⇒ 3 bytes; all
            // other (undefined 0x00–0x0F) ⇒ 1 byte (§7.1.4).
            0x11..=0x17 => 2,
            0x19..=0x1F => 3,
            _ => 1,
        }
    }

    /// EXT1 (0x10) prefix → C2/G2/C3/G3 (§7.1.1). Returns total bytes consumed
    /// including the EXT1 byte.
    fn handle_ext1(service: &mut Service, data: &[u8]) -> usize {
        let Some(&base) = data.get(1) else {
            return 1;
        };
        match base {
            // C2 (0x00–0x1F): EXT1 + base + 0..=3 data bytes (Table 20).
            0x00..=0x07 => 2,
            0x08..=0x0F => 3,
            0x10..=0x17 => 4,
            0x18..=0x1F => 5,
            // G2 (0x20–0x7F): EXT1 + base (two-byte element).
            0x20..=0x7F => {
                Self::put(service, Self::g2_char(base));
                2
            }
            // C3 (0x80–0x9F): fixed/variable length (Tables 22/23).
            0x80..=0x87 => 6,
            0x88..=0x8F => 7,
            0x90..=0x9F => {
                // variable: 1-byte header after the command; N = (data1 & 0x3F)+1.
                let n = data.get(2).map_or(0, |d| (d & 0x3F) as usize + 1);
                3 + n
            }
            // G3 (0xA0–0xFF): EXT1 + base (two-byte element).
            _ => {
                Self::put(service, Self::g3_char(base));
                2
            }
        }
    }

    /// Handle a C1 caption command (§7.1.5 / §8.10.5). Returns bytes consumed.
    fn handle_c1(service: &mut Service, data: &[u8]) -> usize {
        let op = data[0];
        match op {
            C1_CW0..=C1_CW7 => {
                let id = (op - C1_CW0) as usize;
                if service.windows.get(id).and_then(|w| w.as_ref()).is_some() {
                    service.current_window = Some(id);
                }
                1
            }
            C1_CLW => Self::window_map_cmd(service, data, WindowMapOp::Clear),
            C1_DSW => Self::window_map_cmd(service, data, WindowMapOp::Display),
            C1_HDW => Self::window_map_cmd(service, data, WindowMapOp::Hide),
            C1_TGW => Self::window_map_cmd(service, data, WindowMapOp::Toggle),
            C1_DLW => Self::window_map_cmd(service, data, WindowMapOp::Delete),
            C1_DLY => 2, // DLY: command + tenths-of-seconds
            C1_DLC => 1, // DLC: no parameters
            C1_RST => {
                service.reset();
                1
            }
            C1_SPA => Self::set_pen_attributes(service, data),
            C1_SPC => Self::set_pen_color(service, data),
            C1_SPL => Self::set_pen_location(service, data),
            C1_SWA => Self::set_window_attributes(service, data),
            C1_DF0..=C1_DF7 => Self::define_window(service, data),
            // 0x93–0x96 reserved 1-byte window commands (§7.1.5.1).
            _ => 1,
        }
    }

    fn window_map_cmd(service: &mut Service, data: &[u8], op: WindowMapOp) -> usize {
        let Some(&map) = data.get(1) else {
            return 1;
        };
        for id in 0..NUM_WINDOWS {
            if map & (1 << id) == 0 {
                continue;
            }
            match op {
                WindowMapOp::Clear => {
                    if let Some(w) = service.windows[id].as_mut() {
                        w.clear_text();
                    }
                }
                WindowMapOp::Display => {
                    if let Some(w) = service.windows[id].as_mut() {
                        w.state = WindowState::Visible;
                    }
                }
                WindowMapOp::Hide => {
                    if let Some(w) = service.windows[id].as_mut() {
                        w.state = WindowState::Hidden;
                    }
                }
                WindowMapOp::Toggle => {
                    if let Some(w) = service.windows[id].as_mut() {
                        w.state = match w.state {
                            WindowState::Visible => WindowState::Hidden,
                            WindowState::Hidden => WindowState::Visible,
                        };
                    }
                }
                WindowMapOp::Delete => {
                    service.windows[id] = None;
                    if service.current_window == Some(id) {
                        service.current_window = None;
                    }
                }
            }
        }
        2
    }

    /// DefineWindow DF0–DF7 (§8.10.5.2): 6 parameter bytes.
    fn define_window(service: &mut Service, data: &[u8]) -> usize {
        const TOTAL: usize = 7;
        if data.len() < TOTAL {
            return data.len().max(1);
        }
        let id = (data[0] - C1_DF0) as usize;
        let p1 = data[1];
        let p2 = data[2];
        let p3 = data[3];
        let p4 = data[4];
        let p5 = data[5];
        let p6 = data[6];

        let creating = service.windows[id].is_none();
        let w = service.windows[id].get_or_insert_with(Window::new);

        w.priority = p1 & 0x07;
        w.column_lock = (p1 >> 3) & 0x01 != 0;
        w.row_lock = (p1 >> 4) & 0x01 != 0;
        w.state = if (p1 >> 5) & 0x01 != 0 {
            WindowState::Visible
        } else {
            WindowState::Hidden
        };
        w.relative_position = (p2 >> 7) & 0x01 != 0;
        w.anchor_vertical = p2 & 0x7F;
        w.anchor_horizontal = p3;
        w.anchor_point = (p4 >> 4) & 0x0F;
        w.row_count = (p4 & 0x0F) + 1;
        w.column_count = (p5 & 0x3F) + 1;
        w.window_style = (p6 >> 3) & 0x07;
        w.pen_style = p6 & 0x07;

        if creating {
            // On create: apply preset window/pen styles, fill, pen at (0,0).
            apply_window_style(
                w,
                if w.window_style == 0 {
                    1
                } else {
                    w.window_style
                },
            );
            apply_pen_style(w, if w.pen_style == 0 { 1 } else { w.pen_style });
            w.ensure_grid();
            w.clear_text();
        } else {
            // On update: a non-zero style preset is re-applied; pen unaffected.
            if w.window_style != 0 {
                apply_window_style(w, w.window_style);
            }
            if w.pen_style != 0 {
                apply_pen_style(w, w.pen_style);
            }
            w.ensure_grid();
        }
        service.current_window = Some(id);
        TOTAL
    }

    /// SetWindowAttributes SWA (§8.10.5.8): 4 parameter bytes.
    fn set_window_attributes(service: &mut Service, data: &[u8]) -> usize {
        const TOTAL: usize = 5;
        if data.len() < TOTAL {
            return data.len().max(1);
        }
        let p1 = data[1];
        let p2 = data[2];
        let p3 = data[3];
        let p4 = data[4];
        if let Some(w) = service.current() {
            w.fill_opacity = Opacity::from_bits((p1 >> 6) & 0x03);
            w.fill_color = Color::new((p1 >> 4) & 0x03, (p1 >> 2) & 0x03, p1 & 0x03);
            let bt_lo = (p2 >> 6) & 0x03;
            w.border_color = Color::new((p2 >> 4) & 0x03, (p2 >> 2) & 0x03, p2 & 0x03);
            let bt_hi = (p3 >> 7) & 0x01;
            w.border_type = (bt_hi << 2) | bt_lo;
            w.word_wrap = (p3 >> 6) & 0x01 != 0;
            w.print_direction = PrintDirection::from_bits((p3 >> 4) & 0x03);
            w.scroll_direction = ScrollDirection::from_bits((p3 >> 2) & 0x03);
            w.justify = Justify::from_bits(p3 & 0x03);
            // p4: effect speed / direction / display effect — not rendered here.
            let _ = p4;
        }
        TOTAL
    }

    /// SetPenAttributes SPA (§8.10.5.9): 2 parameter bytes.
    fn set_pen_attributes(service: &mut Service, data: &[u8]) -> usize {
        const TOTAL: usize = 3;
        if data.len() < TOTAL {
            return data.len().max(1);
        }
        let p1 = data[1];
        let p2 = data[2];
        if let Some(w) = service.current() {
            w.pen_offset = PenOffset::from_bits((p1 >> 2) & 0x03);
            w.pen_size = PenSize::from_bits(p1 & 0x03);
            w.italics = (p2 >> 7) & 0x01 != 0;
            w.underline = (p2 >> 6) & 0x01 != 0;
            w.edge_type = EdgeType::from_bits((p2 >> 3) & 0x07);
            w.font_style = FontStyle::from_bits(p2 & 0x07);
        }
        TOTAL
    }

    /// SetPenColor SPC (§8.10.5.10): 3 parameter bytes.
    fn set_pen_color(service: &mut Service, data: &[u8]) -> usize {
        const TOTAL: usize = 4;
        if data.len() < TOTAL {
            return data.len().max(1);
        }
        let p1 = data[1];
        let p2 = data[2];
        let p3 = data[3];
        if let Some(w) = service.current() {
            w.fg_opacity = Opacity::from_bits((p1 >> 6) & 0x03);
            w.fg_color = Color::new((p1 >> 4) & 0x03, (p1 >> 2) & 0x03, p1 & 0x03);
            w.bg_opacity = Opacity::from_bits((p2 >> 6) & 0x03);
            w.bg_color = Color::new((p2 >> 4) & 0x03, (p2 >> 2) & 0x03, p2 & 0x03);
            // p3 = edge colour
            w.border_color = Color::new((p3 >> 4) & 0x03, (p3 >> 2) & 0x03, p3 & 0x03);
        }
        TOTAL
    }

    /// SetPenLocation SPL (§8.10.5.11): 2 parameter bytes.
    fn set_pen_location(service: &mut Service, data: &[u8]) -> usize {
        const TOTAL: usize = 3;
        if data.len() < TOTAL {
            return data.len().max(1);
        }
        let row = (data[1] & 0x0F) as usize;
        let col = (data[2] & 0x3F) as usize;
        if let Some(w) = service.current() {
            w.set_pen_location(row, col);
        }
        TOTAL
    }

    fn put(service: &mut Service, ch: char) {
        if let Some(w) = service.current() {
            w.put_char(ch);
        }
    }

    /// G0 byte → glyph (§7.1.6): ASCII printable, 0x7F = musical note ♪.
    fn g0_char(b: u8) -> char {
        if b == G0_MUSIC_NOTE {
            '\u{266A}'
        } else {
            char::from(b)
        }
    }

    /// G2 byte → glyph (§7.1.8 / Table 17), with substitution for the rest.
    fn g2_char(b: u8) -> char {
        match b {
            0x20 | 0x21 => ' ', // TSP / NBTSP — transparent space
            0x25 => '\u{2026}', // …
            0x2A => '\u{0160}', // Š
            0x2C => '\u{0152}', // Œ
            0x30 => '\u{25A0}', // ■ solid block
            0x31 => '\u{2018}', // ‘
            0x32 => '\u{2019}', // ’
            0x33 => '\u{201C}', // "
            0x34 => '\u{201D}', // "
            0x35 => '\u{2022}', // • bullet
            0x39 => '\u{2122}', // ™
            0x3A => '\u{0161}', // š
            0x3C => '\u{0153}', // œ
            0x3D => '\u{2120}', // ℠
            0x3F => '\u{0178}', // Ÿ
            0x76 => '\u{215B}', // ⅛
            0x77 => '\u{215C}', // ⅜
            0x78 => '\u{215D}', // ⅝
            0x79 => '\u{215E}', // ⅞
            _ => '_',           // unsupported G2 ⇒ underscore (Table 28 floor)
        }
    }

    /// G3 byte → glyph (§7.1.9): 0xA0 = [CC] icon; the rest substitute `_`.
    fn g3_char(b: u8) -> char {
        if b == 0xA0 {
            '\u{1F4FA}' // 📺 stand-in for the [CC] icon
        } else {
            '_'
        }
    }

    /// Read the windows of a service (`1`–`6`). Returns an empty array view for
    /// an out-of-range service number.
    #[must_use]
    pub fn windows(&self, service_number: usize) -> &[Option<Window>; NUM_WINDOWS] {
        const EMPTY: [Option<Window>; NUM_WINDOWS] =
            [None, None, None, None, None, None, None, None];
        if service_number == 0 || service_number > NUM_SERVICES {
            return &EMPTY;
        }
        &self.services[service_number - 1].windows
    }

    /// All decoded text for a service (`1`–`6`), visible-window text joined with
    /// `\n` in window-priority order (0 = highest first), then by window ID.
    #[must_use]
    pub fn service_text(&self, service_number: usize) -> String {
        if service_number == 0 || service_number > NUM_SERVICES {
            return String::new();
        }
        let svc = &self.services[service_number - 1];
        let mut idxs: Vec<usize> = (0..NUM_WINDOWS)
            .filter(|&i| {
                svc.windows[i]
                    .as_ref()
                    .is_some_and(|w| w.state == WindowState::Visible)
            })
            .collect();
        idxs.sort_by_key(|&i| {
            svc.windows[i]
                .as_ref()
                .map_or((u8::MAX, i), |w| (w.priority, i))
        });
        let mut out = String::new();
        for i in idxs {
            if let Some(w) = svc.windows[i].as_ref() {
                let t = w.text();
                if t.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t);
            }
        }
        out
    }
}

/// The window-map command kinds that share the CLW/DSW/HDW/TGW/DLW bitmap byte.
#[derive(Clone, Copy)]
enum WindowMapOp {
    Clear,
    Display,
    Hide,
    Toggle,
    Delete,
}

/// Apply a predefined window style 1–7 (Table 26).
fn apply_window_style(w: &mut Window, id: u8) {
    // All presets: print dir L→R (except 7), scroll BOTTOM→TOP (except 7),
    // border NONE, display effect SNAP. justify + wordwrap + fill vary.
    w.border_type = 0;
    match id {
        1 => style(w, Justify::Left, false, Some(Color::BLACK), Opacity::Solid),
        2 => style(w, Justify::Left, false, None, Opacity::Transparent),
        3 => style(
            w,
            Justify::Center,
            false,
            Some(Color::BLACK),
            Opacity::Solid,
        ),
        4 => style(w, Justify::Left, true, Some(Color::BLACK), Opacity::Solid),
        5 => style(w, Justify::Left, true, None, Opacity::Transparent),
        6 => style(w, Justify::Center, true, Some(Color::BLACK), Opacity::Solid),
        7 => {
            w.justify = Justify::Left;
            w.word_wrap = false;
            w.print_direction = PrintDirection::TopToBottom;
            w.scroll_direction = ScrollDirection::RightToLeft;
            w.fill_color = Color::BLACK;
            w.fill_opacity = Opacity::Solid;
        }
        _ => {}
    }
}

fn style(w: &mut Window, j: Justify, ww: bool, fill: Option<Color>, op: Opacity) {
    w.justify = j;
    w.word_wrap = ww;
    w.print_direction = PrintDirection::LeftToRight;
    w.scroll_direction = ScrollDirection::BottomToTop;
    w.fill_opacity = op;
    if let Some(c) = fill {
        w.fill_color = c;
    }
}

/// Apply a predefined pen style 1–7 (Table 27).
fn apply_pen_style(w: &mut Window, id: u8) {
    w.pen_size = PenSize::Standard;
    w.pen_offset = PenOffset::Normal;
    w.italics = false;
    w.underline = false;
    w.fg_color = Color::WHITE;
    w.fg_opacity = Opacity::Solid;
    match id {
        1 => pen(
            w,
            FontStyle::Default,
            EdgeType::None,
            Color::BLACK,
            Opacity::Solid,
        ),
        2 => pen(
            w,
            FontStyle::MonospacedSerif,
            EdgeType::None,
            Color::BLACK,
            Opacity::Solid,
        ),
        3 => pen(
            w,
            FontStyle::ProportionalSerif,
            EdgeType::None,
            Color::BLACK,
            Opacity::Solid,
        ),
        4 => pen(
            w,
            FontStyle::MonospacedSansSerif,
            EdgeType::None,
            Color::BLACK,
            Opacity::Solid,
        ),
        5 => pen(
            w,
            FontStyle::ProportionalSansSerif,
            EdgeType::None,
            Color::BLACK,
            Opacity::Solid,
        ),
        6 => pen(
            w,
            FontStyle::MonospacedSansSerif,
            EdgeType::Uniform,
            Color::BLACK,
            Opacity::Transparent,
        ),
        7 => pen(
            w,
            FontStyle::ProportionalSansSerif,
            EdgeType::Uniform,
            Color::BLACK,
            Opacity::Transparent,
        ),
        _ => {}
    }
}

fn pen(w: &mut Window, font: FontStyle, edge: EdgeType, bg: Color, bg_op: Opacity) {
    w.font_style = font;
    w.edge_type = edge;
    w.bg_color = bg;
    w.bg_opacity = bg_op;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a single-service CCP carrying `cmds` as service `svc`'s block.
    fn ccp(svc: u8, cmds: &[u8]) -> Vec<u8> {
        let mut sb = alloc::vec![(svc << 5) | (cmds.len() as u8)];
        sb.extend_from_slice(cmds);
        // size_code = number of byte-pairs including the header byte.
        let size_code = (sb.len().div_ceil(2) + 1) as u8 & 0x3F;
        let mut packet = alloc::vec![size_code];
        packet.extend_from_slice(&sb);
        packet
    }

    /// CTA-708-E DefineWindow worked example (`cea708-decode.md`, p.66–67):
    /// `0x9A 38 4A D1 8B 0F 11` → window id=2, visible=YES, rl=YES, cl=YES,
    /// priority=0, rp=0, av=74, ah=209, ap=8, rc=11 (→12 rows), cc=15 (→16 cols),
    /// ws=2, ps=1.
    #[test]
    fn define_window_worked_example() {
        let mut dec = Cea708Decoder::new();
        let packet = ccp(1, &[0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11]);
        dec.push_packet(&packet);
        let w = dec.windows(1)[2].as_ref().expect("window 2 defined");
        assert_eq!(w.state, WindowState::Visible);
        assert!(w.row_lock);
        assert!(w.column_lock);
        assert_eq!(w.priority, 0);
        assert!(!w.relative_position);
        assert_eq!(w.anchor_vertical, 74);
        assert_eq!(w.anchor_horizontal, 209);
        assert_eq!(w.anchor_point, 8);
        assert_eq!(w.row_count, 12);
        assert_eq!(w.column_count, 16);
        assert_eq!(w.window_style, 2);
        assert_eq!(w.pen_style, 1);
    }

    /// SWA worked example (`cea708-decode.md`, p.76):
    /// `0x97,0x64,0x53,0x88,0x22` → border type = 5 (SHADOW_RIGHT).
    #[test]
    fn swa_border_type_split() {
        let mut dec = Cea708Decoder::new();
        // define a window first (so there is a current window), then SWA.
        let packet = ccp(
            1,
            &[
                0x98, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, // DF0 visible, defaults
                0x97, 0x64, 0x53, 0x88, 0x22, // SWA
            ],
        );
        dec.push_packet(&packet);
        let w = dec.windows(1)[0].as_ref().expect("window 0");
        assert_eq!(w.border_type, 5);
    }

    /// A simple caption: define a window (visible), write "Hi", read service text.
    #[test]
    fn decode_text() {
        let mut dec = Cea708Decoder::new();
        let packet = ccp(
            1,
            &[
                0x98, 0x20, 0x00, 0x00, 0x02, 0x0F, 0x00, // DF0 visible, 3 rows × 16
                b'H', b'i',
            ],
        );
        dec.push_packet(&packet);
        assert_eq!(dec.service_text(1), "Hi");
    }

    /// ≥2 services + multi-window exercised in one packet.
    #[test]
    fn two_services_multi_window() {
        let mut dec = Cea708Decoder::new();
        // Service 1: define window 0 visible, write "S1".
        let s1_block = [0x98, 0x20, 0x00, 0x00, 0x00, 0x0F, 0x00, b'S', b'1'];
        // Service 2: define window 1 visible, write "S2".
        let s2_block = [0x99, 0x20, 0x00, 0x00, 0x00, 0x0F, 0x00, b'S', b'2'];
        let mut data = Vec::new();
        data.push((1 << 5) | (s1_block.len() as u8));
        data.extend_from_slice(&s1_block);
        data.push((2 << 5) | (s2_block.len() as u8));
        data.extend_from_slice(&s2_block);
        let size_code = (data.len().div_ceil(2) + 1) as u8 & 0x3F;
        let mut packet = alloc::vec![size_code];
        packet.extend_from_slice(&data);
        dec.push_packet(&packet);
        assert_eq!(dec.service_text(1), "S1");
        assert_eq!(dec.service_text(2), "S2");
        assert!(dec.windows(1)[0].is_some());
        assert!(dec.windows(2)[1].is_some());
    }

    #[test]
    fn carriage_return_rolls_up() {
        let mut w = Window::new();
        w.row_count = 2;
        w.column_count = 10;
        w.ensure_grid();
        w.put_char('A');
        w.carriage_return();
        w.put_char('B');
        w.carriage_return(); // now at bottom; should roll up
        w.put_char('C');
        assert_eq!(w.text(), "B\nC");
    }

    #[test]
    fn g0_music_note() {
        assert_eq!(Cea708Decoder::g0_char(0x7F), '\u{266A}');
        assert_eq!(Cea708Decoder::g0_char(b'A'), 'A');
    }

    #[test]
    fn no_panic_on_arbitrary_input() {
        // Feed adversarial / truncated / malformed bytes; must never panic.
        let inputs: &[&[u8]] = &[
            &[],
            &[0x00],
            &[0xFF],
            &[0x01, 0x98],                               // DefineWindow truncated
            &[0x3F, 0x80, 0x90, 0x91, 0x92, 0x97, 0x98], // size_code huge, partial cmds
            &[0x20, 0xEE, (7 << 5) | 1],                 // extended service escape truncated
            &[0x10, 0x9A],                               // C0 EXT1 → C3 variable, truncated
            &[0x18, 0x00],                               // P16 truncated
        ];
        for inp in inputs {
            let mut dec = Cea708Decoder::new();
            dec.push_packet(inp);
        }
        // a long pseudo-random stream
        let mut dec = Cea708Decoder::new();
        let mut x: u32 = 0x1234_5678;
        let mut buf = Vec::new();
        for _ in 0..4096 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            buf.push((x >> 16) as u8);
        }
        dec.push_packet(&buf);
        // also drive it via triplets
        let mut dec2 = Cea708Decoder::new();
        let triplets: Vec<CcTriplet> = buf
            .chunks(2)
            .map(|c| CcTriplet {
                cc_valid: true,
                cc_type: CcType::Dtvcc708Data,
                cc_data_1: c[0],
                cc_data_2: *c.get(1).unwrap_or(&0),
            })
            .collect();
        dec2.push_triplets(&triplets);
    }
}
