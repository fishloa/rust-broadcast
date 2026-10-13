//! r14-RTCP-O1 (#1123): text fields are UTF-8 validated *before* a `String`
//! is allocated, so a hostile packet with a long invalid text field costs no
//! allocation of the text's size (it used to `to_vec()` first, then validate).

use broadcast_common::Parse;
use rtcp_packet::{Bye, SourceDescription};
use test_alloc::ThreadCounting;

#[global_allocator]
static ALLOC: ThreadCounting = ThreadCounting::new();

const TEXT_LEN: usize = 200;

fn bye_bytes(text: &[u8]) -> Vec<u8> {
    let mut v = vec![0x81, 203, 0, 0];
    v.extend_from_slice(&1u32.to_be_bytes());
    v.push(text.len() as u8);
    v.extend_from_slice(text);
    while v.len() % 4 != 0 {
        v.push(0);
    }
    let words = (v.len() / 4 - 1) as u16;
    v[2..4].copy_from_slice(&words.to_be_bytes());
    v
}

fn sdes_bytes(text: &[u8]) -> Vec<u8> {
    let mut v = vec![0x81, 202, 0, 0];
    v.extend_from_slice(&1u32.to_be_bytes());
    v.push(1); // CNAME
    v.push(text.len() as u8);
    v.extend_from_slice(text);
    v.push(0); // terminator
    while v.len() % 4 != 0 {
        v.push(0);
    }
    let words = (v.len() / 4 - 1) as u16;
    v[2..4].copy_from_slice(&words.to_be_bytes());
    v
}

#[test]
fn invalid_utf8_bye_reason_allocates_no_text_copy() {
    let bytes = bye_bytes(&[0xFF; TEXT_LEN]);
    let (r, snap) = ThreadCounting::measure(|| Bye::parse(&bytes));
    assert!(r.is_err());
    assert!(snap.bytes < TEXT_LEN, "allocated {} bytes", snap.bytes);
}

#[test]
fn invalid_utf8_sdes_text_allocates_no_text_copy() {
    let bytes = sdes_bytes(&[0xFF; TEXT_LEN]);
    let (r, snap) = ThreadCounting::measure(|| SourceDescription::parse(&bytes));
    assert!(r.is_err());
    assert!(snap.bytes < TEXT_LEN, "allocated {} bytes", snap.bytes);
}

#[test]
fn valid_text_still_parses() {
    let text = "a".repeat(TEXT_LEN);
    let sd = SourceDescription::parse(&sdes_bytes(text.as_bytes())).unwrap();
    assert_eq!(sd.chunks[0].items[0].text, text);
    let bye = Bye::parse(&bye_bytes(text.as_bytes())).unwrap();
    assert_eq!(bye.reason.as_deref(), Some(text.as_str()));
}
