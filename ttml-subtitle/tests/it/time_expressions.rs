//! Boundary values for the three `<time-expression>` forms (#3077): every
//! field's maximum valid value and maximum+1, plus each grammar branch the
//! clock-time/offset-time/wallclock parsers gate on.
//!
//! Grammar: W3C TTML2 §12.3.1
//! (`private/specs-verified/w3c/ttml-subtitle/ttml2-syntax.md`).

use ttml_subtitle::error::Error;
use ttml_subtitle::time::{
    parse_time_expression as parse, TimeBase, TimeContext, TimeExpression, TimeMetric, WallclockForm,
};

fn ctx() -> TimeContext {
    TimeContext::default()
}

fn is_err(input: &str, ctx: &TimeContext) -> bool {
    matches!(parse(input, ctx), Err(Error::InvalidTimeExpression { .. }))
}

fn clock(hours: u32, minutes: u8, seconds: u8) -> TimeExpression {
    TimeExpression::ClockTime {
        hours,
        minutes,
        seconds,
        fraction: None,
        frames: None,
        sub_frames: None,
    }
}

#[test]
fn clock_time_minutes_max_and_over() {
    assert_eq!(parse("00:59:00", &ctx()).unwrap(), clock(0, 59, 0));
    assert!(is_err("00:60:00", &ctx()));
}

#[test]
fn clock_time_seconds_max_leap_and_over() {
    assert_eq!(parse("00:00:60", &ctx()).unwrap(), clock(0, 0, 60));
    assert!(is_err("00:00:61", &ctx()));
}

#[test]
fn clock_time_hours_digit_count() {
    assert_eq!(parse("100:00:00", &ctx()).unwrap(), clock(100, 0, 0));
    assert!(is_err("5:00:00", &ctx()));
}

#[test]
fn clock_time_frames_max_and_over() {
    let ctx = ctx();
    assert!(parse("00:00:00:29", &ctx).is_ok());
    assert!(is_err("00:00:00:30", &ctx));
}

#[test]
fn clock_time_sub_frame_max_and_over() {
    let ctx = TimeContext {
        sub_frame_rate: 10,
        ..ctx()
    };
    assert!(parse("00:00:00:05.9", &ctx).is_ok());
    assert!(is_err("00:00:00:05.10", &ctx));
}

#[test]
fn sub_frame_check_is_skipped_when_sub_frame_rate_is_zero() {
    let ctx = TimeContext {
        sub_frame_rate: 0,
        ..ctx()
    };
    assert!(parse("00:00:00:05.9", &ctx).is_ok());
}

#[test]
fn zero_frame_rate_falls_back_to_thirty() {
    let ctx = TimeContext {
        frame_rate: 0,
        ..ctx()
    };
    assert!(parse("00:00:00:05", &ctx).is_ok());
}

#[test]
fn clock_time_rejects_empty_or_non_digit_seconds_fraction() {
    assert!(is_err("00:00:01.", &ctx()));
    assert!(is_err("00:00:01.x", &ctx()));
    assert!(parse("00:00:01.5", &ctx()).is_ok());
}

#[test]
fn clock_time_rejects_empty_or_non_digit_sub_frames() {
    assert!(is_err("00:00:00:05.", &ctx()));
    assert!(is_err("00:00:00:05.x", &ctx()));
    assert!(is_err("00:00:00:05.+5", &ctx()));
    let wide = TimeContext {
        sub_frame_rate: 10,
        ..TimeContext::default()
    };
    assert!(is_err("00:00:00:05.+0", &wide));
}

#[test]
fn seconds_fraction_is_exact() {
    assert_eq!(
        parse("00:00:01.5", &ctx()).unwrap(),
        TimeExpression::ClockTime {
            hours: 0,
            minutes: 0,
            seconds: 1,
            fraction: Some("5".into()),
            frames: None,
            sub_frames: None,
        }
    );
}

#[test]
fn offset_time_metrics() {
    let ctx = ctx();
    for (input, count, metric) in [
        ("1.5s", 1, TimeMetric::S),
        ("10f", 10, TimeMetric::F),
        ("3t", 3, TimeMetric::T),
        ("1h", 1, TimeMetric::H),
        ("500ms", 500, TimeMetric::Ms),
        ("2m", 2, TimeMetric::M),
    ] {
        match parse(input, &ctx).unwrap() {
            TimeExpression::OffsetTime {
                count: c, metric: m, ..
            } => {
                assert_eq!(c, count, "{input}");
                assert_eq!(m, metric, "{input}");
            }
            other => panic!("{input}: expected OffsetTime, got {other:?}"),
        }
    }
}

#[test]
fn offset_time_grammar_errors() {
    assert!(is_err("s", &ctx()));
    assert!(is_err("-1s", &ctx()));
    assert!(is_err("1.s", &ctx()));
    assert!(is_err("1.xs", &ctx()));
    assert!(is_err("xs", &ctx()));
    assert!(is_err("+5s", &ctx()));
}

#[test]
fn wallclock_date_boundaries() {
    let ctx = TimeContext {
        time_base: TimeBase::Clock,
        ..ctx()
    };
    assert!(parse("wallclock(2024-02-29)", &ctx).is_ok());
    assert!(parse("wallclock(2024-12-31)", &ctx).is_ok());
    assert!(is_err("wallclock(2024-13-01)", &ctx));
    assert!(is_err("wallclock(2024-00-01)", &ctx));
    assert!(is_err("wallclock(2024-01-00)", &ctx));
    assert!(is_err("wallclock(2024-01-32)", &ctx));
}

#[test]
fn wallclock_wall_time_boundaries() {
    let ctx = TimeContext {
        time_base: TimeBase::Clock,
        ..ctx()
    };
    assert!(parse("wallclock(23:59)", &ctx).is_ok());
    assert!(parse("wallclock(23:59:59)", &ctx).is_ok());
    assert!(parse("wallclock(23:59:59.999)", &ctx).is_ok());
    assert!(is_err("wallclock(24:00)", &ctx));
    assert!(is_err("wallclock(23:60)", &ctx));
    assert!(is_err("wallclock(10:30:61)", &ctx));
    assert!(parse("wallclock(10:30:60)", &ctx).is_ok());
}

#[test]
fn wallclock_time_component_count() {
    let ctx = TimeContext {
        time_base: TimeBase::Clock,
        ..ctx()
    };
    assert!(parse("wallclock(10:30)", &ctx).is_ok());
    assert!(parse("wallclock(10:30:45)", &ctx).is_ok());
    assert!(is_err("wallclock(10)", &ctx));
    assert!(is_err("wallclock(10:30:45:00)", &ctx));
}

#[test]
fn wallclock_seconds_fraction_grammar() {
    let ctx = TimeContext {
        time_base: TimeBase::Clock,
        ..ctx()
    };
    assert!(parse("wallclock(10:30:45.5)", &ctx).is_ok());
    assert!(is_err("wallclock(10:30:45.)", &ctx));
    assert!(is_err("wallclock(10:30:45.x)", &ctx));
}

#[test]
fn wallclock_date_form_and_fields() {
    let ctx = TimeContext {
        time_base: TimeBase::Clock,
        ..ctx()
    };
    match parse("wallclock(2024-12-31)", &ctx).unwrap() {
        TimeExpression::WallclockTime {
            form: WallclockForm::Date {
                years,
                months,
                days,
            },
        } => {
            assert_eq!((years, months, days), (2024, 12, 31));
        }
        other => panic!("expected Date, got {other:?}"),
    }
}
