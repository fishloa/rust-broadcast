//! Box walkers must survive an untrusted `largesize` that overflows `usize`
//! arithmetic (ISO/IEC 14496-12:2015 §4.2 `Box.size` / `largesize`).

use broadcast_common::Parse;

const FOURCCS: &[&[u8; 4]] = &[
    b"avc1", b"avc3", b"hvc1", b"hev1", b"vvc1", b"mp4v", b"mp4a", b"enca", b"ac-3", b"ec-3",
    b"stpp", b"wvtt", b"av01", b"vp09", b"ac-4", b"Opus", b"fLaC", b"mha1", b"dtsc", b"avs3",
    b"avst", b"lav3", b"lav2", b"xxxx",
];

fn stsd(entry: &[u8]) -> Vec<u8> {
    let mut b = ((16 + entry.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(b"stsd");
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    b.extend_from_slice(entry);
    b
}

fn harness(data: &[u8]) {
    let Some((&sel, body)) = data.split_first() else {
        return;
    };
    let fourcc = FOURCCS[usize::from(sel) % FOURCCS.len()];
    let mut compact = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    compact.extend_from_slice(fourcc);
    compact.extend_from_slice(body);
    let mut large = 1u32.to_be_bytes().to_vec();
    large.extend_from_slice(fourcc);
    large.extend_from_slice(&((16 + body.len()) as u64).to_be_bytes());
    large.extend_from_slice(body);
    for entry in [&compact, &large] {
        let _ = transmux::SampleDescriptionBox::parse(&stsd(entry));
    }
    let _ = transmux::TrackExtendsBox::parse(data);
    let _ = transmux::SchemeInformationBox::parse(data);
    let _ = transmux::ProtectionSchemeInfoBox::parse(data);
}

/// Verbatim libFuzzer artifact `crash-5d5150b8d59a6b21d0a75cef5cdcc296f5099750`
/// (`transmux_sample_entries`): a child box with `largesize = u64::MAX` made
/// `find_config_box` panic on `off += size`.
const CRASH_5D5150B8: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x05, 0x05, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xfc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x05, 0x05, 0x05, 0x05, 0x05,
];

#[test]
fn fuzz_crash_5d5150b8_verbatim() {
    assert_eq!(CRASH_5D5150B8.len(), 143);
    harness(CRASH_5D5150B8);
}

fn largesize_max_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = 1u32.to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(&u64::MAX.to_be_bytes());
    b.extend_from_slice(body);
    b
}

fn full_box(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(body);
    b
}

#[test]
fn dref_child_with_overflowing_largesize_does_not_panic() {
    let mut body = vec![0, 0, 0, 0, 0, 0, 0, 1];
    body.extend(largesize_max_box(b"url ", &[0, 0, 0, 1]));
    let dref = full_box(b"dref", &body);
    let _ = transmux::DataReferenceBox::parse(&dref);
}

#[test]
fn stsd_entry_with_overflowing_largesize_does_not_panic() {
    for fourcc in FOURCCS {
        let entry = largesize_max_box(fourcc, &[0u8; 120]);
        let _ = transmux::SampleDescriptionBox::parse(&stsd(&entry));
    }
}

#[test]
fn sample_entry_config_child_with_overflowing_largesize_does_not_panic() {
    for fourcc in FOURCCS {
        let mut inner = vec![0u8; 78];
        inner.extend(largesize_max_box(b"avcC", &[0u8; 8]));
        let entry = full_box(fourcc, &inner);
        let _ = transmux::SampleDescriptionBox::parse(&stsd(&entry));
    }
}

#[test]
fn box_iter_rejects_overflowing_largesize() {
    let bytes = largesize_max_box(b"moov", &[0u8; 8]);
    let mut it = transmux::box_iter(&bytes);
    assert!(matches!(it.next(), Some(Err(_))));
}
