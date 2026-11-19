//! Pixel-data run-length decoding — ETSI EN 300 743 §7.2.5.2, Tables 22–26
//! (`private/specs-verified/dvb/dvb-subtitle/tables/en_300_743/`), plus the
//! sub-block dispatch (Tables 20/21) and the Object Data Segment payload
//! offsets (§7.2.5, Table 17).

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::{
    DataType, ObjectCodingMethod, ObjectDataPayload, ObjectDataSegment, PixelDataSubBlock, PixelRun,
};

struct Bits {
    bits: Vec<u8>,
}

impl Bits {
    fn new() -> Self {
        Bits { bits: Vec::new() }
    }

    fn value(&mut self, value: u32, n: usize) -> &mut Self {
        for i in (0..n).rev() {
            self.bits.push(((value >> i) & 1) as u8);
        }
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.bits.len().div_ceil(8)];
        for (i, bit) in self.bits.iter().enumerate() {
            if *bit != 0 {
                out[i / 8] |= 0x80 >> (i % 8);
            }
        }
        out
    }
}

fn runs(data_type: DataType, data: &[u8]) -> Vec<PixelRun> {
    let block = PixelDataSubBlock { data_type, data };
    block
        .runs()
        .expect("code string sub-block")
        .collect::<Vec<_>>()
}

fn run(pixel_code: u8, run_length: u32) -> PixelRun {
    PixelRun {
        pixel_code,
        run_length,
    }
}

fn segment(flags: u8, payload: &[u8]) -> Vec<u8> {
    let body_len = 3 + payload.len();
    let mut out = vec![0x0F, 0x13, 0x00, 0x01];
    out.extend_from_slice(&(body_len as u16).to_be_bytes());
    out.extend_from_slice(&[0x00, 0x0A, flags]);
    out.extend_from_slice(payload);
    out
}

fn interlaced(top: &[u8], bottom: &[u8], stuffing: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(top.len() as u16).to_be_bytes());
    payload.extend_from_slice(&(bottom.len() as u16).to_be_bytes());
    payload.extend_from_slice(top);
    payload.extend_from_slice(bottom);
    payload.extend_from_slice(stuffing);
    segment(0x00, &payload)
}

#[test]
fn two_bit_pixel_codes_and_runs() {
    // 2-bitpixel-code (Table 22): a non-zero 2-bit value is one pixel.
    for code in 1u8..=3 {
        let mut b = Bits::new();
        b.value(u32::from(code), 2).value(0, 6); // pixel, then end_of_string
        assert_eq!(
            runs(DataType::CodeString2Bit, &b.bytes()),
            vec![run(code, 1)]
        );
    }

    // switch_1 = 1: run_length_3-10 + 2-bitpixel-code.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(1, 1)
        .value(3, 3)
        .value(1, 2)
        .value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(1, 6)]);

    let mut b = Bits::new();
    b.value(0, 2)
        .value(1, 1)
        .value(7, 3)
        .value(3, 2)
        .value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(3, 10)]);

    // switch_2 = 1: one pixel in colour 0.
    let mut b = Bits::new();
    b.value(0, 2).value(0, 1).value(1, 1).value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(0, 1)]);

    // switch_3 = 01: two pixels in colour 0.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b01, 2)
        .value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(0, 2)]);

    // switch_3 = 10: run_length_12-27 + 2-bitpixel-code.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b10, 2)
        .value(15, 4)
        .value(2, 2)
        .value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(2, 27)]);

    // switch_3 = 11: run_length_29-284 + 2-bitpixel-code.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b11, 2)
        .value(255, 8)
        .value(3, 2)
        .value(0, 6);
    assert_eq!(
        runs(DataType::CodeString2Bit, &b.bytes()),
        vec![run(3, 284)]
    );

    let mut b = Bits::new();
    b.value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b11, 2)
        .value(0, 8)
        .value(1, 2)
        .value(0, 6);
    assert_eq!(runs(DataType::CodeString2Bit, &b.bytes()), vec![run(1, 29)]);
}

#[test]
fn four_bit_pixel_codes_and_runs() {
    for code in [1u8, 5, 15] {
        let mut b = Bits::new();
        b.value(u32::from(code), 4).value(0, 8); // pixel, end_of_string
        assert_eq!(
            runs(DataType::CodeString4Bit, &b.bytes()),
            vec![run(code, 1)]
        );
    }

    // switch_1 = 0: run_length_3-9 in colour 0 (value + 2).
    let mut b = Bits::new();
    b.value(0, 4).value(0, 1).value(3, 3).value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(0, 5)]);

    let mut b = Bits::new();
    b.value(0, 4).value(0, 1).value(7, 3).value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(0, 9)]);

    // switch_2 = 0: run_length_4-7 + 4-bit_pixel-code.
    for (n2, expected) in [(0u32, 4u32), (3, 7)] {
        let mut b = Bits::new();
        b.value(0, 4)
            .value(1, 1)
            .value(0, 1)
            .value(n2, 2)
            .value(9, 4)
            .value(0, 8);
        assert_eq!(
            runs(DataType::CodeString4Bit, &b.bytes()),
            vec![run(9, expected)]
        );
    }

    // switch_3 = 00 / 01.
    let mut b = Bits::new();
    b.value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b00, 2)
        .value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(0, 1)]);

    let mut b = Bits::new();
    b.value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b01, 2)
        .value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(0, 2)]);

    // switch_3 = 10: run_length_9-24 + 4-bit_pixel-code.
    let mut b = Bits::new();
    b.value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b10, 2)
        .value(15, 4)
        .value(7, 4)
        .value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(7, 24)]);

    // switch_3 = 11: run_length_25-280 + 4-bit_pixel-code.
    let mut b = Bits::new();
    b.value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b11, 2)
        .value(255, 8)
        .value(12, 4)
        .value(0, 8);
    assert_eq!(
        runs(DataType::CodeString4Bit, &b.bytes()),
        vec![run(12, 280)]
    );

    let mut b = Bits::new();
    b.value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b11, 2)
        .value(0, 8)
        .value(1, 4)
        .value(0, 8);
    assert_eq!(runs(DataType::CodeString4Bit, &b.bytes()), vec![run(1, 25)]);
}

#[test]
fn eight_bit_pixel_codes_and_runs() {
    assert_eq!(
        runs(DataType::CodeString8Bit, &[0x05, 0x00, 0x00]),
        vec![run(5, 1)]
    );
    assert_eq!(
        runs(DataType::CodeString8Bit, &[0x00, 0x01, 0x00, 0x00]),
        vec![run(0, 1)]
    );
    assert_eq!(
        runs(DataType::CodeString8Bit, &[0x00, 0x7F, 0x00, 0x00]),
        vec![run(0, 127)]
    );
    assert_eq!(
        runs(DataType::CodeString8Bit, &[0x00, 0x83, 0x09, 0x00, 0x00]),
        vec![run(9, 3)]
    );
    assert_eq!(
        runs(DataType::CodeString8Bit, &[0x00, 0xFF, 0x7A, 0x00, 0x00]),
        vec![run(0x7A, 127)]
    );
    // Truncated tails stop cleanly.
    assert!(runs(DataType::CodeString8Bit, &[0x00]).is_empty());
    assert!(runs(DataType::CodeString8Bit, &[0x00, 0x80]).is_empty());
    assert!(runs(DataType::CodeString8Bit, &[]).is_empty());
}

#[test]
fn code_string_scanners_consume_the_right_byte_count() {
    // 2-bit: one pixel (2 bits) + end (6 bits) = 8 bits -> 1 byte.
    let mut b = Bits::new();
    b.value(0b01, 2).value(0, 6);
    let bytes = b.bytes();
    let mut top = vec![0x10u8];
    top.extend_from_slice(&bytes);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks[0].data, &[0x40u8]);

    // 2-bit: run 3-10 token (8 bits) + end (6 bits) = 14 bits -> 2 bytes.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(1, 1)
        .value(3, 3)
        .value(1, 2)
        .value(0, 6);
    let bytes = b.bytes();
    assert_eq!(bytes.len(), 2);
    let mut top = vec![0x10u8];
    top.extend_from_slice(&bytes);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks[0].data.len(), 2);

    // 4-bit: one pixel (4 bits) + end (8 bits) = 12 bits -> 2 bytes.
    let mut b = Bits::new();
    b.value(1, 4).value(0, 8);
    let bytes = b.bytes();
    assert_eq!(bytes.len(), 2);
    let mut top = vec![0x11u8];
    top.extend_from_slice(&bytes);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks[0].data.len(), 2);

    // 4-bit: run 3-9 (8 bits) + end (8 bits) = 16 bits -> 2 bytes.
    let mut b = Bits::new();
    b.value(0, 4).value(0, 1).value(2, 3).value(0, 8);
    let bytes = b.bytes();
    let mut top = vec![0x11u8];
    top.extend_from_slice(&bytes);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks[0].data.len(), 2);

    // 8-bit: one pixel + end = 24 bits -> 3 bytes.
    let bytes = [0x05u8, 0x00, 0x00];
    let mut top = vec![0x12u8];
    top.extend_from_slice(&bytes);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks[0].data.len(), 3);
}

#[test]
fn pixel_sub_block_dispatch_and_map_table_sizes() {
    let mut top = vec![
        0x10u8, 0x00, // 2-bit: end + stuffing
        0x11, 0x00, // 4-bit: end
        0x12, 0x00, 0x00, // 8-bit: end
        0x20, 0xAA, 0xBB, // 2-to-4 map table (2 bytes)
    ];
    top.extend_from_slice(&[0x21, 0x01, 0x02, 0x03, 0x04]); // 2-to-8 (4 bytes)
    let mut table_4to8 = vec![0x22u8];
    table_4to8.extend(0u8..16);
    top.extend_from_slice(&table_4to8);
    top.push(0xF0); // end of line
    top.extend_from_slice(&[0x30, 0x01, 0x02]); // reserved: consumes the rest

    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    let blocks = &ip.top_sub_blocks;
    assert_eq!(blocks.len(), 8);
    assert_eq!(blocks[0].data_type, DataType::CodeString2Bit);
    assert_eq!(blocks[1].data_type, DataType::CodeString4Bit);
    assert_eq!(blocks[2].data_type, DataType::CodeString8Bit);
    assert_eq!(blocks[3].data_type, DataType::MapTable2To4);
    assert_eq!(blocks[3].data, &[0xAA, 0xBB]);
    assert_eq!(blocks[4].data_type, DataType::MapTable2To8);
    assert_eq!(blocks[4].data, &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(blocks[5].data_type, DataType::MapTable4To8);
    assert_eq!(blocks[5].data.len(), 16);
    assert_eq!(blocks[6].data_type, DataType::EndOfLine);
    assert!(blocks[6].data.is_empty());
    assert_eq!(blocks[7].data_type, DataType::Reserved(0x30));
    assert_eq!(blocks[7].data, &[0x01, 0x02]);

    let out = seg.try_to_bytes().unwrap();
    assert_eq!(out, interlaced(&top, &[], &[]));
}

#[test]
fn top_and_bottom_field_offsets_are_independent() {
    let mut top = vec![0xF0u8];
    top.extend(std::iter::repeat_n(0x00u8, 299));
    let mut bottom = vec![0xF0u8];
    bottom.extend(std::iter::repeat_n(0x00u8, 69));
    let bytes = interlaced(&top, &bottom, &[]);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.top_sub_blocks.len(), 2);
    assert_eq!(ip.top_sub_blocks[0].data_type, DataType::EndOfLine);
    assert_eq!(ip.top_sub_blocks[1].data.len(), top.len() - 2);
    assert_eq!(ip.bottom_sub_blocks[1].data.len(), bottom.len() - 2);
    assert_eq!(ip.top_sub_blocks[1].data, &top[2..]);
    assert_eq!(ip.bottom_sub_blocks.len(), 2);
    assert_eq!(ip.bottom_sub_blocks[1].data, &bottom[2..]);
    assert!(ip.stuffing.is_empty());
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn stuffing_byte_is_preserved_and_validated() {
    let top = [0xF0u8];
    let bytes = interlaced(&top, &top, &[0x00]);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    assert_eq!(ip.stuffing, &[0x00]);
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);

    let bad = interlaced(&top, &top, &[0x01]);
    assert!(ObjectDataSegment::parse(&bad).is_err());
}

#[test]
fn character_payload_parses_and_serializes() {
    // version 0xB, coding method 1 (characters), non-modifying colour flag.
    let flags = 0xB0 | (1 << 2) | 0x02;
    let payload = [0x02u8, 0x00, 0x41, 0x20, 0xAC];
    let bytes = segment(flags, &payload);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    assert_eq!(seg.object_id, 10);
    assert_eq!(seg.object_version_number, 0xB);
    assert_eq!(seg.object_coding_method, ObjectCodingMethod::Characters);
    assert!(seg.non_modifying_colour_flag);
    let ObjectDataPayload::Characters { character_codes } = &seg.payload else {
        panic!("expected characters");
    };
    assert_eq!(character_codes, &[0x0041, 0x20AC]);
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);

    // One character code needs 3 payload bytes.
    let one = segment(0x04, &[0x01, 0x00, 0x42]);
    let seg = ObjectDataSegment::parse(&one).unwrap();
    let ObjectDataPayload::Characters { character_codes } = &seg.payload else {
        panic!("expected characters");
    };
    assert_eq!(character_codes, &[0x0042]);

    // Truncated code arrays are rejected.
    assert!(ObjectDataSegment::parse(&segment(0x04, &[0x01, 0x00])).is_err());
    assert!(ObjectDataSegment::parse(&segment(0x04, &[0x03, 0, 0, 0, 0, 0])).is_err());
    // An empty payload cannot hold the count byte.
    assert!(ObjectDataSegment::parse(&segment(0x04, &[])).is_err());
}

#[test]
fn progressive_payload_parses_and_serializes() {
    let flags = 0x08; // coding method 2
    let mut payload = vec![0x12u8, 0x34, 0x05, 0x67, 0x00, 0x03];
    payload.extend_from_slice(&[0xDE, 0xAD, 0xBE]);
    let bytes = segment(flags, &payload);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    assert_eq!(
        seg.object_coding_method,
        ObjectCodingMethod::ProgressivePixels
    );
    let ObjectDataPayload::ProgressivePixels(pp) = &seg.payload else {
        panic!("expected progressive pixels");
    };
    assert_eq!(pp.bitmap_width, 0x1234);
    assert_eq!(pp.bitmap_height, 0x0567);
    assert_eq!(pp.compressed_data, &[0xDE, 0xAD, 0xBE]);
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);

    // Declared compressed length beyond the payload is rejected.
    let truncated = segment(flags, &[0x00, 0x01, 0x00, 0x01, 0x00, 0x03]);
    assert!(ObjectDataSegment::parse(&truncated).is_err());
    // Short of the 6-byte progressive header.
    assert!(ObjectDataSegment::parse(&segment(flags, &[0x00, 0x01])).is_err());
}

#[test]
fn reserved_coding_method_preserves_raw_payload() {
    let flags = 0x0C; // coding method 3 (reserved)
    let payload = [0x11u8, 0x22, 0x33];
    let bytes = segment(flags, &payload);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    assert_eq!(seg.object_coding_method, ObjectCodingMethod::Reserved(3));
    let ObjectDataPayload::Reserved {
        coding_method,
        data,
    } = &seg.payload
    else {
        panic!("expected reserved payload");
    };
    assert_eq!(*coding_method, 3);
    assert_eq!(*data, &payload[..]);
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn pixel_coding_method_and_object_fields() {
    let bytes = segment(0x00, &[0x00, 0x00, 0x00, 0x00]);
    let seg = ObjectDataSegment::parse(&bytes).unwrap();
    assert_eq!(seg.page_id, 1);
    assert_eq!(seg.object_id, 10);
    assert_eq!(seg.object_version_number, 0);
    assert_eq!(seg.object_coding_method, ObjectCodingMethod::Pixels);
    assert!(!seg.non_modifying_colour_flag);

    // The 3-byte fixed header is required.
    assert!(ObjectDataSegment::parse(&[0x0F, 0x13, 0x00, 0x01, 0x00, 0x02, 0x00, 0x0A]).is_err());
    // A short outer buffer must be rejected without panicking.
    assert!(ObjectDataSegment::parse(&[0x0F, 0x13, 0x00]).is_err());
    // Declared segment length beyond the buffer is rejected.
    let mut short = segment(0x00, &[0x00, 0x00, 0x00, 0x00]);
    let len = u16::from_be_bytes([short[4], short[5]]) + 1;
    short[4..6].copy_from_slice(&len.to_be_bytes());
    assert!(ObjectDataSegment::parse(&short).is_err());
}
