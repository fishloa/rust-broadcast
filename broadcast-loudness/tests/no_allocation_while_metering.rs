//! Issue #1729: gated integrated loudness and LRA used to keep one value per
//! 100 ms block, so a long-running meter grew without bound. `LoudnessMeter::new`
//! now allocates an exact block list per history sized to the 320 000-byte
//! histogram it later folds into. Pushing allocates nothing until a list is
//! full, then each history allocates its histogram once and frees its list,
//! and nothing after that. The counting allocator makes the unbounded design
//! fail on its first `Vec` growth. The run passes the fold point (66 min 40 s
//! of 100 ms blocks); under `RUST_BROADCAST_SOAK` it lasts twenty hours.

use broadcast_loudness::{ChannelLayout, LoudnessMeter, MIN_SAMPLE_RATE};
use test_alloc::ThreadCounting;

#[global_allocator]
static COUNTER: ThreadCounting = ThreadCounting::new();

const HISTOGRAM_BYTES: usize = 320_000;

fn minutes() -> usize {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        20 * 60
    } else {
        70
    }
}

fn push_minutes(m: &mut LoudnessMeter, minutes: usize) {
    for i in 0..MIN_SAMPLE_RATE as usize * 60 * minutes {
        let s = if i % 10 < 5 { 0.1f32 } else { -0.1 };
        m.push_f32(&[s]).unwrap();
    }
}

#[test]
fn metering_a_long_stream_allocates_only_the_one_histogram_fold() {
    let total = minutes();
    let mut m = LoudnessMeter::new(MIN_SAMPLE_RATE, ChannelLayout::Mono).unwrap();
    let ((), first_hour) = ThreadCounting::measure(|| push_minutes(&mut m, 60));
    assert_eq!(first_hour.allocs, 0, "{first_hour:?}");
    let ((), fold) = ThreadCounting::measure(|| push_minutes(&mut m, total - 60));
    assert_eq!(fold.allocs, 2, "{fold:?}");
    assert_eq!(fold.bytes, 2 * HISTOGRAM_BYTES, "{fold:?}");
    assert_eq!(fold.deallocs, 2, "{fold:?}");
    let ((), after) = ThreadCounting::measure(|| {
        push_minutes(&mut m, 5);
        m.finish();
    });
    assert_eq!(after.allocs, 0, "{after:?}");
    assert!(m.integrated_lufs().is_finite(), "{}", m.integrated_lufs());
}
