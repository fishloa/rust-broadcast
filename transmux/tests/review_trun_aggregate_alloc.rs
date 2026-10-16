//! Adversarial review (review/transmux): the #1130 zero-width `trun` cap
//! (`MAX_ZERO_WIDTH_TRUN_SAMPLES`) is PER `trun`, so it does not bound a
//! `moof`. A zero-width `trun` (flags 0) is 16 bytes on the wire and may
//! legally declare 2^20 samples = 2^20 x 40-byte `TrunSample`s = 40 MiB. A
//! `traf` may carry any number of `trun`s (ISO/IEC 14496-12 §8.8.8: "zero or
//! more"), so N x 16 input bytes still cost N x 40 MiB: 16 `trun`s (256 bytes)
//! allocate ~640 MiB+. FAILS on the code under review.

use broadcast_common::Unpackage;
use test_alloc::ThreadCounting;
use transmux::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
use transmux::{Fmp4Demux, MovieFragmentBox};

#[global_allocator]
static ALLOC: ThreadCounting = ThreadCounting::new();

/// What a few hundred bytes of input may reasonably cost.
const BUDGET: usize = 256 << 20;
const TRUNS: usize = 16;

fn zero_width_trun(count: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&16u32.to_be_bytes());
    b.extend_from_slice(b"trun");
    b.extend_from_slice(&0u32.to_be_bytes()); // version 0, flags 0
    b.extend_from_slice(&count.to_be_bytes());
    b
}

fn boxed(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(body);
    b
}

#[test]
fn many_zero_width_truns_in_one_moof_stay_bounded() {
    let count = MAX_ZERO_WIDTH_TRUN_SAMPLES as u32;
    let mfhd = boxed(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]);
    // tfhd v0, flags 0x020000 (default-base-is-moof), track_ID 1.
    let tfhd = boxed(b"tfhd", &[0, 0x02, 0, 0, 0, 0, 0, 1]);
    let mut traf_body = tfhd;
    for _ in 0..TRUNS {
        traf_body.extend_from_slice(&zero_width_trun(count));
    }
    let traf = boxed(b"traf", &traf_body);
    let moof_body = [mfhd, traf].concat();
    assert!(moof_body.len() < 400, "a tiny input");

    let (res, snap) = ThreadCounting::measure(|| MovieFragmentBox::parse_body(&moof_body));
    drop(res);
    assert!(
        snap.bytes <= BUDGET,
        "a {}-byte moof made the parser allocate {} MiB",
        moof_body.len(),
        snap.bytes >> 20
    );
}

#[test]
fn many_zero_width_truns_through_fmp4_demux_stay_bounded() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mp4/llcmaf/ll_chunked.mp4"
    );
    let mut data = std::fs::read(path).expect("fixture");
    // Insert TRUNS zero-width truns right after the first real trun, growing
    // the enclosing traf and moof (both compact headers in this fixture).
    let trun_at = data.windows(4).position(|w| w == b"trun").unwrap() - 4;
    let trun_size = u32::from_be_bytes(data[trun_at..trun_at + 4].try_into().unwrap()) as usize;
    let traf_at = data[..trun_at]
        .windows(4)
        .rposition(|w| w == b"traf")
        .unwrap()
        - 4;
    let moof_at = data[..traf_at]
        .windows(4)
        .rposition(|w| w == b"moof")
        .unwrap()
        - 4;
    let extra: Vec<u8> = (0..TRUNS)
        .flat_map(|_| zero_width_trun(MAX_ZERO_WIDTH_TRUN_SAMPLES as u32))
        .collect();
    for at in [traf_at, moof_at] {
        let s = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) + extra.len() as u32;
        data[at..at + 4].copy_from_slice(&s.to_be_bytes());
    }
    let ins = trun_at + trun_size;
    data.splice(ins..ins, extra);

    let (res, snap) = ThreadCounting::measure(|| Fmp4Demux::new().unpackage(&data).map(|_| ()));
    drop(res);
    assert!(
        snap.bytes <= BUDGET,
        "{} extra input bytes made Fmp4Demux allocate {} MiB",
        TRUNS * 16,
        snap.bytes >> 20
    );
}
