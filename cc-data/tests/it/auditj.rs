#![cfg(feature = "decode")]
use broadcast_common::parity::set_odd_parity as par;
use cc_data::decode::{Cea608Channel, Cea608Decoder, Cea708Decoder};

fn pair(d: &mut Cea608Decoder, a: u8, b: u8) {
    d.push_pair(false, par(a), par(b));
}

#[test]
fn text_restart_must_not_erase_the_caption_screen() {
    let mut d = Cea608Decoder::new();
    pair(&mut d, 0x14, 0x20);
    pair(&mut d, 0x14, 0x70);
    pair(&mut d, b'H', b'I');
    pair(&mut d, 0x14, 0x2F);
    assert_eq!(d.channel_text(Cea608Channel::Cc1), "HI");
    pair(&mut d, 0x14, 0x2A);
    pair(&mut d, b'T', b'X');
    assert_eq!(d.channel_text(Cea608Channel::Cc1), "HI");
}

#[test]
fn second_carriage_return_after_a_null_pair_is_not_a_doubled_control() {
    let mut d = Cea608Decoder::new();
    pair(&mut d, 0x14, 0x25);
    pair(&mut d, b'A', b'B');
    pair(&mut d, 0x14, 0x2D);
    pair(&mut d, b'C', b'D');
    assert_eq!(d.channel_text(Cea608Channel::Cc1), "AB\nCD");
    pair(&mut d, 0x14, 0x2D);
    d.push_pair(false, 0x80, 0x80);
    pair(&mut d, 0x14, 0x2D);
    assert_eq!(d.channel_text(Cea608Channel::Cc1), "");
}

#[test]
fn spl_back_over_existing_text_overwrites_instead_of_appending() {
    let mut d = Cea708Decoder::new();
    d.push_packet(&[0x05, 0x27, 0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11, 0x00]);
    d.push_packet(&[0x45, 0x26, b'A', b'B', b'C', 0x92, 0x00, 0x00, 0x00, 0x00]);
    d.push_packet(&[0x82, 0x22, b'X', 0x00]);
    let w = d.windows(1)[2].as_ref().unwrap();
    assert_eq!(w.text(), "XBC");
}
