//! fMP4 recovery on bytes (DESIGN §3.6, 8.1 rebuild rows, #3676).

use dvr_runtime::recover::{RecoverError, RecoveredBox, scan_fmp4_data};

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + payload.len()).expect("size");
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn large_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u64::try_from(16 + payload.len()).expect("size");
    let mut bytes = 1u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn to_eof_mdat(payload: &[u8]) -> Vec<u8> {
    let mut bytes = 0u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(b"mdat");
    bytes.extend_from_slice(payload);
    bytes
}

fn init() -> Vec<u8> {
    let mut bytes = boxed(b"ftyp", b"isom0000");
    bytes.extend_from_slice(&boxed(b"moov", &[0x11; 24]));
    bytes
}

fn pair(content: u8) -> Vec<u8> {
    let mut bytes = boxed(b"moof", &[0x22; 8]);
    bytes.extend_from_slice(&boxed(b"mdat", &[content; 32]));
    bytes
}

fn spans(boxes: &[RecoveredBox]) -> Vec<(u64, u64)> {
    boxes
        .iter()
        .map(|recovered| match recovered {
            RecoveredBox::Init(span) | RecoveredBox::Fragment(span) => {
                (span.offset, span.len.get())
            }
            _ => panic!("unknown recovered box"),
        })
        .collect()
}

#[test]
fn three_moof_mdat_pairs_recover_exact_spans() {
    let init = init();
    let first = pair(0xAA);
    let second = pair(0xBB);
    let third = pair(0xCC);
    let mut data = init.clone();
    data.extend_from_slice(&first);
    data.extend_from_slice(&second);
    data.extend_from_slice(&third);
    let recovered = scan_fmp4_data(&data).expect("recovers");
    assert!(matches!(recovered[0], RecoveredBox::Init(_)));
    let init_len = init.len() as u64;
    let pair_len = first.len() as u64;
    assert_eq!(
        spans(&recovered),
        vec![
            (0, init_len),
            (init_len, pair_len),
            (init_len + pair_len, pair_len),
            (init_len + 2 * pair_len, pair_len),
        ]
    );
}

#[test]
fn a_moof_followed_by_free_is_a_fragment_by_itself() {
    let init = init();
    let moof = boxed(b"moof", &[0x22; 8]);
    let free = boxed(b"free", &[0; 8]);
    let pair = pair(0xDD);
    let mut data = init.clone();
    data.extend_from_slice(&moof);
    data.extend_from_slice(&free);
    data.extend_from_slice(&pair);
    let recovered = scan_fmp4_data(&data).expect("recovers");
    let init_len = init.len() as u64;
    let moof_len = moof.len() as u64;
    assert_eq!(
        spans(&recovered),
        vec![
            (0, init_len),
            (init_len, moof_len),
            (init_len + moof_len + free.len() as u64, pair.len() as u64),
        ]
    );
}

#[test]
fn trailing_bytes_after_the_last_mdat_are_ignored() {
    let init = init();
    let first = pair(0xAA);
    let second = pair(0xBB);
    let mut data = init.clone();
    data.extend_from_slice(&first);
    data.extend_from_slice(&second);
    data.extend_from_slice(&[0xFF; 5]);
    let recovered = scan_fmp4_data(&data).expect("recovers");
    assert_eq!(recovered.len(), 3);
    assert_eq!(spans(&recovered)[2].1, second.len() as u64);
}

#[test]
fn a_largesize_mdat_is_one_fragment_with_its_moof() {
    let init = init();
    let moof = boxed(b"moof", &[]);
    let mdat = large_box(b"mdat", &[0xAB; 32]);
    let mut data = init.clone();
    data.extend_from_slice(&moof);
    data.extend_from_slice(&mdat);
    let recovered = scan_fmp4_data(&data).expect("recovers");
    assert_eq!(recovered.len(), 2);
    assert_eq!(
        spans(&recovered)[1],
        (init.len() as u64, (moof.len() + mdat.len()) as u64)
    );
}

#[test]
fn a_to_eof_mdat_is_one_fragment_with_its_moof() {
    let init = init();
    let moof = boxed(b"moof", &[]);
    let mdat = to_eof_mdat(&[0xEF; 20]);
    let mut data = init.clone();
    data.extend_from_slice(&moof);
    data.extend_from_slice(&mdat);
    let recovered = scan_fmp4_data(&data).expect("recovers");
    assert_eq!(recovered.len(), 2);
    assert_eq!(
        spans(&recovered)[1],
        (init.len() as u64, (moof.len() + mdat.len()) as u64)
    );
}

#[test]
fn a_period_truncated_after_a_moof_does_not_panic() {
    let init = init();
    let mut data = init.clone();
    data.extend_from_slice(&boxed(b"moof", &[]));
    data.extend_from_slice(&[0; 5]);
    let recovered = scan_fmp4_data(&data).expect("recovers what landed");
    assert_eq!(recovered.len(), 2);
    assert_eq!(spans(&recovered)[1], (init.len() as u64, 8));
}

#[test]
fn a_period_with_no_init_is_typed() {
    let mut data = boxed(b"moof", &[]);
    data.extend_from_slice(&boxed(b"mdat", &[0; 8]));
    assert!(matches!(
        scan_fmp4_data(&data),
        Err(RecoverError::MissingInit)
    ));
}

#[test]
fn an_empty_or_broken_period_is_not_iso_bmff() {
    assert!(matches!(scan_fmp4_data(&[]), Err(RecoverError::NotIsoBmff)));
    assert!(matches!(
        scan_fmp4_data(&[0, 0, 0]),
        Err(RecoverError::NotIsoBmff)
    ));
    let mut short = init();
    short.truncate(4);
    assert!(matches!(
        scan_fmp4_data(&short),
        Err(RecoverError::NotIsoBmff)
    ));
}
