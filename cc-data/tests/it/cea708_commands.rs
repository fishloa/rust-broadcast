#![cfg(feature = "decode")]

//! CEA-708 command handlers — ANSI/CTA-708-E §8.10.5 (DefineWindow, SWA, SPA,
//! SPC, SPL, the window-map commands), §8.10.5.1/2 window + pen style presets
//! (Tables 26/27), §7.1.4 C0, §7.1.8/§7.1.9 G2/G3, and the §5/§6 packet and
//! service-block framing. Every case pins both the decoded struct fields and
//! the number of parameter bytes a handler consumes (the following text byte
//! must be printed at the right offset).

use cc_data::decode::{
    AnchorPoint, Cea708Decoder, Color, EdgeType, FontStyle, Justify, Opacity, PenOffset, PenSize,
    PrintDirection, ScrollDirection, WindowState,
};

fn ccp_seq(seq: u8, svc: u8, cmds: &[u8]) -> Vec<u8> {
    let mut block = vec![(svc << 5) | (cmds.len() as u8)];
    block.extend_from_slice(cmds);
    let size_code = (block.len().div_ceil(2) + 1) as u8 & 0x3F;
    let mut packet = vec![(seq << 6) | size_code];
    packet.extend_from_slice(&block);
    packet
}

fn ccp(svc: u8, cmds: &[u8]) -> Vec<u8> {
    ccp_seq(0, svc, cmds)
}

fn define_window(id: u8, p1: u8, p2: u8, p3: u8, p4: u8, p5: u8, p6: u8) -> Vec<u8> {
    vec![0x98 + id, p1, p2, p3, p4, p5, p6]
}

/// A visible window 0, 1 row x 11 columns, with the style/pen defaults.
fn base_window() -> Vec<u8> {
    define_window(0, 0x20, 0x00, 0x00, 0x00, 0x0A, 0x00)
}

fn text(dec: &Cea708Decoder, svc: usize, id: usize) -> String {
    dec.windows(svc)[id]
        .as_ref()
        .map(|w| w.text())
        .unwrap_or_default()
}

#[test]
fn anchor_point_table_and_labels() {
    let expected = [
        AnchorPoint::TopLeft,
        AnchorPoint::TopCenter,
        AnchorPoint::TopRight,
        AnchorPoint::MiddleLeft,
        AnchorPoint::MiddleCenter,
        AnchorPoint::MiddleRight,
        AnchorPoint::BottomLeft,
        AnchorPoint::BottomCenter,
        AnchorPoint::BottomRight,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
        AnchorPoint::TopLeft,
    ];
    let names = [
        "top_left",
        "top_center",
        "top_right",
        "middle_left",
        "middle_center",
        "middle_right",
        "bottom_left",
        "bottom_center",
        "bottom_right",
    ];
    for v in 0u8..=15 {
        let ap = AnchorPoint::from_bits(v);
        assert_eq!(ap, expected[v as usize], "v={v}");
        let name = if v <= 8 {
            names[v as usize]
        } else {
            "top_left"
        };
        assert_eq!(ap.name(), name, "v={v}");
    }
}

#[test]
fn window_state_labels() {
    assert_eq!(WindowState::Hidden.name(), "hidden");
    assert_eq!(WindowState::Visible.name(), "visible");
}

#[test]
fn define_window_decodes_every_bit_field() {
    let mut dec = Cea708Decoder::new();
    // id 3: priority 5, column lock, row lock, visible.
    let p1 = 0x20 | 0x10 | 0x08 | 0x05;
    let p2 = 0x80 | 0x45; // relative position, anchor vertical 69
    let p3 = 0xAB;
    let p4 = 0x83; // anchor point 8, rc 3 -> 4 rows
    let p5 = 0x2A; // cc 42 -> 43 columns
    let p6 = 0x35; // window style 6, pen style 5
    dec.push_packet(&ccp(1, &define_window(3, p1, p2, p3, p4, p5, p6)));
    let w = dec.windows(1)[3].as_ref().expect("window 3");
    assert_eq!(w.priority, 5);
    assert!(w.column_lock);
    assert!(w.row_lock);
    assert_eq!(w.state, WindowState::Visible);
    assert!(w.relative_position);
    assert_eq!(w.anchor_vertical, 69);
    assert_eq!(w.anchor_horizontal, 0xAB);
    assert_eq!(w.anchor_point, AnchorPoint::BottomRight);
    assert_eq!(w.row_count, 4);
    assert_eq!(w.column_count, 43);
    assert_eq!(w.window_style, 6);
    assert_eq!(w.pen_style, 5);

    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(
        1,
        &define_window(0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00),
    ));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.priority, 0);
    assert!(!w.column_lock);
    assert!(!w.row_lock);
    assert_eq!(w.state, WindowState::Hidden);
    assert!(!w.relative_position);
    assert_eq!(w.anchor_vertical, 0);
    assert_eq!(w.anchor_point, AnchorPoint::TopLeft);
    assert_eq!(w.row_count, 1);
    assert_eq!(w.column_count, 1);
    assert_eq!(w.window_style, 0);
    assert_eq!(w.pen_style, 0);
}

#[test]
fn define_window_style_presets_1_to_7() {
    // (style id, justify, word_wrap, print, scroll, fill opacity)
    let table = [
        (
            1u8,
            Justify::Left,
            false,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Solid,
        ),
        (
            2,
            Justify::Left,
            false,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Transparent,
        ),
        (
            3,
            Justify::Center,
            false,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Solid,
        ),
        (
            4,
            Justify::Left,
            true,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Solid,
        ),
        (
            5,
            Justify::Left,
            true,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Transparent,
        ),
        (
            6,
            Justify::Center,
            true,
            PrintDirection::LeftToRight,
            ScrollDirection::BottomToTop,
            Opacity::Solid,
        ),
        (
            7,
            Justify::Left,
            false,
            PrintDirection::TopToBottom,
            ScrollDirection::RightToLeft,
            Opacity::Solid,
        ),
    ];
    for (ws, justify, word_wrap, print, scroll, fill_op) in table {
        let mut dec = Cea708Decoder::new();
        dec.push_packet(&ccp(1, &define_window(0, 0x00, 0, 0, 0, 0x0A, ws << 3)));
        let w = dec.windows(1)[0].as_ref().expect("window 0");
        assert_eq!(w.window_style, ws);
        assert_eq!(w.justify, justify, "ws={ws}");
        assert_eq!(w.word_wrap, word_wrap, "ws={ws}");
        assert_eq!(w.print_direction, print, "ws={ws}");
        assert_eq!(w.scroll_direction, scroll, "ws={ws}");
        assert_eq!(w.fill_opacity, fill_op, "ws={ws}");
        assert_eq!(w.fill_color, Color::BLACK, "ws={ws}");
        assert_eq!(w.border_type, EdgeType::None, "ws={ws}");
    }
    // Style 0 is treated as preset 1.
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &define_window(0, 0x00, 0, 0, 0, 0x0A, 0x00)));
    assert_eq!(dec.windows(1)[0].as_ref().unwrap().justify, Justify::Left);
    assert_eq!(
        dec.windows(1)[0].as_ref().unwrap().fill_opacity,
        Opacity::Solid
    );
}

#[test]
fn define_window_pen_style_presets_1_to_7() {
    // (style id, font, edge, bg opacity)
    let table = [
        (1u8, FontStyle::Default, EdgeType::None, Opacity::Solid),
        (
            2,
            FontStyle::MonospacedSerif,
            EdgeType::None,
            Opacity::Solid,
        ),
        (
            3,
            FontStyle::ProportionalSerif,
            EdgeType::None,
            Opacity::Solid,
        ),
        (
            4,
            FontStyle::MonospacedSansSerif,
            EdgeType::None,
            Opacity::Solid,
        ),
        (
            5,
            FontStyle::ProportionalSansSerif,
            EdgeType::None,
            Opacity::Solid,
        ),
        (
            6,
            FontStyle::MonospacedSansSerif,
            EdgeType::Uniform,
            Opacity::Transparent,
        ),
        (
            7,
            FontStyle::ProportionalSansSerif,
            EdgeType::Uniform,
            Opacity::Transparent,
        ),
    ];
    for (ps, font, edge, bg_op) in table {
        let mut dec = Cea708Decoder::new();
        dec.push_packet(&ccp(1, &define_window(0, 0, 0, 0, 0, 0x0A, ps)));
        let w = dec.windows(1)[0].as_ref().expect("window 0");
        assert_eq!(w.pen_style, ps);
        assert_eq!(w.font_style, font, "ps={ps}");
        assert_eq!(w.edge_type, edge, "ps={ps}");
        assert_eq!(w.bg_opacity, bg_op, "ps={ps}");
        assert_eq!(w.bg_color, Color::BLACK, "ps={ps}");
        assert_eq!(w.pen_size, PenSize::Standard, "ps={ps}");
        assert_eq!(w.pen_offset, PenOffset::Normal, "ps={ps}");
        assert!(!w.italics, "ps={ps}");
        assert!(!w.underline, "ps={ps}");
        assert_eq!(w.fg_color, Color::WHITE, "ps={ps}");
        assert_eq!(w.fg_opacity, Opacity::Solid, "ps={ps}");
    }
}

#[test]
fn set_window_attributes_decodes_every_field() {
    let mut dec = Cea708Decoder::new();
    // SWA: fill opacity 2, fill colour (1,2,2); border colour (1,2,2) with
    // border type 5; word wrap 1, print 3, scroll 2, justify 1.
    let p1 = (2 << 6) | (1 << 4) | (2 << 2) | 2;
    let p2 = (1 << 6) | (1 << 4) | (2 << 2) | 2;
    let p3 = 0x80 | 0x40 | (3 << 4) | (2 << 2) | 1;
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x97, p1, p2, p3, 0x00]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.fill_opacity, Opacity::Translucent);
    assert_eq!(w.fill_color, Color::new(1, 2, 2));
    assert_eq!(w.border_color, Color::new(1, 2, 2));
    assert_eq!(w.border_type, EdgeType::RightDropShadow);
    assert!(w.word_wrap);
    assert_eq!(w.print_direction, PrintDirection::BottomToTop);
    assert_eq!(w.scroll_direction, ScrollDirection::TopToBottom);
    assert_eq!(w.justify, Justify::Right);
}

#[test]
fn set_pen_attributes_decodes_every_field_and_consumes_three_bytes() {
    let mut dec = Cea708Decoder::new();
    // SPA: offset 2, size 1; italics 1, underline 0, edge 5, font 3.
    let p1 = (2 << 2) | 1;
    let p2 = 0x80 | (5 << 3) | 3;
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x90, p1, p2, b'A']);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.pen_offset, PenOffset::Superscript);
    assert_eq!(w.pen_size, PenSize::Standard);
    assert!(w.italics);
    assert!(!w.underline);
    assert_eq!(w.edge_type, EdgeType::RightDropShadow);
    assert_eq!(w.font_style, FontStyle::MonospacedSansSerif);
    assert_eq!(w.text(), "A");

    // Underline set, italics clear.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x90, 0x00, 0x40, b'B']);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().unwrap();
    assert!(w.underline);
    assert!(!w.italics);
    assert_eq!(w.text(), "B");
}

#[test]
fn set_pen_color_decodes_every_channel_and_consumes_four_bytes() {
    let mut dec = Cea708Decoder::new();
    // SPC: fg opacity 1, fg (2,1,2); bg opacity 3, bg (1,2,0);
    // edge colour (1,2,2).
    let p1 = (1 << 6) | (2 << 4) | (1 << 2) | 2;
    let p2 = (1 << 6) | (1 << 4) | (2 << 2);
    let p3 = (1 << 4) | (2 << 2) | 2;
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x91, p1, p2, p3, b'C']);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.fg_opacity, Opacity::Flash);
    assert_eq!(w.fg_color, Color::new(2, 1, 2));
    assert_eq!(w.bg_opacity, Opacity::Flash);
    assert_eq!(w.bg_color, Color::new(1, 2, 0));
    assert_eq!(w.edge_color, Color::new(1, 2, 2));
    assert_eq!(w.text(), "C");
}

#[test]
fn set_pen_location_moves_the_pen_and_consumes_three_bytes() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00);
    cmds.extend_from_slice(&[0x92, 0x02, 0x03, b'X']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "   X");
}

#[test]
fn window_map_commands_display_hide_toggle_clear_delete() {
    // DSW shows both defined windows; service_text then exposes their text.
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x00, 0, 0, 0, 0x0A, 0x00);
    cmds.extend_from_slice(&define_window(1, 0x00, 0, 0, 0, 0x0A, 0x00));
    cmds.extend_from_slice(b"AB");
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(
        dec.windows(1)[0].as_ref().unwrap().state,
        WindowState::Hidden
    );
    assert_eq!(text(&dec, 1, 1), "AB");
    dec.push_packet(&ccp_seq(1, 1, &[0x89, 0x03]));
    assert_eq!(
        dec.windows(1)[0].as_ref().unwrap().state,
        WindowState::Visible
    );
    assert_eq!(
        dec.windows(1)[1].as_ref().unwrap().state,
        WindowState::Visible
    );
    assert_eq!(dec.service_text(1), "AB");

    // HDW hides window 0 only.
    dec.push_packet(&ccp_seq(2, 1, &[0x8A, 0x01]));
    assert_eq!(
        dec.windows(1)[0].as_ref().unwrap().state,
        WindowState::Hidden
    );
    assert_eq!(
        dec.windows(1)[1].as_ref().unwrap().state,
        WindowState::Visible
    );

    // TGW toggles: window 0 hidden -> visible, window 1 visible -> hidden.
    dec.push_packet(&ccp_seq(3, 1, &[0x8B, 0x03]));
    assert_eq!(
        dec.windows(1)[0].as_ref().unwrap().state,
        WindowState::Visible
    );
    assert_eq!(
        dec.windows(1)[1].as_ref().unwrap().state,
        WindowState::Hidden
    );

    // CLW clears window 0's text.
    dec.push_packet(&ccp_seq(0, 1, &[0x88, 0x01]));
    assert_eq!(text(&dec, 1, 0), "");
    assert_eq!(text(&dec, 1, 1), "AB");

    // DLW deletes window 1.
    dec.push_packet(&ccp_seq(1, 1, &[0x8C, 0x02]));
    assert!(dec.windows(1)[1].is_none());
    assert!(dec.windows(1)[0].is_some());
}

#[test]
fn delete_window_keeps_the_current_window_when_another_is_deleted() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00);
    cmds.extend_from_slice(&define_window(1, 0x20, 0, 0, 0, 0x0A, 0x00));
    cmds.push(b'A');
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 1), "A");
    // Deleting window 0 (not current) must leave window 1 current.
    dec.push_packet(&ccp_seq(1, 1, &[0x8C, 0x01]));
    dec.push_packet(&ccp_seq(2, 1, b"B"));
    assert_eq!(text(&dec, 1, 1), "AB");
}

#[test]
fn window_map_command_consumes_its_bitmap_byte() {
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    // DSW with bitmap 0x97, then text. 0x97 is SWA if the bitmap byte is not
    // consumed by DSW, which would swallow four text bytes.
    dec.push_packet(&ccp_seq(
        1,
        1,
        &[0x89, 0x97, b'A', b'B', b'C', b'D', b'E', b'F'],
    ));
    assert_eq!(text(&dec, 1, 0), "ABCDEF");
}

#[test]
fn delay_and_delay_cancel_consume_their_bytes() {
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    // DLY + one byte, then text: the delay byte must not be executed.
    dec.push_packet(&ccp_seq(1, 1, &[0x8D, 0x9B, b'A', b'B']));
    assert_eq!(text(&dec, 1, 0), "AB");

    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    dec.push_packet(&ccp_seq(1, 1, &[0x8E, b'C']));
    assert_eq!(text(&dec, 1, 0), "C");
}

#[test]
fn change_window_selects_an_existing_window() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00);
    cmds.extend_from_slice(&define_window(1, 0x20, 0, 0, 0, 0x0A, 0x00));
    cmds.extend_from_slice(&[0x80, b'A']); // CW0 then text
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "A");
    assert_eq!(text(&dec, 1, 1), "");
}

#[test]
fn reset_command_and_decoder_clear_all_windows() {
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    dec.push_packet(&ccp_seq(1, 1, b"A"));
    assert!(dec.windows(1)[0].is_some());
    dec.push_packet(&ccp(1, &[0x8F]));
    assert!(dec.windows(1)[0].is_none());

    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    dec.push_packet(&ccp_seq(1, 1, b"B"));
    assert!(dec.windows(1)[0].is_some());
    dec.reset();
    assert!(dec.windows(1)[0].is_none());
}

#[test]
fn c0_control_codes_consume_their_declared_length() {
    // NUL and ETX consume one byte and print nothing.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x00, 0x03, b'A']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "A");

    // P16 consumes the command plus its two parameter bytes.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x18, 0xFF, 0xFF, b'B']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "B");

    // Undefined 0x11..=0x17 consume two bytes.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x11, 0xFF, b'C']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "C");

    // Undefined 0x19..=0x1F consume three bytes.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x19, 0xFF, 0xFF, b'D']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "D");
}

#[test]
fn ext1_c2_block_lengths() {
    let cases: &[&[u8]] = &[
        &[0x10, 0x02, 0x00, b'A'],
        &[0x10, 0x08, 0x00, 0x00, b'B'],
        &[0x10, 0x14, 0x00, 0x00, 0x00, b'C'],
        &[0x10, 0x18, 0x00, 0x00, 0x00, 0x00, b'D'],
    ];
    for (i, cmds) in cases.iter().enumerate() {
        let mut dec = Cea708Decoder::new();
        let mut all = base_window();
        all.extend_from_slice(cmds);
        dec.push_packet(&ccp(1, &all));
        let expected = ['A', 'B', 'C', 'D'][i].to_string();
        assert_eq!(text(&dec, 1, 0), expected, "case {i}");
    }
}

#[test]
fn ext1_c3_block_lengths() {
    let cases: &[(&[u8], char)] = &[
        (&[0x10, 0x80, 0x00, 0x00, 0x00, 0x00, b'A'], 'A'),
        (&[0x10, 0x89, 0x00, 0x00, 0x00, 0x00, 0x00, b'B'], 'B'),
        // Variable length: n = (0x02 & 0x3F) + 1 = 3 -> command + 3 data bytes.
        (&[0x10, 0x90, 0x02, 0x00, 0x00, b'!', b'C'], 'C'),
    ];
    for (cmds, expected) in cases {
        let mut dec = Cea708Decoder::new();
        let mut all = base_window();
        all.extend_from_slice(cmds);
        dec.push_packet(&ccp(1, &all));
        assert_eq!(text(&dec, 1, 0), expected.to_string());
    }
}

#[test]
fn g2_characters_via_ext1() {
    let table = [
        (0x20u8, ' '),
        (0x21, ' '),
        (0x25, '\u{2026}'),
        (0x2A, '\u{0160}'),
        (0x2C, '\u{0152}'),
        (0x30, '\u{25A0}'),
        (0x31, '\u{2018}'),
        (0x32, '\u{2019}'),
        (0x33, '\u{201C}'),
        (0x34, '\u{201D}'),
        (0x35, '\u{2022}'),
        (0x39, '\u{2122}'),
        (0x3A, '\u{0161}'),
        (0x3C, '\u{0153}'),
        (0x3D, '\u{2120}'),
        (0x3F, '\u{0178}'),
        (0x76, '\u{215B}'),
        (0x77, '\u{215C}'),
        (0x78, '\u{215D}'),
        (0x79, '\u{215E}'),
        (0x24, '_'),
        (0x7F, '_'),
    ];
    for (b, expected) in table {
        let mut dec = Cea708Decoder::new();
        let mut cmds = base_window();
        cmds.extend_from_slice(&[0x10, b, b'A']);
        dec.push_packet(&ccp(1, &cmds));
        assert_eq!(text(&dec, 1, 0), format!("{expected}A"), "g2 {b:#04X}");
    }
}

#[test]
fn g3_characters_via_ext1() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x10, 0xA0, b'A']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "\u{1F4FA}A");

    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x10, 0xA1, b'B']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "_B");
}

#[test]
fn window_text_control_codes() {
    // Back space (0x08).
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[b'A', 0x08, b'B']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "B");

    // Back space at column 0 is a no-op.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x08, b'A']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "A");

    // Form feed (0x0C) clears the window.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[b'A', b'B', 0x0C, b'C']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "C");

    // Horizontal carriage return (0x0E) erases the current row.
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00);
    cmds[4] = 0x01; // rc = 1 -> two rows
    cmds.extend_from_slice(&[b'A', b'B', 0x0D, b'C', b'D', 0x0E, b'E']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "AB\nE");
}

#[test]
fn carriage_return_rolls_up_at_the_bottom_row() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00);
    cmds[4] = 0x01; // two rows
    cmds.extend_from_slice(&[b'A', 0x0D, b'B']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "A\nB");
}

#[test]
fn put_char_pads_to_the_pen_column_and_stops_at_the_column_width() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x92, 0x00, 0x03, b'X']);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "   X");

    // Two-column window: the third character is dropped.
    let mut dec = Cea708Decoder::new();
    let mut cmds = define_window(0, 0x20, 0, 0, 0, 0x01, 0x00); // cc=1 -> 2 cols
    cmds.extend_from_slice(b"ABC");
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "AB");
}

#[test]
fn consecutive_packet_sequences_do_not_reset_services() {
    let mut dec = Cea708Decoder::new();
    // seq 0 then seq 1 (header high bits); both carry service 1 and text.
    dec.push_packet(&[0x05, 0x28, 0x98, 0x20, 0, 0, 0, 0x0A, 0, b'A']);
    dec.push_packet(&[0x42, 0x21, b'B']);
    assert_eq!(text(&dec, 1, 0), "AB");
}

#[test]
fn a_non_consecutive_packet_sequence_resets_services() {
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&[0x05, 0x28, 0x98, 0x20, 0, 0, 0, 0x0A, 0, b'A']);
    // seq jumps from 0 to 2: services reset, so the text is lost.
    dec.push_packet(&[0x85, 0x21, b'B']);
    assert_eq!(text(&dec, 1, 0), "");
}

#[test]
fn packet_data_size_is_honoured() {
    // size_code 1 declares one data byte; the trailing bytes must not be
    // parsed as service blocks. seq 1 follows the window-defining seq 0.
    let mut dec = Cea708Decoder::new();
    dec.push_packet(&ccp(1, &base_window()));
    dec.push_packet(&[0x41, 0x21, b'X']);
    assert_eq!(text(&dec, 1, 0), "");
}

#[test]
fn extended_service_block_header_decodes_the_service_number() {
    let mut dec = Cea708Decoder::new();
    // Define window 0 on service 3 via the ordinary service-block header.
    dec.push_packet(&ccp(3, &base_window()));
    // Service escape header (svc field 7, size 1), 2nd byte service 3, block 'Z'.
    let block = [(7 << 5) | 1, 3, b'Z'];
    let size_code = (block.len().div_ceil(2) + 1) as u8 & 0x3F;
    let mut packet = vec![0x40 | size_code];
    packet.extend_from_slice(&block);
    dec.push_packet(&packet);
    assert_eq!(text(&dec, 3, 0), "Z");
}

#[test]
fn service_six_is_modelled() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.push(b'S');
    dec.push_packet(&ccp(6, &cmds));
    assert!(dec.windows(6)[0].is_some());
    assert_eq!(dec.service_text(6), "S");
}

#[test]
fn set_window_attributes_with_zero_flags() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    // word wrap 0, print 2, scroll 1, justify 2, border type 0.
    cmds.extend_from_slice(&[0x97, 0x00, 0x00, 0x26, 0x00]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert!(!w.word_wrap);
    assert_eq!(w.print_direction, PrintDirection::TopToBottom);
    assert_eq!(w.scroll_direction, ScrollDirection::RightToLeft);
    assert_eq!(w.justify, Justify::Center);
    assert_eq!(w.fill_opacity, Opacity::Solid);
    assert_eq!(w.fill_color, Color::BLACK);
    assert_eq!(w.border_color, Color::BLACK);
    assert_eq!(w.border_type, EdgeType::None);
}

#[test]
fn redefine_window_keeps_attributes_when_the_preset_is_zero() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    // SWA border type SHADOW_RIGHT; SPA italics + underline.
    cmds.extend_from_slice(&[0x97, 0x00, 0x40, 0x88, 0x00]);
    cmds.extend_from_slice(&[0x90, 0x03, 0xC3]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().unwrap();
    assert_eq!(w.border_type, EdgeType::RightDropShadow);
    assert!(w.italics);
    assert!(w.underline);

    // Redefine with window_style 0 and pen_style 0: both must leave the
    // attributes set by SWA/SPA untouched.
    dec.push_packet(&ccp_seq(1, 1, &define_window(0, 0x20, 0, 0, 0, 0x0A, 0x00)));
    let w = dec.windows(1)[0].as_ref().unwrap();
    assert_eq!(w.border_type, EdgeType::RightDropShadow);
    assert!(w.italics);
    assert!(w.underline);
}

#[test]
fn pen_commands_at_the_end_of_the_block_still_decode() {
    // Exactly three bytes: the command plus its parameters, with no text
    // after it. A `<`-vs-`<=` style guard must not skip the last command.
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x90, 0x09, 0xAB]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.pen_offset, PenOffset::Superscript);
    assert_eq!(w.pen_size, PenSize::Standard);
    assert!(w.italics);
    assert_eq!(w.edge_type, EdgeType::RightDropShadow);
    assert_eq!(w.font_style, FontStyle::MonospacedSansSerif);

    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[0x91, 0x66, 0x58, 0x1A]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.fg_opacity, Opacity::Flash);
    assert_eq!(w.fg_color, Color::new(2, 1, 2));
    assert_eq!(w.bg_opacity, Opacity::Flash);
    assert_eq!(w.bg_color, Color::new(1, 2, 0));
    assert_eq!(w.edge_color, Color::new(1, 2, 2));
}

#[test]
fn back_space_truncates_the_row_without_a_following_character() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[b'A', b'B', 0x08]);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "A");

    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    cmds.extend_from_slice(&[b'A', 0x08]);
    dec.push_packet(&ccp(1, &cmds));
    assert_eq!(text(&dec, 1, 0), "");
}

#[test]
fn pen_size_small_from_zero_bits() {
    let mut dec = Cea708Decoder::new();
    let mut cmds = base_window();
    // SPA: offset 2 (bits [3:2]), size 0 (bits [1:0]).
    cmds.extend_from_slice(&[0x90, 0x08, 0x00]);
    dec.push_packet(&ccp(1, &cmds));
    let w = dec.windows(1)[0].as_ref().expect("window 0");
    assert_eq!(w.pen_offset, PenOffset::Superscript);
    assert_eq!(w.pen_size, PenSize::Small);
}
