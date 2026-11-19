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
        .take(16)
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
    let mut b = Bits::new();
    b.value(3, 2) // one pixel of code 3
        .value(1, 2) // one pixel of code 1
        .value(0, 2)
        .value(1, 1)
        .value(0b101, 3)
        .value(1, 2) // run 8 of code 1
        .value(0, 2)
        .value(0, 1)
        .value(1, 1) // one pixel of code 0
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b01, 2) // two pixels of code 0
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b10, 2)
        .value(0b1001, 4)
        .value(2, 2) // run 21 of code 2
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b11, 2)
        .value(0b0101_1010, 8)
        .value(3, 2) // run 119 of code 3
        .value(1, 2) // one pixel of code 1
        .value(0, 2)
        .value(0, 1)
        .value(1, 1) // one pixel of code 0
        .value(0, 6); // end_of_string
    assert_eq!(
        runs(DataType::CodeString2Bit, &b.bytes()),
        vec![
            run(3, 1),
            run(1, 1),
            run(1, 8),
            run(0, 1),
            run(0, 2),
            run(2, 21),
            run(3, 119),
            run(1, 1),
            run(0, 1),
        ]
    );
}

#[test]
fn four_bit_pixel_codes_and_runs() {
    let mut b = Bits::new();
    b.value(5, 4) // one pixel of code 5
        .value(0, 4)
        .value(0, 1)
        .value(0b101, 3) // run 7 of code 0
        .value(0, 4)
        .value(1, 1)
        .value(0, 1)
        .value(0b10, 2)
        .value(0b1010, 4) // run 6 of code 10
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b00, 2) // one pixel of code 0
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b01, 2) // two pixels of code 0
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b10, 2)
        .value(0b0110, 4)
        .value(0b1100, 4) // run 15 of code 12
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b11, 2)
        .value(0b0101_1010, 8)
        .value(0b0011, 4) // run 115 of code 3
        .value(9, 4) // one pixel of code 9
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b01, 2) // two pixels of code 0
        .value(0, 8); // end_of_string
    assert_eq!(
        runs(DataType::CodeString4Bit, &b.bytes()),
        vec![
            run(5, 1),
            run(0, 7),
            run(10, 6),
            run(0, 1),
            run(0, 2),
            run(12, 15),
            run(3, 115),
            run(9, 1),
            run(0, 2),
        ]
    );
}

#[test]
fn eight_bit_pixel_codes_and_runs() {
    let data = [
        0x5A, // one pixel of code 0x5A
        0x00, 0x05, // run 5 of code 0
        0x00, 0x85, 0x33, // run 5 of code 0x33
        0x11, // one pixel of code 0x11
        0x00, 0x00, // end_of_string
    ];
    assert_eq!(
        runs(DataType::CodeString8Bit, &data),
        vec![run(0x5A, 1), run(0, 5), run(0x33, 5), run(0x11, 1)]
    );
    // Truncated tails stop cleanly.
    assert!(runs(DataType::CodeString8Bit, &[0x00]).is_empty());
    assert!(runs(DataType::CodeString8Bit, &[0x00, 0x80]).is_empty());
    assert!(runs(DataType::CodeString8Bit, &[]).is_empty());
}

fn assert_scanned_len(data_type: u8, code_bytes: &[u8], expected: usize) {
    let mut top = vec![data_type];
    top.extend_from_slice(code_bytes);
    // A map-table sub-block must start exactly where the code string ends.
    top.extend_from_slice(&[0x20, 0xAA, 0xBB]);
    let wire = interlaced(&top, &[], &[]);
    let seg = ObjectDataSegment::parse(&wire).unwrap();
    let ObjectDataPayload::InterlacedPixels(ip) = &seg.payload else {
        panic!("expected interlaced payload");
    };
    eprintln!(
        "actual {} expected {}",
        ip.top_sub_blocks[0].data.len(),
        expected
    );
    assert_eq!(ip.top_sub_blocks[0].data.len(), expected);
    assert_eq!(ip.top_sub_blocks[1].data_type, DataType::MapTable2To4);
    assert_eq!(ip.top_sub_blocks[1].data, &[0xAA, 0xBB]);
}

#[test]
fn code_string_scanners_consume_the_right_byte_count() {
    // 2-bit: six tokens of mixed lengths + end = 54 bits -> 7 bytes.
    let mut b = Bits::new();
    b.value(0b10, 2)
        .value(0, 2)
        .value(1, 1)
        .value(0b101, 3)
        .value(1, 2)
        .value(0, 2)
        .value(0, 1)
        .value(1, 1)
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b01, 2)
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b10, 2)
        .value(0b1001, 4)
        .value(2, 2)
        .value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b11, 2)
        .value(0b0101_1010, 8)
        .value(3, 2)
        .value(0, 6);
    assert_scanned_len(0x10, &b.bytes(), 7);

    // 4-bit: seven tokens + end = 84 bits -> 11 bytes.
    let mut b = Bits::new();
    b.value(5, 4)
        .value(0, 4)
        .value(0, 1)
        .value(0b101, 3)
        .value(0, 4)
        .value(1, 1)
        .value(0, 1)
        .value(0b10, 2)
        .value(0b1010, 4)
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b00, 2)
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b01, 2)
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b10, 2)
        .value(0b0110, 4)
        .value(0b1100, 4)
        .value(0, 4)
        .value(1, 1)
        .value(1, 1)
        .value(0b11, 2)
        .value(0b0101_1010, 8)
        .value(0b0011, 4)
        .value(0, 8);
    assert_scanned_len(0x11, &b.bytes(), 11);

    // 8-bit: pixel + two run codes + end = 8 bytes.
    let data = [0x5A, 0x00, 0x05, 0x00, 0x85, 0x33, 0x00, 0x00];
    assert_scanned_len(0x12, &data, 8);
}

#[test]
fn sub_block_type_names() {
    assert_eq!(DataType::CodeString2Bit.name(), "2-bit_code_string");
    assert_eq!(DataType::CodeString4Bit.name(), "4-bit_code_string");
    assert_eq!(DataType::CodeString8Bit.name(), "8-bit_code_string");
    assert_eq!(DataType::MapTable2To4.name(), "2_to_4_map_table");
    assert_eq!(DataType::MapTable2To8.name(), "2_to_8_map_table");
    assert_eq!(DataType::MapTable4To8.name(), "4_to_8_map_table");
    assert_eq!(DataType::EndOfLine.name(), "end_of_line");
    assert_eq!(DataType::Reserved(0x30).name(), "reserved");
    assert_eq!(ObjectCodingMethod::Pixels.name(), "pixels");
    assert_eq!(ObjectCodingMethod::Characters.name(), "characters");
    assert_eq!(
        ObjectCodingMethod::ProgressivePixels.name(),
        "progressive_pixels"
    );
    assert_eq!(ObjectCodingMethod::Reserved(3).name(), "reserved");
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

#[test]
fn truncated_cross_byte_reads_stop_cleanly() {
    // The 8-bit run-length field starts at bit 6 and its second byte is
    // missing: the zero-padded read must not slice past the data.
    assert_eq!(
        runs(DataType::CodeString2Bit, &[0b0000_1101]),
        vec![run(0, 93)]
    );
    // 4-bit switch_3 = 10 starts at bit 6 with no second byte.
    assert_eq!(
        runs(DataType::CodeString4Bit, &[0b0000_1110]),
        vec![run(0, 9)]
    );
}

fn token_2bit(b: &mut Bits, variant: usize) {
    match variant {
        0 => {
            b.value(1, 2);
        }
        1 => {
            b.value(2, 2);
        }
        2 => {
            b.value(0, 2).value(1, 1).value(0b011, 3).value(2, 2);
        }
        3 => {
            b.value(0, 2).value(0, 1).value(1, 1);
        }
        4 => {
            b.value(0, 2).value(0, 1).value(0, 1).value(0b01, 2);
        }
        _ => {
            b.value(0, 2)
                .value(0, 1)
                .value(0, 1)
                .value(0b10, 2)
                .value(0b1001, 4)
                .value(1, 2);
        }
    }
}

fn token_4bit(b: &mut Bits, variant: usize) {
    match variant {
        0 => {
            b.value(5, 4);
        }
        1 => {
            b.value(0, 4).value(0, 1).value(0b101, 3);
        }
        2 => {
            b.value(0, 4)
                .value(1, 1)
                .value(0, 1)
                .value(0b01, 2)
                .value(0b1010, 4);
        }
        3 => {
            b.value(0, 4).value(1, 1).value(1, 1).value(0b00, 2);
        }
        4 => {
            b.value(0, 4).value(1, 1).value(1, 1).value(0b01, 2);
        }
        5 => {
            b.value(0, 4)
                .value(1, 1)
                .value(1, 1)
                .value(0b10, 2)
                .value(0b0110, 4)
                .value(0b1100, 4);
        }
        _ => {
            b.value(0, 4)
                .value(1, 1)
                .value(1, 1)
                .value(0b11, 2)
                .value(0b0101_1010, 8)
                .value(0b0011, 4);
        }
    }
}

#[test]
fn scanner_byte_length_sweep() {
    for first in 0..6 {
        for second in 0..6 {
            let mut b = Bits::new();
            token_2bit(&mut b, first);
            token_2bit(&mut b, second);
            b.value(0, 6);
            let expected = b.bits.len().div_ceil(8);
            assert_scanned_len(0x10, &b.bytes(), expected);
        }
    }
    for first in 0..7 {
        for second in 0..7 {
            let mut b = Bits::new();
            token_4bit(&mut b, first);
            token_4bit(&mut b, second);
            b.value(0, 8);
            let expected = b.bits.len().div_ceil(8);
            assert_scanned_len(0x11, &b.bytes(), expected);
        }
    }
    let cells: [&[u8]; 5] = [
        &[0x01],
        &[0x5A],
        &[0x00, 0x01],
        &[0x00, 0x83, 0x11],
        &[0x00, 0xFF, 0x5A],
    ];
    for first in cells {
        for second in cells {
            let mut data = Vec::new();
            data.extend_from_slice(first);
            data.extend_from_slice(second);
            data.extend_from_slice(&[0x00, 0x00]);
            let expected = data.len();
            assert_scanned_len(0x12, &data, expected);
        }
    }
}

fn next_rand(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

#[test]
fn scanner_random_stream_sweep() {
    let mut state = 0x1234_5678u32;
    for _ in 0..2000 {
        let count = 1 + (next_rand(&mut state) as usize % 5);
        let mut b = Bits::new();
        for _ in 0..count {
            token_2bit(&mut b, next_rand(&mut state) as usize % 6);
        }
        b.value(0, 6);
        let expected = b.bits.len().div_ceil(8);
        assert_scanned_len(0x10, &b.bytes(), expected);

        let count = 1 + (next_rand(&mut state) as usize % 5);
        let mut b = Bits::new();
        for _ in 0..count {
            token_4bit(&mut b, next_rand(&mut state) as usize % 7);
        }
        b.value(0, 8);
        let expected = b.bits.len().div_ceil(8);
        assert_scanned_len(0x11, &b.bytes(), expected);

        let count = 1 + (next_rand(&mut state) as usize % 5);
        let mut data = Vec::new();
        for _ in 0..count {
            match next_rand(&mut state) % 3 {
                0 => data.push((next_rand(&mut state) as u8).max(1)),
                1 => {
                    data.push(0x00);
                    data.push((next_rand(&mut state) as u8 & 0x7F).max(1));
                }
                _ => {
                    data.push(0x00);
                    data.push(0x80 | ((next_rand(&mut state) as u8 & 0x7F).max(3)));
                    data.push(next_rand(&mut state) as u8);
                }
            }
        }
        data.extend_from_slice(&[0x00, 0x00]);
        assert_scanned_len(0x12, &data, data.len());
    }
}

#[test]
fn cross_byte_iterator_reads_pin_both_bytes() {
    // switch_3 = 11 with the 8-bit run-length field starting at bit 6: the
    // field spans two bytes, so both halves must be combined correctly.
    let mut b = Bits::new();
    b.value(0, 2)
        .value(0, 1)
        .value(0, 1)
        .value(0b11, 2)
        .value(0b1011_0100, 8)
        .value(3, 2)
        .value(2, 2) // one pixel of code 2
        .value(0, 2)
        .value(0, 1)
        .value(1, 1) // one pixel of code 0
        .value(0, 6);
    assert_eq!(
        runs(DataType::CodeString2Bit, &b.bytes()),
        vec![run(3, 209), run(2, 1), run(0, 1)]
    );
}
