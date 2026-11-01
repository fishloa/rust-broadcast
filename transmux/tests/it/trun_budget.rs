//! The zero-width `trun` allowance is AGGREGATE per `moof`, and breaching it is
//! a typed error (not a generic `InvalidInput`).

use transmux::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
use transmux::{Error, MovieFragmentBox};

fn boxed(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(body);
    b
}

fn moof_with_zero_width_truns(counts: &[u32]) -> Vec<u8> {
    let mfhd = boxed(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]);
    let mut traf_body = boxed(b"tfhd", &[0, 0x02, 0, 0, 0, 0, 0, 1]);
    for &c in counts {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&c.to_be_bytes());
        traf_body.extend_from_slice(&boxed(b"trun", &body));
    }
    [mfhd, boxed(b"traf", &traf_body)].concat()
}

#[test]
fn aggregate_zero_width_samples_over_the_cap_are_a_typed_error() {
    let half = (MAX_ZERO_WIDTH_TRUN_SAMPLES / 2) as u32;
    // Exactly at the cap: accepted. (Counts only, no per-sample bytes.)
    assert!(MovieFragmentBox::parse_body(&moof_with_zero_width_truns(&[half, half])).is_ok());
    // One sample over the aggregate cap, though each trun is far below it.
    let err = MovieFragmentBox::parse_body(&moof_with_zero_width_truns(&[half, half, 1]))
        .expect_err("aggregate cap");
    assert_eq!(
        err,
        Error::ZeroWidthTrunBudgetExceeded {
            limit: MAX_ZERO_WIDTH_TRUN_SAMPLES
        }
    );
}
