//! Issue #1729: gated integrated loudness and LRA used to keep one value per
//! 100 ms block, so a long-running meter grew without bound. All of the
//! meter's state is now allocated by `LoudnessMeter::new`; pushing samples
//! and finishing must not allocate at all, however long the stream. The
//! counting allocator makes the unbounded design fail on its first `Vec`
//! growth. Five minutes by default; twenty hours under `RUST_BROADCAST_SOAK`.

use broadcast_loudness::{ChannelLayout, LoudnessMeter, MIN_SAMPLE_RATE};
use test_alloc::ThreadCounting;

#[global_allocator]
static COUNTER: ThreadCounting = ThreadCounting::new();

fn frames() -> usize {
    let minutes = if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        20 * 60
    } else {
        5
    };
    MIN_SAMPLE_RATE as usize * 60 * minutes
}

#[test]
fn metering_a_long_stream_allocates_nothing_after_construction() {
    let mut m = LoudnessMeter::new(MIN_SAMPLE_RATE, ChannelLayout::Mono).unwrap();
    let n = frames();
    let ((), counts) = ThreadCounting::measure(|| {
        for i in 0..n {
            let s = if i % 10 < 5 { 0.1f32 } else { -0.1 };
            m.push_f32(&[s]).unwrap();
        }
        m.finish();
    });
    assert_eq!(counts.allocs, 0, "{counts:?}");
    assert!(m.integrated_lufs().is_finite(), "{}", m.integrated_lufs());
}
