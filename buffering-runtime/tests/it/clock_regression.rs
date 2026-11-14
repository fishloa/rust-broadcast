//! A caller clock that steps backwards on `on_track_media` is clamped to the latest instant seen,
//! like `on_hint`, so a regressed first delivery is not stamped as an old one and excluded as silent.

use buffering_runtime::{BufferController, ControllerConfig, Event, Micros, PolicyConfig, Ticks27};

const A: u32 = 1;
const B: u32 = 2;
const T0: Micros = Micros(50_000_000);
const REGRESSION_US: Micros = Micros(5_000_000);
const STEP_US: Micros = Micros(40_000);
const STEPS: u64 = 15;
const FILL_US: Micros = Micros(300_000);
const CHUNK_BYTES: u32 = 1316;
const TICKS_PER_US: Ticks27 = Ticks27(27);

fn run(with_b: bool, regress_b_us: Micros) -> bool {
    let d = ControllerConfig::default();
    let mut p = PolicyConfig::default();
    p.initial_us = FILL_US;
    let cfg = ControllerConfig::new(d.jitter, p, d.starve_threshold_us, d.nodata_after_us).unwrap();
    let mut c = BufferController::new(cfg, None).unwrap();
    c.on_track_media(T0, A, Ticks27::ZERO);
    if with_b {
        c.on_track_media(Micros(T0.0 + STEP_US.0 - regress_b_us.0), B, Ticks27::ZERO);
    }
    for k in 1..=STEPS {
        let now = Micros(T0.0 + k * STEP_US.0);
        c.on_chunk(now, CHUNK_BYTES, None);
        c.on_track_media(now, A, Ticks27(k * STEP_US.0 * TICKS_PER_US.0));
        c.on_tick(now, None);
    }
    std::iter::from_fn(|| c.poll_event()).any(|e| matches!(e, Event::StartPlaying { .. }))
}

#[test]
fn control_without_b_starts_and_with_b_in_order_waits() {
    assert!(
        run(false, Micros::ZERO),
        "track A alone must start in this scenario"
    );
    assert!(
        !run(true, Micros::ZERO),
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
