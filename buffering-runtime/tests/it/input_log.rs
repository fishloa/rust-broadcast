use buffering_runtime::input_log::{ConfigKey, Input, InputLog, InputLogError, Reason};
use buffering_runtime::{
    BufferController, ControllerConfig, Event, Hint, Micros, PolicyConfig, StallMemorySnapshot,
    Ticks27,
};

const fn us(v: u64) -> Micros {
    Micros(v)
}

const fn t(v: u64) -> Ticks27 {
    Ticks27(v)
}

fn every_variant() -> Vec<Input> {
    let mut v = vec![
        Input::Chunk {
            now_us: us(1),
            bytes: 1316,
            pcr: Some(t(27_000_000)),
        },
        Input::Chunk {
            now_us: us(2),
            bytes: 0,
            pcr: None,
        },
        Input::TrackMedia {
            now_us: us(3),
            track: 7,
            end: t(u64::MAX),
        },
        Input::Tick {
            now_us: us(4),
            pts: Some(t(0)),
        },
        Input::Tick {
            now_us: us(5),
            pts: None,
        },
        Input::Register {
            now_us: us(6),
            track: u32::MAX,
        },
        Input::Unregister {
            now_us: us(7),
            track: 1,
        },
    ];
    v.extend((1..).map_while(Hint::from_code).map(|hint| Input::Hint {
        now_us: us(8),
        hint,
    }));
    v
}

fn row(i: &Input) -> String {
    let mut s = String::new();
    i.write_row(&mut s).unwrap();
    s
}

#[test]
fn every_input_round_trips_through_its_row() {
    for i in every_variant() {
        let text = row(&i);
        let back = Input::parse_row(&text).unwrap();
        assert_eq!(back, i, "{text}");
        assert_eq!(row(&back), text);
    }
}

#[test]
fn rows_match_the_documented_shapes() {
    assert_eq!(
        row(&Input::Chunk {
            now_us: us(9),
            bytes: 188,
            pcr: None
        }),
        "chunk,9,188,"
    );
    assert_eq!(
        row(&Input::Tick {
            now_us: us(9),
            pts: None
        }),
        "tick,9,0,"
    );
    assert_eq!(
        row(&Input::TrackMedia {
            now_us: us(9),
            track: 2,
            end: t(5)
        }),
        "media,9,2,5"
    );
    assert_eq!(
        row(&Input::Hint {
            now_us: us(9),
            hint: Hint::Reconnected
        }),
        "hint,9,reconnected,"
    );
}

#[test]
fn malformed_rows_are_typed_errors_carrying_the_line() {
    let cases: [(&str, Reason); 6] = [
        ("bogus,1,2,3", Reason::UnknownKind),
        ("chunk,x,1,", Reason::BadNumber { field: "now_us" }),
        ("chunk,1,,", Reason::MissingField { field: "bytes" }),
        ("media,1,2,zz", Reason::BadNumber { field: "end" }),
        ("hint,1,nonsense,", Reason::UnknownHint),
        (
            "tick,1,2,3",
            Reason::BadNumber {
                field: "has_position",
            },
        ),
    ];
    for (line, reason) in cases {
        let e: InputLogError = Input::parse_row(line).unwrap_err();
        assert_eq!(e.reason, reason, "{line}");
        assert_eq!(e.line, line);
        assert!(e.to_string().contains(line));
    }
}

fn custom_config() -> ControllerConfig {
    let mut c = ControllerConfig::default();
    let mut p = PolicyConfig::DEFAULT;
    p.initial_us = us(500_000);
    c.policy = p;
    c.track_granularity = false;
    c.track_silence_timeout_us = us(5_500_000);
    c
}

#[test]
fn a_log_round_trips_with_config_seed_and_inputs() {
    let log = InputLog::new(
        custom_config(),
        Some(StallMemorySnapshot::new(us(120_000), us(3_000))),
        every_variant(),
    );
    let mut text = String::new();
    log.write(&mut text).unwrap();
    assert_eq!(
        text.lines().filter(|l| l.starts_with("config,")).count(),
        ConfigKey::ALL.len()
    );
    let back = InputLog::parse(&text).unwrap();
    assert_eq!(back, log);
    let mut again = String::new();
    back.write(&mut again).unwrap();
    assert_eq!(again, text);
}

#[test]
fn every_config_field_is_covered_by_a_key() {
    let mut c = ControllerConfig::default();
    c.jitter.min_window_us += us(1);
    c.jitter.interval_us += us(1);
    c.jitter.outlier_us += us(1);
    c.jitter.bucket_us += 1;
    c.jitter.buckets += 1;
    c.jitter.forget_q15 += 1;
    c.jitter.quantile_permille -= 1;
    c.jitter.hold_us += us(1);
    c.jitter.decay_us += us(1);
    c.jitter.max_samples += 1;
    c.jitter.discontinuity_us += us(1);
    c.policy.min_us += us(1);
    c.policy.initial_us += us(1);
    c.policy.max_us -= us(1);
    c.policy.k1_q8 += 1;
    c.policy.margin_us += us(1);
    c.policy.k2_q8 += 1;
    c.policy.after_rebuffer_q8 += 1;
    c.policy.max_step_down_us_per_s += 1;
    c.policy.reprime_floor_us += us(1);
    c.policy.reprime_hold_us += us(1);
    c.starve_threshold_us += us(1);
    c.nodata_after_us += us(1);
    c.track_silence_timeout_us += us(1);
    c.track_granularity = !c.track_granularity;
    c.granularity_hold_us += us(1);
    c.granularity_decay_us += us(1);
    c.validate().unwrap();
    let mut text = String::new();
    InputLog::new(c, None, vec![]).write(&mut text).unwrap();
    assert_eq!(InputLog::parse(&text).unwrap().config, c);
}

#[test]
fn a_log_without_config_rows_needs_a_caller_config() {
    let text = "kind,now_us,a,b\nchunk,10,188,\n";
    assert_eq!(InputLog::parse(text).unwrap_err().reason, Reason::NoConfig);
    let log = InputLog::parse_with_config(text, custom_config()).unwrap();
    assert_eq!(log.config, custom_config());
    assert_eq!(log.inputs.len(), 1);
}

#[test]
fn bad_header_and_bad_config_are_rejected() {
    assert_eq!(
        InputLog::parse("nope\n").unwrap_err().reason,
        Reason::BadHeader
    );
    let unknown = "kind,now_us,a,b\nconfig,0,no.such,1\n";
    assert_eq!(
        InputLog::parse(unknown).unwrap_err().reason,
        Reason::UnknownConfigKey
    );
    let invalid = "kind,now_us,a,b\nconfig,0,policy.min_us,0\n";
    assert!(matches!(
        InputLog::parse_with_config(invalid, ControllerConfig::default())
            .unwrap_err()
            .reason,
        Reason::InvalidConfig(_)
    ));
}

#[test]
fn replay_matches_driving_a_controller_directly() {
    let cfg = custom_config();
    let mut live = BufferController::new(cfg, None).unwrap();
    let mut inputs = Vec::new();
    let mut expected = Vec::new();
    let mut at = 0u64;
    let mut media = 0u64;
    live.register_track(1);
    inputs.push(Input::Register {
        now_us: us(0),
        track: 1,
    });
    for k in 0..400u64 {
        at += 20_000;
        media += 20_000 * 27;
        let step = [
            Input::Chunk {
                now_us: us(at),
                bytes: 1316,
                pcr: Some(t(media)),
            },
            Input::TrackMedia {
                now_us: us(at),
                track: 1,
                end: t(media),
            },
            Input::Tick {
                now_us: us(at),
                pts: (k > 60).then_some(t(media.saturating_sub(300_000 * 27))),
            },
        ];
        for i in step {
            i.apply(&mut live);
            while let Some(e) = live.poll_event() {
                expected.push((i.now_us(), e));
            }
            inputs.push(i);
        }
    }
    assert!(
        expected
            .iter()
            .any(|(_, e)| matches!(e, Event::StartPlaying { .. })),
        "the drive must reach a decision or the comparison is vacuous"
    );
    let mut text = String::new();
    InputLog::new(cfg, None, inputs).write(&mut text).unwrap();
    assert_eq!(InputLog::parse(&text).unwrap().replay().unwrap(), expected);
}
