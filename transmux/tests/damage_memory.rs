//! Peak allocator bound for the largest damaged non-ISOBMFF input the damage
//! matrix uses. Its own test binary and its own process-global allocator, so
//! the measurement is not polluted by a sibling test.

use test_alloc::ProcessPeak;
use test_harness::damage::{Damage, apply};

#[global_allocator]
static ALLOC: ProcessPeak = ProcessPeak::new();

/// The damaged image must not make a streaming demuxer hold the input.
const PEAK_LIMIT: usize = 64 * 1024 * 1024;

fn demux_ts(image: &[u8]) {
    let mut demux = transmux::StreamingTsDemux::new();
    let _ = demux.feed(image);
    while demux.poll_event().is_some() {}
    let _ = demux.finish();
    while demux.poll_event().is_some() {}
}

fn demux_mkv(image: &[u8]) {
    let _ = test_harness::collect::media_from_matroska(
        image,
        test_harness::collect::CollectMode::Seekable,
    );
}

#[test]
fn a_damaged_input_does_not_materialise_in_memory() {
    let clean = test_fixtures::read("ts/h264-aac.ts");
    let mkv = test_fixtures::read("mkv/h264-aac.mkv");
    let mut worst = 0usize;
    for (name, seed) in [("ts", &clean), ("mkv", &mkv)] {
        for (at, damage) in [
            (seed.len() / 2, Damage::Zero),
            (seed.len() / 3, Damage::Random),
            (seed.len() / 4, Damage::Delete),
            (seed.len() / 5, Damage::Insert),
            (seed.len() * 9 / 10, Damage::Truncate),
        ] {
            let image = apply(seed, at, damage, 0xD00D);
            let (peak, ()) = ALLOC.peak_during(|| {
                if name == "ts" {
                    demux_ts(&image);
                } else {
                    demux_mkv(&image);
                }
            });
            println!("{name} {damage:?} at {at}: peak={peak} bytes above entry");
            worst = worst.max(peak);
        }
    }
    assert!(
        worst <= PEAK_LIMIT,
        "peak allocation {worst} exceeds {PEAK_LIMIT}"
    );
}
