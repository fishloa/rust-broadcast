//! #2745: the pack scan must never drop a successfully parsed pack because a
//! later one failed, and the scan types must be exhaustive-proof.

use crate::psm_vectors;

use mpeg_ps::program_stream::{PackScan, SkippedRegion, scan_packs};

#[test]
fn a_scan_with_a_failed_pack_still_yields_the_packs_that_worked() {
    let data = psm_vectors::two_psms();
    let clean = scan_packs(&data);
    assert_eq!(clean.packs.len(), 2);
    assert_eq!(clean.skipped.len(), 0);

    let second = data
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == [0x00, 0x00, 0x01, 0xBA])
        .nth(1)
        .map(|(i, _)| i)
        .expect("two pack start codes");
    let mut damaged = data.clone();
    damaged[second] = 0xFF;
    let scan = scan_packs(&damaged);
    assert_eq!(scan.packs.len(), 1, "the first pack is still reported");
    assert_eq!(scan.packs[0].pes_packets.len(), 1);
    assert_eq!(scan.skipped.len(), 1);
    assert_eq!(scan.skipped[0].offset, second);
    assert_eq!(
        scan.skipped[0].len,
        data.len() - second,
        "the skipped span runs to the end code's offset"
    );
}

#[test]
fn the_scan_types_are_exhaustive_proof() {
    fn scan_parts(scan: &PackScan<'_>) -> (usize, usize, usize) {
        let PackScan {
            packs,
            skipped,
            remaining,
            ..
        } = scan;
        (packs.len(), skipped.len(), remaining.len())
    }
    fn skipped_parts(region: &SkippedRegion) -> usize {
        let SkippedRegion {
            offset, len, error, ..
        } = region;
        let _ = error;
        offset + len
    }
    let data = psm_vectors::two_psms();
    {
        let scan = scan_packs(&data);
        assert_eq!(scan_parts(&scan), (2, 0, 0));
    }
    let mut damaged = data;
    damaged[0] = 0xFF;
    let scan = scan_packs(&damaged);
    let (packs, skipped, _) = scan_parts(&scan);
    assert_eq!((packs, skipped), (1, 1));
    assert_eq!(
        skipped_parts(&scan.skipped[0]),
        scan.skipped[0].offset + scan.skipped[0].len
    );
}
