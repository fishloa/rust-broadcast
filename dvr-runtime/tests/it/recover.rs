//! fMP4 recovery on bytes (DESIGN §3.6, 8.1 rebuild rows, #3676).

use dvr_runtime::recover::{BOX_HEADER_MAX, Fmp4Scan, RecoverError, RecoveredBox, ScanStep};

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
    let recovered = scan(&data).expect("recovers");
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
fn a_segment_type_box_before_a_moof_belongs_to_that_fragment() {
    let init = init();
    let styp = boxed(b"styp", b"msdh0000");
    let first = pair(0xAA);
    let mut data = init.clone();
    data.extend_from_slice(&styp);
    data.extend_from_slice(&first);
    data.extend_from_slice(&styp);
    data.extend_from_slice(&pair(0xBB));
    let recovered = scan(&data).expect("recovers");
    let init_len = init.len() as u64;
    let segment_len = (styp.len() + first.len()) as u64;
    assert_eq!(
        spans(&recovered),
        vec![
            (0, init_len),
            (init_len, segment_len),
            (init_len + segment_len, segment_len),
        ],
        "a CMAF segment starts at its styp, not at its moof"
    );
}

#[test]
fn a_producer_reference_time_box_before_a_moof_belongs_to_that_fragment() {
    let init = init();
    let prft = boxed(b"prft", &[0x44; 12]);
    let first = pair(0xAA);
    let mut data = init.clone();
    data.extend_from_slice(&prft);
    data.extend_from_slice(&first);
    let recovered = scan(&data).expect("recovers");
    assert_eq!(
        spans(&recovered)[1],
        (init.len() as u64, (prft.len() + first.len()) as u64)
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
    let recovered = scan(&data).expect("recovers");
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
    let recovered = scan(&data).expect("recovers");
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
    let recovered = scan(&data).expect("recovers");
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
    let recovered = scan(&data).expect("recovers");
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
    let recovered = scan(&data).expect("recovers what landed");
    assert_eq!(recovered.len(), 2);
    assert_eq!(spans(&recovered)[1], (init.len() as u64, 8));
}

#[test]
fn a_period_with_no_init_is_typed() {
    let mut data = boxed(b"moof", &[]);
    data.extend_from_slice(&boxed(b"mdat", &[0; 8]));
    assert!(matches!(scan(&data), Err(RecoverError::MissingInit)));
}

#[test]
fn an_empty_or_broken_period_is_not_iso_bmff() {
    assert!(matches!(scan(&[]), Err(RecoverError::NotIsoBmff)));
    assert!(matches!(scan(&[0, 0, 0]), Err(RecoverError::NotIsoBmff)));
    let mut short = init();
    short.truncate(4);
    assert!(matches!(scan(&short), Err(RecoverError::NotIsoBmff)));
}

/// Most `step`/`supply` rounds one drive may take before the test calls it a
/// non-terminating scan.
const MAX_STEPS: usize = 1 << 20;

fn scan(data: &[u8]) -> Result<Vec<RecoveredBox>, RecoverError> {
    scan_with(Fmp4Scan::new(data.len() as u64), data)
}

fn scan_with(scan: Fmp4Scan, data: &[u8]) -> Result<Vec<RecoveredBox>, RecoverError> {
    drive_scan(scan, data).map(|(_, boxes)| boxes)
}

/// Drive a default scan over `data`.
fn drive(data: &[u8]) -> Result<(usize, Vec<RecoveredBox>), RecoverError> {
    drive_scan(Fmp4Scan::new(data.len() as u64), data)
}

/// Drive `scan` over `data`, reporting the number of bytes it asked for as
/// well as the recovered boxes.
fn drive_scan(mut scan: Fmp4Scan, data: &[u8]) -> Result<(usize, Vec<RecoveredBox>), RecoverError> {
    let mut supplied = 0usize;
    for _ in 0..MAX_STEPS {
        match scan.step()? {
            ScanStep::Need { offset, len } => {
                assert!(
                    len <= BOX_HEADER_MAX,
                    "asked for {len} bytes, more than a box header"
                );
                let start = usize::try_from(offset).expect("offset fits a usize");
                assert!(
                    start.saturating_add(len) <= data.len(),
                    "asked for bytes past the end of the file"
                );
                scan.supply(offset, &data[start..start + len])?;
                supplied += len;
            }
            ScanStep::Done(boxes) => return Ok((supplied, boxes)),
            _ => panic!("unknown scan step"),
        }
    }
    panic!("the scan did not finish within {MAX_STEPS} steps");
}

#[test]
fn the_scan_only_ever_asks_for_box_headers() {
    let init = init();
    let mut data = init.clone();
    data.extend_from_slice(&boxed(b"styp", b"msdh0000"));
    data.extend_from_slice(&pair(0xAA));
    data.extend_from_slice(&pair(0xBB));
    let boxes = 7;
    let (supplied, recovered) = drive(&data).expect("recovers");
    assert_eq!(recovered.len(), 3);
    assert!(
        supplied <= BOX_HEADER_MAX * (boxes + 1),
        "{supplied} bytes supplied for {boxes} boxes"
    );
}

#[test]
fn a_declared_size_past_the_file_end_stops_at_the_last_whole_box() {
    let init = init();
    let whole = pair(0xAA);
    let mut data = init.clone();
    data.extend_from_slice(&whole);
    let mut torn_moof = 0x0100_0000u32.to_be_bytes().to_vec();
    torn_moof.extend_from_slice(b"moof");
    torn_moof.extend_from_slice(&[0x22; 8]);
    data.extend_from_slice(&torn_moof);
    let recovered = scan(&data).expect("recovers what landed");
    let init_len = init.len() as u64;
    assert_eq!(
        spans(&recovered),
        vec![(0, init_len), (init_len, whole.len() as u64)]
    );

    let mut torn_mdat = init.clone();
    let moof = boxed(b"moof", &[0x22; 8]);
    torn_mdat.extend_from_slice(&moof);
    let mut mdat = 0x0100_0000u32.to_be_bytes().to_vec();
    mdat.extend_from_slice(b"mdat");
    mdat.extend_from_slice(&[0x33; 8]);
    torn_mdat.extend_from_slice(&mdat);
    let recovered = scan(&torn_mdat).expect("recovers what landed");
    assert_eq!(
        spans(&recovered),
        vec![(0, init_len), (init_len, moof.len() as u64)]
    );
}

#[test]
fn a_zero_size_box_and_a_size_smaller_than_its_header_end_the_scan_without_looping() {
    let init = init();
    let moof = boxed(b"moof", &[]);
    let mut to_eof = init.clone();
    to_eof.extend_from_slice(&moof);
    to_eof.extend_from_slice(&to_eof_mdat(&[0xEF; 20]));
    let recovered = scan(&to_eof).expect("recovers");
    let init_len = init.len() as u64;
    assert_eq!(
        spans(&recovered),
        vec![(0, init_len), (init_len, to_eof.len() as u64 - init_len)]
    );

    let mut underflowing = init.clone();
    underflowing.extend_from_slice(&pair(0xAA));
    underflowing.extend_from_slice(&4u32.to_be_bytes());
    underflowing.extend_from_slice(b"free");
    underflowing.extend_from_slice(&[0; 8]);
    let recovered = scan(&underflowing).expect("recovers what landed");
    assert_eq!(recovered.len(), 2);
}

#[test]
fn a_64_bit_largesize_mdat_is_skipped_by_its_size() {
    let init = init();
    let moof = boxed(b"moof", &[0x22; 8]);
    let mdat = large_box(b"mdat", &[0xAB; 4096]);
    let after = pair(0xCC);
    let mut data = init.clone();
    data.extend_from_slice(&moof);
    data.extend_from_slice(&mdat);
    data.extend_from_slice(&after);
    let recovered = scan(&data).expect("recovers");
    let init_len = init.len() as u64;
    let first_len = (moof.len() + mdat.len()) as u64;
    assert_eq!(
        spans(&recovered),
        vec![
            (0, init_len),
            (init_len, first_len),
            (init_len + first_len, after.len() as u64),
        ]
    );
}

#[test]
fn a_stray_supply_is_typed_and_a_step_after_done_reports_an_empty_list() {
    let data = init();
    let mut scan = Fmp4Scan::new(data.len() as u64);
    let mut finished = None;
    for _ in 0..MAX_STEPS {
        match scan.step().expect("steps") {
            ScanStep::Need { offset, len } => {
                let start = offset as usize;
                assert_eq!(
                    scan.supply(offset, &data[start..start + len - 1]),
                    Err(RecoverError::UnexpectedSupply),
                    "a short supply does not answer the need"
                );
                scan.supply(offset, &data[start..start + len])
                    .expect("supply");
            }
            ScanStep::Done(recovered) => {
                finished = Some(recovered);
                break;
            }
            _ => panic!("unknown scan step"),
        }
    }
    let recovered = finished.expect("finishes");
    assert_eq!(spans(&recovered), vec![(0, data.len() as u64)]);
    assert_eq!(
        scan.step().expect("steps"),
        ScanStep::Done(Vec::new()),
        "the list was handed over, so a second step reports an empty one"
    );
    assert_eq!(
        scan.supply(0, &data[..4]),
        Err(RecoverError::UnexpectedSupply)
    );
}

#[test]
fn a_file_with_more_fragments_than_the_limit_is_a_typed_error() {
    let init = init();
    let limit = 3;
    let mut four = init.clone();
    for _ in 0..4 {
        four.extend_from_slice(&pair(0xAA));
    }
    assert_eq!(
        drive_scan(
            Fmp4Scan::new(four.len() as u64).with_max_fragments(limit),
            &four
        ),
        Err(RecoverError::TooManyFragments { limit })
    );
    let mut three = init.clone();
    for _ in 0..3 {
        three.extend_from_slice(&pair(0xAA));
    }
    let recovered = scan_with(
        Fmp4Scan::new(three.len() as u64).with_max_fragments(limit),
        &three,
    )
    .expect("three fragments fit the limit");
    assert_eq!(recovered.len(), 4);
}

#[test]
fn a_file_of_tiny_boxes_cannot_grow_the_result_past_the_limit() {
    let init = init();
    let mut data = init.clone();
    for _ in 0..10_000 {
        data.extend_from_slice(&boxed(b"moof", &[]));
    }
    let limit = 100;
    let mut scan = Fmp4Scan::new(data.len() as u64).with_max_fragments(limit);
    let mut needs = 0usize;
    let outcome = loop {
        match scan.step().expect("steps") {
            ScanStep::Need { offset, len } => {
                needs += 1;
                let start = usize::try_from(offset).expect("offset fits a usize");
                if let Err(error) = scan.supply(offset, &data[start..start + len]) {
                    break Err(error);
                }
            }
            ScanStep::Done(boxes) => break Ok(boxes),
            _ => panic!("unknown scan step"),
        }
    };
    assert_eq!(outcome, Err(RecoverError::TooManyFragments { limit }));
    let init_boxes = 2;
    assert!(
        needs <= limit + init_boxes + 2,
        "{needs} headers read for a {limit}-fragment limit"
    );
}
