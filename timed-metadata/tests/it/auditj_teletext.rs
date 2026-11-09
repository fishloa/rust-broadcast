#![cfg(feature = "teletext")]
use broadcast_common::parity::set_odd_parity as parity;
use dvb_vbi::{FRAMING_CODE_EBU, LineHeader, TeletextDataField};
use timed_metadata::webvtt::TeletextCueExtractor;

fn hamming(n: u8) -> u8 {
    let (d1, d2, d3, d4) = (n & 1, (n >> 1) & 1, (n >> 2) & 1, (n >> 3) & 1);
    let p1 = 1 ^ d1 ^ d3 ^ d4;
    let p2 = 1 ^ d1 ^ d2 ^ d4;
    let p3 = 1 ^ d1 ^ d2 ^ d3;
    let p4 = 1 ^ p1 ^ d1 ^ p2 ^ d2 ^ p3 ^ d3 ^ d4;
    p1 | (d1 << 1) | (p2 << 2) | (d2 << 3) | (p3 << 4) | (d3 << 5) | (p4 << 6) | (d4 << 7)
}

fn field(block: [u8; 42]) -> TeletextDataField {
    TeletextDataField {
        header: LineHeader::new(true, 0),
        framing_code: FRAMING_CODE_EBU,
        txt_data_block: block.map(u8::reverse_bits),
    }
}

fn header() -> [u8; 42] {
    let mut h = [0u8; 42];
    h[0] = hamming(0);
    h[1] = hamming(0);
    h[2] = hamming(8);
    h[3] = hamming(8);
    h[5] = hamming(0b1000);
    h[7] = hamming(0b1000);
    for b in h.iter_mut().skip(10) {
        *b = parity(0x20);
    }
    h
}

fn row1(codes: &[u8]) -> [u8; 42] {
    let mut r = [0u8; 42];
    r[0] = hamming(1 << 3);
    r[1] = hamming(0);
    for b in r.iter_mut().skip(2) {
        *b = parity(0x20);
    }
    for (i, &c) in codes.iter().enumerate() {
        r[2 + i] = parity(c);
    }
    r
}

#[test]
fn two_boxes_on_one_row_are_not_glued_together() {
    let mut ex = TeletextCueExtractor::new(8, 0x88);
    ex.push_frame(0, &[field(header())]);
    let codes = [
        0x0B, 0x0B, b'H', b'i', 0x0A, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x0B, 0x0B, b'Y', b'o', 0x0A,
        0x0A,
    ];
    ex.push_frame(1, &[field(row1(&codes))]);
    ex.finalize(2);
    let cues = ex.cues();
    assert_eq!(cues.len(), 1);
    assert_ne!(cues[0].text, "HiYo");
    assert!(cues[0].text.contains("Hi") && cues[0].text.contains("Yo"));
}
