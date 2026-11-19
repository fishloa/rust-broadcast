#![cfg(feature = "decode")]

//! CEA-608 decoder semantics — ANSI/CTA-608-E Tables 49–53 and the
//! RCL/BS/DER/RU2–4/FON/RDC/TR/RTD/EDM/CR/ENM/EOC misc-control state machine
//! (`private/specs-verified/captions/cc-data/decode/cea608-decode.md`).
//!
//! Each case asserts the exact screen state (row, column, glyph, colour,
//! italics, underline) after a single code, so a mis-decoded PAC, character
//! table, mid-row style or erase behaviour cannot pass on aggregate counts.

use broadcast_common::parity::set_odd_parity;
use cc_data::decode::{
    Cea608Channel, Cea608Color, Cea608Decoder, Cea608Mode, Cea608Screen, Cea608StyledChar,
};
use cc_data::{CcTriplet, CcType};

// Misc-control second bytes (Table 52).
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

const COLORS: [Cea608Color; 8] = [
    Cea608Color::White,
    Cea608Color::Green,
    Cea608Color::Blue,
    Cea608Color::Cyan,
    Cea608Color::Red,
    Cea608Color::Yellow,
    Cea608Color::Magenta,
    Cea608Color::White,
];

fn feed(dec: &mut Cea608Decoder, pairs: &[(u8, u8)]) {
    for &(a, b) in pairs {
        dec.push_pair(false, set_odd_parity(a), set_odd_parity(b));
    }
}

fn cell(screen: &Cea608Screen, row: usize, col: usize) -> Option<Cea608StyledChar> {
    screen.rows()[row]
        .styled_cells()
        .into_iter()
        .find(|(c, _)| *c == col)
        .map(|(_, s)| s)
}

fn row_of(screen: &Cea608Screen, ch: char) -> Option<usize> {
    screen
        .rows()
        .iter()
        .position(|row| row.styled_cells().into_iter().any(|(_, c)| c.ch == ch))
}

#[test]
fn pac_row_groups_and_pair_selection() {
    // (first byte, first-of-pair row, second-of-pair row), Table 53.
    let table = [
        (0x11u8, 1usize, 2usize),
        (0x12, 3, 4),
        (0x15, 5, 6),
        (0x16, 7, 8),
        (0x17, 9, 10),
        (0x10, 11, 11),
        (0x13, 12, 13),
        (0x14, 14, 15),
    ];
    for (b1, first, second) in table {
        for (b2, expected) in [(0x40u8, first), (0x60u8, second)] {
            let mut dec = Cea608Decoder::new();
            feed(&mut dec, &[(b1, b2), (b'X', 0x00)]);
            let screen = dec.screen(Cea608Channel::Cc1);
            assert_eq!(
                row_of(screen, 'X'),
                Some(expected - 1),
                "b1={b1:#04X} b2={b2:#04X} expected row {expected}"
            );
        }
    }
}

#[test]
fn pac_c2_first_byte_folds_onto_cc2() {
    // 0x18..=0x1F are the data-channel-2 copies of 0x10..=0x17.
    let table = [
        (0x18u8, 11usize),
        (0x19, 1),
        (0x1A, 3),
        (0x1B, 12),
        (0x1C, 14),
        (0x1D, 5),
        (0x1E, 7),
        (0x1F, 9),
    ];
    for (b1, row) in table {
        let mut dec = Cea608Decoder::new();
        feed(&mut dec, &[(b1, 0x40), (b'X', 0x00)]);
        assert_eq!(
            row_of(dec.screen(Cea608Channel::Cc2), 'X'),
            Some(row - 1),
            "b1={b1:#04X} should fold to row {row} on CC2"
        );
        assert_eq!(dec.screen(Cea608Channel::Cc1).text(), "");
    }
}

#[test]
fn pac_colour_italics_and_underline_codes() {
    for attr in 0u8..0x10 {
        let b2 = 0x40 | attr;
        let mut dec = Cea608Decoder::new();
        feed(&mut dec, &[(0x11, b2), (b'X', 0x00)]);
        let got = cell(dec.screen(Cea608Channel::Cc1), 0, 0).expect("X at (0,0)");
        let idx = (attr >> 1) as usize;
        let italics = idx == 7;
        assert_eq!(got.ch, 'X', "attr={attr:#04X}");
        assert_eq!(got.color, COLORS[idx], "attr={attr:#04X}");
        assert_eq!(got.italics, italics, "attr={attr:#04X}");
        assert_eq!(got.underline, attr & 0x01 != 0, "attr={attr:#04X}");
    }
}

#[test]
fn pac_indent_codes_0_to_28_in_steps_of_4() {
    for attr in 0x10u8..0x20 {
        let b2 = 0x40 | attr;
        let mut dec = Cea608Decoder::new();
        feed(&mut dec, &[(0x11, b2), (b'X', 0x00)]);
        let expected_col = (((attr - 0x10) >> 1) as usize) * 4;
        let got = cell(dec.screen(Cea608Channel::Cc1), 0, expected_col)
            .unwrap_or_else(|| panic!("no X at column {expected_col} for attr {attr:#04X}"));
        assert_eq!(got.ch, 'X');
        assert_eq!(got.color, Cea608Color::White);
        assert!(!got.italics);
        assert_eq!(got.underline, attr & 0x01 != 0);
    }
}

#[test]
fn mid_row_codes_set_style_and_occupy_one_cell() {
    for i in 0u8..0x10 {
        let b2 = 0x20 | i;
        let mut dec = Cea608Decoder::new();
        feed(
            &mut dec,
            &[(0x11, 0x40), (b'A', 0x00), (0x11, b2), (b'B', 0x00)],
        );
        let screen = dec.screen(Cea608Channel::Cc1);
        let space = cell(screen, 0, 1).expect("mid-row space at column 1");
        let b = cell(screen, 0, 2).expect("B at column 2");
        let idx = i as usize;
        let expected_color = COLORS[idx >> 1];
        let expected_italics = idx >> 1 == 7;
        assert_eq!(space.ch, ' ');
        assert_eq!(b.ch, 'B');
        for c in [space, b] {
            assert_eq!(c.color, expected_color, "mid-row {b2:#04X}");
            assert_eq!(c.italics, expected_italics, "mid-row {b2:#04X}");
            assert_eq!(c.underline, idx & 1 != 0, "mid-row {b2:#04X}");
        }
    }
}

#[test]
fn special_characters_table_49() {
    let table = [
        (0x30u8, '\u{00AE}'),
        (0x31, '\u{00B0}'),
        (0x32, '\u{00BD}'),
        (0x33, '\u{00BF}'),
        (0x34, '\u{2122}'),
        (0x35, '\u{00A2}'),
        (0x36, '\u{00A3}'),
        (0x37, '\u{266A}'),
        (0x38, '\u{00E0}'),
        (0x39, ' '),
        (0x3A, '\u{00E8}'),
        (0x3B, '\u{00E2}'),
        (0x3C, '\u{00EA}'),
        (0x3D, '\u{00EE}'),
        (0x3E, '\u{00F4}'),
        (0x3F, '\u{00FB}'),
    ];
    for (b2, expected) in table {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, set_odd_parity(0x14), set_odd_parity(MC_RDC));
        dec.push_pair(false, set_odd_parity(0x11), set_odd_parity(b2));
        assert_eq!(row_of(dec.screen(Cea608Channel::Cc1), expected), Some(0));
    }
}

#[test]
fn standard_characters_table_50_accents() {
    let table = [
        (0x2Au8, '\u{00E1}'),
        (0x5C, '\u{00E9}'),
        (0x5E, '\u{00ED}'),
        (0x5F, '\u{00F3}'),
        (0x60, '\u{00FA}'),
        (0x7B, '\u{00E7}'),
        (0x7C, '\u{00F7}'),
        (0x7D, '\u{00D1}'),
        (0x7E, '\u{00F1}'),
        (0x7F, '\u{25A0}'),
    ];
    for (b, expected) in table {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, set_odd_parity(0x14), set_odd_parity(MC_RDC));
        dec.push_pair(false, set_odd_parity(b), set_odd_parity(0x00));
        assert_eq!(
            row_of(dec.screen(Cea608Channel::Cc1), expected),
            Some(0),
            "b={b:#04X}"
        );
    }
}

fn assert_extended_block(first: u8, table: &[(u8, char)]) {
    let channel = if first >= 0x18 {
        Cea608Channel::Cc2
    } else {
        Cea608Channel::Cc1
    };
    for &(b2, expected) in table {
        let mut dec = Cea608Decoder::new();
        dec.push_pair(false, set_odd_parity(0x14), set_odd_parity(MC_RDC));
        dec.push_pair(false, set_odd_parity(b'x'), set_odd_parity(0x00));
        dec.push_pair(false, set_odd_parity(first), set_odd_parity(b2));
        let screen = dec.screen(channel);
        let cells = screen.rows()[0].styled_cells();
        assert_eq!(
            cells.len(),
            1,
            "backspace should erase the fallback, b2={b2:#04X}"
        );
        assert_eq!(
            cells[0],
            (
                0,
                Cea608StyledChar {
                    ch: expected,
                    underline: false,
                    italics: false,
                    color: Cea608Color::White,
                }
            ),
            "b2={b2:#04X}"
        );
    }
}

#[test]
fn extended_chars_block1_tables_5_to_7() {
    let table = [
        (0x20u8, '\u{00C1}'),
        (0x21, '\u{00C9}'),
        (0x22, '\u{00D3}'),
        (0x23, '\u{00DA}'),
        (0x24, '\u{00DC}'),
        (0x25, '\u{00FC}'),
        (0x26, '\u{2018}'),
        (0x27, '\u{00A1}'),
        (0x28, '*'),
        (0x29, '\''),
        (0x2A, '\u{2014}'),
        (0x2B, '\u{00A9}'),
        (0x2C, '\u{2120}'),
        (0x2D, '\u{2022}'),
        (0x2E, '\u{201C}'),
        (0x2F, '\u{201D}'),
        (0x30, '\u{00C0}'),
        (0x31, '\u{00C2}'),
        (0x32, '\u{00C7}'),
        (0x33, '\u{00C8}'),
        (0x34, '\u{00CA}'),
        (0x35, '\u{00CB}'),
        (0x36, '\u{00EB}'),
        (0x37, '\u{00CE}'),
        (0x38, '\u{00CF}'),
        (0x39, '\u{00EF}'),
        (0x3A, '\u{00D4}'),
        (0x3B, '\u{00D9}'),
        (0x3C, '\u{00F9}'),
        (0x3D, '\u{00DB}'),
        (0x3E, '\u{00AB}'),
        (0x3F, '\u{00BB}'),
    ];
    assert_extended_block(0x12, &table);
    assert_extended_block(0x1A, &table);
}

#[test]
fn extended_chars_block2_tables_8_to_10() {
    let table = [
        (0x20u8, '\u{00C3}'),
        (0x21, '\u{00E3}'),
        (0x22, '\u{00CD}'),
        (0x23, '\u{00CC}'),
        (0x24, '\u{00EC}'),
        (0x25, '\u{00D2}'),
        (0x26, '\u{00F2}'),
        (0x27, '\u{00D5}'),
        (0x28, '\u{00F5}'),
        (0x29, '{'),
        (0x2A, '}'),
        (0x2B, '\\'),
        (0x2C, '^'),
        (0x2D, '_'),
        (0x2E, '|'),
        (0x2F, '~'),
        (0x30, '\u{00C4}'),
        (0x31, '\u{00E4}'),
        (0x32, '\u{00D6}'),
        (0x33, '\u{00F6}'),
        (0x34, '\u{00DF}'),
        (0x35, '\u{00A5}'),
        (0x36, '\u{00A4}'),
        (0x37, '|'),
        (0x38, '\u{00C5}'),
        (0x39, '\u{00E5}'),
        (0x3A, '\u{00D8}'),
        (0x3B, '\u{00F8}'),
        (0x3C, '\u{231C}'),
        (0x3D, '\u{231D}'),
        (0x3E, '\u{231E}'),
        (0x3F, '\u{231F}'),
    ];
    assert_extended_block(0x13, &table);
    assert_extended_block(0x1B, &table);
}

fn with_control(code: u8) -> Cea608Decoder {
    let mut dec = Cea608Decoder::new();
    dec.push_pair(false, set_odd_parity(0x14), set_odd_parity(code));
    dec
}

#[test]
fn misc_control_modes() {
    assert_eq!(
        with_control(MC_RCL).mode(Cea608Channel::Cc1),
        Cea608Mode::PopOn
    );
    assert_eq!(
        with_control(MC_RU2).mode(Cea608Channel::Cc1),
        Cea608Mode::RollUp(2)
    );
    assert_eq!(
        with_control(MC_RU3).mode(Cea608Channel::Cc1),
        Cea608Mode::RollUp(3)
    );
    assert_eq!(
        with_control(MC_RU4).mode(Cea608Channel::Cc1),
        Cea608Mode::RollUp(4)
    );
    assert_eq!(
        with_control(MC_RDC).mode(Cea608Channel::Cc1),
        Cea608Mode::PaintOn
    );
    assert_eq!(
        with_control(MC_TR).mode(Cea608Channel::Cc1),
        Cea608Mode::Text
    );
    assert_eq!(
        with_control(MC_RTD).mode(Cea608Channel::Cc1),
        Cea608Mode::Text
    );
    // FON is a no-op that must not disturb the selected mode.
    let mut dec = Cea608Decoder::new();
    feed(&mut dec, &[(0x14, MC_RDC), (0x14, MC_FON)]);
    assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::PaintOn);
}

#[test]
fn roll_up_carriage_return_scrolls_the_window_by_its_row_count() {
    for (code, rows) in [(MC_RU2, 2usize), (MC_RU3, 3), (MC_RU4, 4)] {
        let mut dec = Cea608Decoder::new();
        feed(
            &mut dec,
            &[(0x14, code), (b'O', b'L'), (0x14, MC_CR), (b'N', b'E')],
        );
        let screen = dec.screen(Cea608Channel::Cc1);
        assert_eq!(row_of(screen, 'N'), Some(14), "rows={rows}");
        assert_eq!(row_of(screen, 'O'), Some(13), "rows={rows}");
        // The row above the window top is untouched, and only `rows` rows move.
        let top = 14 - (rows - 1);
        if top > 0 {
            assert_eq!(screen.rows()[top - 1].text(), "", "rows={rows}");
        }
    }
}

#[test]
fn carriage_return_outside_roll_up_moves_down_one_row() {
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[(0x11, 0x40), (b'A', 0x00), (0x14, MC_CR), (b'B', 0x00)],
    );
    let screen = dec.screen(Cea608Channel::Cc1);
    assert_eq!(cell(screen, 0, 0).map(|c| c.ch), Some('A'));
    assert_eq!(cell(screen, 1, 0).map(|c| c.ch), Some('B'));
}

#[test]
fn backspace_erases_the_cell_before_the_cursor() {
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[(0x14, MC_RDC), (b'A', b'B'), (b'C', 0x00), (0x14, MC_BS)],
    );
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "AB");

    // Backspace at column 0 is a no-op (never underflows).
    let mut dec = Cea608Decoder::new();
    feed(&mut dec, &[(0x14, MC_RDC), (0x14, MC_BS), (b'A', 0x00)]);
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "A");
}

#[test]
fn delete_to_end_of_row_clears_from_the_cursor() {
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[
            (0x14, MC_RDC),
            (b'A', b'B'),
            (b'C', b'D'),
            (0x11, 0x40),
            (0x14, MC_DER),
        ],
    );
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "");
}

#[test]
fn erase_displayed_and_non_displayed_memories() {
    let mut visible = Cea608Decoder::new();
    feed(&mut visible, &[(0x14, MC_RDC), (b'H', b'I')]);
    assert_eq!(visible.channel_text(Cea608Channel::Cc1), "HI");
    visible.push_pair(false, set_odd_parity(0x14), set_odd_parity(MC_EDM));
    assert_eq!(visible.channel_text(Cea608Channel::Cc1), "");

    let mut hidden = Cea608Decoder::new();
    feed(
        &mut hidden,
        &[
            (0x14, MC_RCL),
            (0x11, 0x70),
            (b'H', b'I'),
            (0x14, MC_ENM),
            (0x14, MC_EOC),
        ],
    );
    assert_eq!(hidden.channel_text(Cea608Channel::Cc1), "");
}

#[test]
fn end_of_caption_swaps_the_memories_once() {
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[(0x14, MC_RCL), (0x11, 0x70), (b'H', b'I'), (0x14, MC_EOC)],
    );
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");
    // A null pair separates the two EOCs (an immediately repeated control is
    // a doubled copy and acts once), then EOC swaps back to the now-empty
    // non-displayed screen.
    dec.push_pair(false, set_odd_parity(0x00), set_odd_parity(0x00));
    dec.push_pair(false, set_odd_parity(0x14), set_odd_parity(MC_EOC));
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "");
}

#[test]
fn tab_offsets_advance_the_cursor() {
    for (b2, n) in [(0x21u8, 1usize), (0x22, 2), (0x23, 3)] {
        let mut dec = Cea608Decoder::new();
        feed(
            &mut dec,
            &[(0x11, 0x40), (b'A', b'B'), (0x17, b2), (b'C', 0x00)],
        );
        assert_eq!(
            cell(dec.screen(Cea608Channel::Cc1), 0, 2 + n).map(|c| c.ch),
            Some('C'),
            "tab {b2:#04X}"
        );
    }
    // Data-channel-2 tab first byte 0x1F (the 0x1C PAC selects row 15).
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[(0x1C, 0x40), (b'A', 0x00), (0x1F, 0x22), (b'B', 0x00)],
    );
    assert_eq!(
        cell(dec.screen(Cea608Channel::Cc2), 13, 3).map(|c| c.ch),
        Some('B')
    );

    // At the right edge the cursor clamps to column 31, so the next char fits.
    let mut dec = Cea608Decoder::new();
    feed(
        &mut dec,
        &[(0x11, 0x5E), (b'A', 0x00), (0x17, 0x23), (b'Z', 0x00)],
    );
    assert_eq!(
        cell(dec.screen(Cea608Channel::Cc1), 0, 31).map(|c| c.ch),
        Some('Z')
    );
}

#[test]
fn repeated_control_codes_are_applied_once() {
    let mut dec = Cea608Decoder::new();
    feed(&mut dec, &[(0x14, MC_RDC), (0x14, MC_RDC)]);
    assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::PaintOn);
    // A null pair separates two identical controls, so both act.
    feed(&mut dec, &[(0x14, MC_EDM), (0x00, 0x00), (0x14, MC_EDM)]);
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "");
}

fn tr(cc_valid: bool, cc_type: CcType, d1: u8, d2: u8) -> CcTriplet {
    CcTriplet {
        cc_valid,
        cc_type,
        cc_data_1: set_odd_parity(d1),
        cc_data_2: set_odd_parity(d2),
    }
}

#[test]
fn push_triplets_routes_fields_and_skips_invalid() {
    let field1 = [
        tr(true, CcType::Ntsc608Field1, 0x14, MC_RCL),
        tr(true, CcType::Ntsc608Field1, 0x11, 0x70),
        tr(true, CcType::Ntsc608Field1, b'H', b'I'),
        tr(true, CcType::Ntsc608Field1, 0x14, MC_EOC),
    ];
    let mut dec = Cea608Decoder::new();
    dec.push_triplets(&field1);
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");

    // Invalid triplets carry no caption data.
    let mut dec = Cea608Decoder::new();
    dec.push_triplets(&[
        tr(false, CcType::Ntsc608Field1, 0x14, MC_RDC),
        tr(false, CcType::Ntsc608Field1, b'N', b'O'),
    ]);
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "");

    // DTVCC-typed triplets are not 608 line-21 data.
    let mut dec = Cea608Decoder::new();
    dec.push_triplets(&[tr(true, CcType::Dtvcc708Data, 0x14, MC_RDC)]);
    assert_eq!(dec.mode(Cea608Channel::Cc1), Cea608Mode::None);

    // Field 2 routes to CC3/CC4.
    let mut dec = Cea608Decoder::new();
    dec.push_triplets(&[
        tr(true, CcType::Ntsc608Field2, 0x15, MC_RDC),
        tr(true, CcType::Ntsc608Field2, 0x15, 0x70),
        tr(true, CcType::Ntsc608Field2, b'H', b'I'),
    ]);
    assert_eq!(dec.channel_text(Cea608Channel::Cc3), "HI");
    assert_eq!(dec.channel_text(Cea608Channel::Cc1), "");
}

#[test]
fn channel_labels_and_numbers() {
    let table = [
        (Cea608Channel::Cc1, 1u8, "cc1"),
        (Cea608Channel::Cc2, 2, "cc2"),
        (Cea608Channel::Cc3, 3, "cc3"),
        (Cea608Channel::Cc4, 4, "cc4"),
    ];
    for (ch, number, name) in table {
        assert_eq!(ch.number(), number);
        assert_eq!(ch.name(), name);
    }
}

#[test]
fn mode_labels() {
    let table = [
        (Cea608Mode::None, "none"),
        (Cea608Mode::PopOn, "pop_on"),
        (Cea608Mode::RollUp(3), "roll_up"),
        (Cea608Mode::PaintOn, "paint_on"),
        (Cea608Mode::Text, "text"),
    ];
    for (mode, name) in table {
        assert_eq!(mode.name(), name);
    }
}

#[test]
fn colour_labels() {
    let table = [
        (Cea608Color::White, "white"),
        (Cea608Color::Green, "green"),
        (Cea608Color::Blue, "blue"),
        (Cea608Color::Cyan, "cyan"),
        (Cea608Color::Red, "red"),
        (Cea608Color::Yellow, "yellow"),
        (Cea608Color::Magenta, "magenta"),
    ];
    for (color, name) in table {
        assert_eq!(color.name(), name);
    }
}
