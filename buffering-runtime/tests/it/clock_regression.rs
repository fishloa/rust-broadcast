//! A caller clock that steps backwards on `on_track_media` is clamped to the latest instant seen,
//! like `on_hint`, so a regressed first delivery is not stamped as an old one and excluded as silent.
use broadcast_common::clock33::PCR_WRAP_27MHZ;

/// A 27 MHz tick count as a validated clock position.
fn clock27(ticks: u64) -> Clock27 {
    Clock27::from_27mhz(ticks % PCR_WRAP_27MHZ).expect("a reduced count is on the clock")
}

const fn dur(us: u64) -> Duration {
    Duration::from_micros(us)
}
use buffering_runtime::{BufferController, Clock27, ControllerConfig, Event, PolicyConfig};
use core::time::Duration;

const A: u32 = 1;
const B: u32 = 2;
const T0: Duration = dur(50_000_000);
const REGRESSION_US: Duration = dur(5_000_000);
const STEP_US: Duration = dur(40_000);
const STEPS: u64 = 15;
const FILL_US: Duration = dur(300_000);
const CHUNK_BYTES: u32 = 1316;
const TICKS_PER_US: u64 = 27;

fn run(with_b: bool, regress_b_us: Duration) -> bool {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial = FILL_US;
    let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold, d.nodata_after).unwrap();
    let mut c = BufferController::new(cfg, None).unwrap();
    c.on_track_media(T0, A, clock27(0));
    if with_b {
        c.on_track_media(T0 + STEP_US - regress_b_us, B, clock27(0));
    }
    for k in 1..=STEPS {
        let now = T0 + Duration::from_micros(k * STEP_US.as_micros() as u64);
        c.on_chunk(now, CHUNK_BYTES, None);
        c.on_track_media(
            now,
            A,
            clock27(k * STEP_US.as_micros() as u64 * TICKS_PER_US),
        );
        c.on_tick(now, None);
    }
    std::iter::from_fn(|| c.poll_event()).any(|e| matches!(e, Event::StartPlaying { .. }))
}

#[test]
fn control_without_b_starts_and_with_b_in_order_waits() {
    assert!(
        run(false, Duration::ZERO),
        "track A alone must start in this scenario"
    );
    assert!(
        !run(true, Duration::ZERO),
        "B has only its zero-length first report: the start waits for it"
    );
}

#[test]
fn a_regressed_first_delivery_is_not_excluded_as_silent() {
    assert!(
        !run(true, REGRESSION_US),
        "track B's first delivery was stamped REGRESSION_US in the past and excluded as silent at once"
    );
}
