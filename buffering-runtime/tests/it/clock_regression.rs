//! A caller clock that steps backwards on `on_track_media` is clamped to the latest instant seen,
//! like `on_hint`, so a regressed first delivery is not stamped as an old one and excluded as silent.

use buffering_runtime::{BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27};

const A: u32 = 1;
const B: u32 = 2;
const T0: Micros = 50_000_000;
const REGRESSION_US: Micros = 5_000_000;
const STEP_US: Micros = 40_000;
const STEPS: u64 = 15;
const FILL_US: Micros = 300_000;
const CHUNK_BYTES: u32 = 1316;
const TICKS_PER_US: Ticks27 = 27;

fn run(with_b: bool, regress_b_us: Micros) -> bool {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = FILL_US;
    let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap();
    let mut c = BufferController::new(cfg, None).unwrap();
    c.on_track_media(T0, A, 0);
    if with_b {
        c.on_track_media(T0 + STEP_US - regress_b_us, B, 0);
    }
    for k in 1..=STEPS {
        let now = T0 + k * STEP_US;
        c.on_chunk(now, CHUNK_BYTES, None);
        c.on_track_media(now, A, k * STEP_US * TICKS_PER_US);
        c.on_tick(now, None);
    }
    std::iter::from_fn(|| c.poll_event()).any(|e| matches!(e, Event::StartPlaying { .. }))
}

#[test]
fn control_without_b_starts_and_with_b_in_order_waits() {
    assert!(run(false, 0), "track A alone must start in this scenario");
    assert!(
        !run(true, 0),
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
