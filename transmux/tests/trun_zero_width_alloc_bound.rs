//! `trun` zero-width-entry alloc/CPU bound (#1130).
//!
//! A `trun` whose flags select no per-sample field has entries that are ZERO
//! bytes wide, so the "samples must fit in the remaining bytes" bound cannot
//! limit its `sample_count`. A 20-byte `trun` declaring `0x1000_0000` samples
//! made `Fmp4Demux` push 268 M `TrunSample`s (a 10 GB `Vec`) from a real
//! LL-CMAF fixture with 4 bytes overwritten. Found by an offset sweep that
//! inflated every 4-byte window of the committed fixtures.
//!
//! The allocator caps this thread's live bytes at 256 MiB so the pre-fix
//! behaviour fails (aborts) instead of exhausting the host.

use broadcast_common::Unpackage;
use test_alloc::ThreadCapped;
use transmux::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
use transmux::{Fmp4Demux, TrackFragmentRunBox};

#[global_allocator]
static ALLOC: ThreadCapped<{ 256 << 20 }> = ThreadCapped::new();

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/mp4/llcmaf/ll_chunked.mp4"
);

/// Offset of the `trun` box's `version|flags` word (just after its fourcc).
fn trun_flags_offset(data: &[u8]) -> usize {
    let at = data
        .windows(4)
        .position(|w| w == b"trun")
        .expect("fixture has a trun");
    at + 4
}

/// The whole fixture with its first `trun`'s flags/sample_count replaced.
fn with_trun(sample_count: u32, flags: u32) -> Vec<u8> {
    let mut data = std::fs::read(FIXTURE).expect("fixture");
    let off = trun_flags_offset(&data);
    data[off..off + 4].copy_from_slice(&flags.to_be_bytes());
    data[off + 4..off + 8].copy_from_slice(&sample_count.to_be_bytes());
    data
}

/// The real fixture's `trun` box body (`version|flags` onwards), with flags and
/// sample_count replaced.
fn trun_box(sample_count: u32, flags: u32) -> Vec<u8> {
    let data = std::fs::read(FIXTURE).expect("fixture");
    let off = trun_flags_offset(&data);
    let size =
        u32::from_be_bytes([data[off - 8], data[off - 7], data[off - 6], data[off - 5]]) as usize;
    let mut b = data[off..off - 8 + size].to_vec();
    b[0..4].copy_from_slice(&flags.to_be_bytes());
    b[4..8].copy_from_slice(&sample_count.to_be_bytes());
    b
}

#[test]
fn unmodified_fixture_still_demuxes() {
    let data = std::fs::read(FIXTURE).expect("fixture");
    Fmp4Demux::new()
        .unpackage(&data)
        .expect("real LL-CMAF fixture demuxes");
}

#[test]
fn zero_width_trun_with_hostile_sample_count_is_rejected_not_allocated() {
    // flags = 0: no data_offset, no per-sample fields -> zero-width entries.
    for count in [0x1000_0000u32, 0x7FFF_FFFF, u32::MAX] {
        let err = TrackFragmentRunBox::parse_body(&trun_box(count, 0))
            .expect_err("hostile sample_count must be rejected");
        assert!(
            format!("{err}").contains("MAX_ZERO_WIDTH_TRUN_SAMPLES"),
            "sample_count {count:#x}: unexpected error {err}"
        );
    }
}

#[test]
fn zero_width_trun_at_the_cap_still_parses() {
    let run = TrackFragmentRunBox::parse_body(&trun_box(MAX_ZERO_WIDTH_TRUN_SAMPLES as u32, 0))
        .expect("a count equal to the cap is legal");
    assert_eq!(run.samples.len(), MAX_ZERO_WIDTH_TRUN_SAMPLES);
}

/// End to end: the demuxer survives the hostile fixture without allocating
/// anything near the declared 10 GB (the capped allocator aborts otherwise).
#[test]
fn demuxing_the_hostile_fixture_stays_bounded() {
    let data = with_trun(0x1000_0000, 0);
    let _ = Fmp4Demux::new().unpackage(&data);
}
