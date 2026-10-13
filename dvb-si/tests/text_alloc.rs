//! Allocation bounds for Annex A text decoding (audit r02-O4): the decode
//! used to build an intermediate `Vec<u16>` for UCS-2 and then a second
//! `String` just to strip control codes that were usually absent.

use dvb_si::text::decode_dvb_string;
use test_alloc::ThreadCounting;

#[global_allocator]
static A: ThreadCounting = ThreadCounting::new();

/// Charset selector 0x01 = ISO 8859-5 (Cyrillic); no control codes inside.
fn iso8859_5() -> Vec<u8> {
    let mut v = vec![0x01];
    v.extend((0..200).map(|i| 0xB0 + (i % 0x40) as u8));
    v
}

/// Selector 0x11 = ISO/IEC 10646 BMP (UCS-2 BE); no control codes inside.
fn ucs2() -> Vec<u8> {
    let mut v = vec![0x11];
    for i in 0..100u16 {
        v.extend_from_slice(&(0x0410 + i % 32).to_be_bytes());
    }
    v
}

#[test]
fn iso8859_5_decode_is_one_allocation() {
    let raw = iso8859_5();
    let (s, snap) = ThreadCounting::measure(|| decode_dvb_string(&raw));
    assert_eq!(s.chars().count(), 200);
    assert!(
        snap.allocs <= 1,
        "ISO 8859-5 decode allocated {} times",
        snap.allocs
    );
}

#[test]
fn ucs2_decode_has_no_intermediate_vec() {
    let raw = ucs2();
    let (s, snap) = ThreadCounting::measure(|| decode_dvb_string(&raw));
    assert_eq!(s.chars().count(), 100);
    assert!(
        snap.allocs <= 2,
        "UCS-2 decode allocated {} times",
        snap.allocs
    );
}

#[test]
fn control_codes_are_still_stripped_and_crlf_becomes_space() {
    // 0x8A (CR/LF) -> space, 0x86/0x87 stripped, plain text untouched.
    let s = decode_dvb_string(&[b'A', 0x86, b'B', 0x8A, b'C', 0x87]);
    assert_eq!(s, "AB C");
}
