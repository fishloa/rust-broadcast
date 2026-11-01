//! Synthetic delivery scenarios (deterministic, seeded; they run everywhere,
//! no private fixtures). DESIGN.md section 5.
//!
//! The 1x/2x/4x stall pattern is the shape of a transport that retransmits
//! with exponential backoff (RFC 6298); the real ~1.4 s stalls have an
//! undetermined cause, the controller must survive them from any cause.
use crate::support::*;
use buffering_runtime::{
    BufferController, ControllerConfig, Event, Hint, Reason, State, WRAP_27MHZ,
};

const MIN: u64 = 300_000;
const MAX: u64 = 3_000_000;

fn opts(initial: u64) -> Opts {
    let mut o = Opts::new(initial);
    // a 2 s / 4 s silence here is a stall, not a reconnect: send no hints
    o.send_hints = false;
    o
}

fn run(s: &Synth, o: &Opts) -> Run {
    replay(&s.generate(), o)
}

#[test]
fn single_stalls_of_1_2_and_4_seconds() {
    for (gap, bar) in [
        (GAP_1S, 1_250_000u64),
        (GAP_2S, 2_500_000),
        (GAP_4S, 3_000_000),
    ] {
        let mut s = Synth::new(60 * SEC);
        s.gaps = vec![(10 * SEC, gap)];
        let r = run(&s, &opts(500_000));
        let p = r.pauses();
        assert_eq!(p.len(), 1, "gap {gap}: {p:?}");
        let reason = r.events.iter().find_map(|(_, e)| match e {
            Event::Pause { reason, .. } => Some(*reason),
            _ => None,
        });
        assert_eq!(reason, Some(Reason::NoData));
        // the stall is only learned from the first post-gap chunk: resume lead is the
        // 1.25 x depth bar, clamped to the cap
        let lead = r.events.iter().find_map(|(_, e)| match e {
            Event::Resume { lead_us, .. } => Some(*lead_us),
            _ => None,
        });
        let lead = lead.unwrap();
        assert!(
            lead >= bar - 50_000 && lead <= bar + 80_000,
            "gap {gap}: resume lead {lead} vs bar {bar}"
        );
        assert!(r.targets.iter().all(|&t| (MIN..=MAX).contains(&t)));
        let over = r.count(|e| matches!(e, Event::OverMax { .. }));
        assert_eq!(
            over,
            usize::from(gap == GAP_4S),
            "OverMax only when 1.2 x gap exceeds the cap"
        );
        assert_eq!(r.ctl.state(), State::Playing);
    }
}

#[test]
fn consecutive_backoff_stalls_1_2_4_seconds() {
    let mut s = Synth::new(90 * SEC);
    s.gaps = vec![(10 * SEC, GAP_1S), (25 * SEC, GAP_2S), (45 * SEC, GAP_4S)];
    let r = run(&s, &opts(500_000));
    let p = r.pauses();
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(p.iter().all(|x| x.1.is_some()));
    assert_eq!(r.ctl.stats().stalls, 3);
    assert_eq!(r.ctl.state(), State::Playing);
    assert!(r.targets.iter().all(|&t| t <= MAX));
    // the freezes grow with the gap, bounded by the cap-limited refill
    let f: Vec<u64> = p.iter().map(|&(a, b)| b.unwrap() - a).collect();
    assert!(f[0] < f[1] && f[1] < f[2], "{f:?}");
}

#[test]
fn periodic_stalls_every_minute_keep_the_memory_alive() {
    let mut s = Synth::new(20 * 60 * SEC);
    s.gaps = (1..20).map(|k| (k * 60 * SEC, GAP_1S)).collect();
    let r = run(&s, &opts(1_800_000));
    // after the first stall the 1.2 x 1 s memory is refreshed every minute: the
    // target never falls back to the floor
    let n = r.targets.len();
    let after_first = &r.targets[n / 19..];
    assert!(
        *after_first.iter().min().unwrap() >= 1_100_000,
        "min {}",
        after_first.iter().min().unwrap()
    );
    assert!(r.targets.iter().all(|&t| t <= MAX));
}

#[test]
fn periodic_stalls_every_five_minutes_decay_between_and_do_not_ratchet() {
    let mut s = Synth::new(30 * 60 * SEC);
    s.gaps = (1..6).map(|k| (k * 300 * SEC, 1_430_000)).collect();
    let r = run(&s, &opts(1_800_000));
    // just before each following gap the memory has held (60 s), decayed (60 s) and the
    // slew-limited target has fallen back to the floor
    let at = |t_s: u64| {
        let i = r.targets.len() * t_s as usize / 1800;
        r.targets[i]
    };
    for k in 2..6u64 {
        assert_eq!(at(k * 300 - 10), MIN, "before gap {k}");
    }
    // no ratchet: the target never exceeds what one 1.43 s stall implies, and every
    // post-stall resume asks for the same ~1.25 x depth
    assert!(r.targets.iter().max().unwrap() <= &1_800_000);
    let leads: Vec<u64> = r
        .events
        .iter()
        .filter_map(|(_, e)| match e {
            Event::Resume { lead_us, .. } => Some(*lead_us),
            _ => None,
        })
        .collect();
    assert!(!leads.is_empty());
    let (lo, hi) = (leads.iter().min().unwrap(), leads.iter().max().unwrap());
    assert!(
        hi - lo < 150_000 && *hi < 2_400_000,
        "resume leads drift: {leads:?}"
    );
    // the first gap is survived at initial 1.8 s, the later ones stall: covered, one pause each
    assert_eq!(r.pauses().len(), leads.len());
}

#[test]
fn one_outlier_does_not_ratchet_the_target() {
    let mut s = Synth::new(300 * SEC);
    s.gaps = vec![(10 * SEC, 6_000_000)];
    let r = run(&s, &opts(500_000));
    assert_eq!(
        r.targets.iter().max(),
        Some(&MAX),
        "a 6 s stall is capped at max_us"
    );
    let n = r.targets.len();
    // hold 60 s + decay 60 s later it is gone (target at 200 s of 300 s)
    assert_eq!(r.targets[n * 200 / 300], MIN);
    assert_eq!(*r.targets.last().unwrap(), MIN);
}

#[test]
fn ramping_jitter_raises_the_target_through_k1_and_the_margin() {
    let mut s = Synth::new(180 * SEC);
    s.jitter_us = 1_000;
    s.ramp_to_us = Some(300_000);
    let r = run(&s, &opts(1_800_000));
    let last = *r.targets.last().unwrap();
    assert!((600_000..=1_200_000).contains(&last), "final target {last}");
    assert!(r.pauses().is_empty());
    assert_eq!(r.count(|e| matches!(e, Event::OverMax { .. })), 0);
    // early: floor; later higher (the ramp is felt)
    assert_eq!(r.targets[10], MIN);
    assert!(last > r.targets[r.targets.len() / 2]);
}

#[test]
fn clock_drift_of_100_ppm_over_30_minutes_does_not_creep_the_target() {
    for ppm in [100i64, -100] {
        let mut s = Synth::new(30 * 60 * SEC);
        s.drift_ppm = ppm;
        let r = run(&s, &opts(1_800_000));
        assert!(
            r.targets.iter().all(|&t| t == MIN),
            "ppm {ppm}: target crept to {:?}",
            r.targets.iter().max()
        );
        let st = r.ctl.stats();
        assert!(
            st.est.baseline_us <= 40_000,
            "ppm {ppm}: baseline {}",
            st.est.baseline_us
        );
        assert!(
            st.est.stall_peak_us <= 40_000,
            "ppm {ppm}: stall peak {}",
            st.est.stall_peak_us
        );
        assert!(r.pauses().is_empty(), "ppm {ppm}");
    }
}

#[test]
fn pcr_wrap_is_invisible() {
    let mut s = Synth::new(40 * SEC);
    s.gaps = vec![(15 * SEC, 1_400_000)];
    let base = run(&s, &opts(500_000));
    let mut w = s.clone();
    w.pcr0 = WRAP_27MHZ - 20 * 27_000_000; // the stream crosses the 2^33 x 300 boundary at 20 s
    let r = run(&w, &opts(500_000));
    assert_eq!(r.count(|e| matches!(e, Event::Rebaselined)), 0);
    let times = |x: &Run| {
        x.events
            .iter()
            .map(|(t, e)| (*t, e.name()))
            .collect::<Vec<_>>()
    };
    assert_eq!(times(&r), times(&base));
    assert_eq!(r.targets, base.targets);
}

#[test]
fn pcr_discontinuities_rebaseline_keep_the_memory_and_recover() {
    for jump_us in [5_000_000i64, -5_000_000] {
        let mut s = Synth::new(60 * SEC);
        s.gaps = vec![(8 * SEC, 1_400_000)];
        s.pcr_jump = Some((25 * SEC, jump_us));
        let r = run(&s, &opts(1_800_000));
        assert_eq!(
            r.count(|e| matches!(e, Event::Rebaselined)),
            1,
            "jump {jump_us}"
        );
        let st = r.ctl.stats();
        assert_eq!(st.discontinuities, 1);
        assert!(
            st.est.stall_peak_us >= 1_300_000,
            "stall memory kept across the re-baseline"
        );
        assert_eq!(st.state, State::Playing);
        let has = r.events.iter().any(|(_, e)| {
            matches!(
                e,
                Event::Pause {
                    reason: Reason::Discontinuity,
                    ..
                }
            )
        });
        assert!(has, "Pause(Discontinuity) at the jump");
    }
}

#[test]
fn catch_up_burst_does_not_inflate_the_baseline() {
    let mut s = Synth::new(40 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000)];
    s.catch_up = true;
    let r = run(&s, &opts(1_800_000));
    let st = r.ctl.stats();
    assert!(
        st.est.baseline_us <= 50_000,
        "burst-aware max: baseline {}",
        st.est.baseline_us
    );
    assert!(
        st.est.stall_peak_us >= 1_000_000,
        "the stall itself is remembered: {}",
        st.est.stall_peak_us
    );
}

#[test]
fn freeze_budget_is_conserved_for_every_policy_preset() {
    // 1.43 s no-catch-up gap every 180 s for 30 min: total frozen time stays within 20% of
    // gaps x 1.43 s whatever the preset (a ratcheting or hovering controller would not)
    let mut s = Synth::new(30 * 60 * SEC);
    s.gaps = (1..10).map(|k| (k * 180 * SEC, 1_430_000)).collect();
    let chunks = s.generate();
    let gaps = 9u64;
    let mut presets: Vec<(&str, Opts)> = vec![
        ("default", opts(1_800_000)),
        ("initial 500 ms", opts(500_000)),
    ];
    let mut p = opts(1_800_000);
    p.cfg.policy.after_rebuffer_q8 = 320; // 1.25
    presets.push(("after_rebuffer 1.25", p));
    let mut p = opts(500_000);
    p.cfg.policy.after_rebuffer_q8 = 256; // 1.0
    presets.push(("after_rebuffer 1.0", p));
    for (name, o) in presets {
        let r = replay(&chunks, &o);
        let frozen: u64 = r.pauses().iter().map(|&(a, b)| b.unwrap() - a).sum();
        let want = gaps * 1_430_000;
        assert!(
            frozen as f64 >= want as f64 * 0.8 && frozen as f64 <= want as f64 * 1.2,
            "{name}: frozen {frozen} us vs {want} us"
        );
    }
}

#[test]
fn model_a_free_running_clock_starves_forever_model_b_obeys_and_recovers() {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(10 * SEC, 1_500_000)];
    let chunks = s.generate();
    // Model A: the zenith-tv failure. Clock free-runs from the first StartPlaying and
    // ignores Pause/Resume. Its own lead (media end - clock) never recovers after the gap.
    let mut c = BufferController::new(opts(500_000).cfg, None).unwrap();
    let mut clock: Option<(u64, u64)> = None; // (position at start, start time)
    let mut leads_after_gap = vec![];
    let mut pauses_seen = 0;
    let mut next_tick = 0;
    for ch in &chunks {
        while next_tick <= ch.arrival_us {
            let pos = clock.map(|(p0, t0)| (p0 + (next_tick - t0) * TICKS_PER_US) % WRAP_27MHZ);
            c.on_tick(next_tick, pos);
            while let Some(e) = c.poll_event() {
                match e {
                    Event::StartPlaying { media_27mhz, .. } => {
                        clock = Some((media_27mhz, next_tick))
                    }
                    Event::Pause { .. } => pauses_seen += 1,
                    _ => {} // Resume IGNORED
                }
            }
            next_tick += 5_000;
        }
        let pcr = ch.pcr.unwrap();
        c.on_chunk(ch.arrival_us, ch.bytes, Some(pcr));
        c.on_media(ch.arrival_us, pcr);
        if let Some((p0, t0)) = clock
            && ch.arrival_us > 12 * SEC
        {
            let pos = p0 + (ch.arrival_us - t0) * TICKS_PER_US;
            let lead = (i128::from(pcr) - i128::from(pos % WRAP_27MHZ)) / 27;
            leads_after_gap.push(lead);
        }
    }
    assert!(pauses_seen >= 1, "the controller did ask for a pause");
    assert!(!leads_after_gap.is_empty());
    assert!(
        leads_after_gap.iter().all(|&l| l <= 0),
        "model A reproduces permanent starvation (lead <= 0 for the rest of the run)"
    );
    // Model B: obeys 2.4 (the shared replay renderer)
    let r = replay(&chunks, &opts(500_000));
    let p = r.pauses();
    assert_eq!(p.len(), 1);
    let resume = r.events.iter().find_map(|(_, e)| match e {
        Event::Resume { lead_us, .. } => Some(*lead_us),
        _ => None,
    });
    assert!(
        resume.unwrap() >= 1_850_000,
        "ends the gap with lead >= the resume target"
    );
    assert_eq!(r.ctl.stats().stalls, 1, "zero further Pause events");
    assert!(r.min_lead_playing.unwrap() > 20_000);
    // media queued while paused is never behind the resumed clock: Resume.media == hold_at
    let hold = r.events.iter().find_map(|(_, e)| match e {
        Event::Pause { hold_at_27mhz, .. } => Some(*hold_at_27mhz),
        _ => None,
    });
    let resume_media = r.events.iter().find_map(|(_, e)| match e {
        Event::Resume { media_27mhz, .. } => Some(*media_27mhz),
        _ => None,
    });
    assert_eq!(hold, resume_media);
}

#[test]
fn two_controllers_fed_interleaved_do_not_interact() {
    let mut gappy = Synth::new(40 * SEC);
    gappy.gaps = vec![(10 * SEC, 1_400_000)];
    let mut clean = Synth::new(40 * SEC);
    clean.seed = 777;
    let (a, b) = (gappy.generate(), clean.generate());
    let (ra, rb) = (replay(&a, &opts(500_000)), replay(&b, &opts(500_000)));
    assert_eq!(ra.pauses().len(), 1);
    assert!(rb.pauses().is_empty());
    let (mut sa, mut sb) = (Session::new(&opts(500_000)), Session::new(&opts(500_000)));
    for i in 0..a.len().max(b.len()) {
        if let Some(c) = a.get(i) {
            sa.feed(c);
        }
        if let Some(c) = b.get(i) {
            sb.feed(c);
        }
    }
    let (ia, ib) = (sa.finish(), sb.finish());
    assert_eq!(ia.events, ra.events);
    assert_eq!(ib.events, rb.events);
    assert_eq!(ia.targets, ra.targets);
    assert_eq!(ib.targets, rb.targets);
}

#[test]
fn two_stalls_in_one_trace_use_the_memory_of_the_first() {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000), (25 * SEC, 1_400_000)];
    let r = run(&s, &opts(500_000));
    let leads: Vec<u64> = r
        .events
        .iter()
        .filter_map(|(_, e)| match e {
            Event::Resume { lead_us, .. } => Some(*lead_us),
            _ => None,
        })
        .collect();
    // first stall: resume ~2.1 s; the second gap (1.4 s) is survived by that 2.1 s lead
    assert_eq!(leads.len(), 1, "{leads:?}");
    assert_eq!(r.ctl.stats().stalls, 1);
}

// ----------------------------------------------------- hostile input, no panics

#[test]
fn non_monotonic_clocks_and_u64_extremes_never_panic() {
    let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
    for k in 0..200u64 {
        c.on_chunk(1_000_000 + k * 35_000, 1316, Some(27_000 + k * 945_000));
        c.on_media(1_000_000 + k * 35_000, 27_000 + k * 945_000);
        c.on_tick(1_000_000 + k * 35_000, None);
    }
    let before = c.stats();
    // arrival clock regresses
    c.on_chunk(5, 1316, Some(1));
    c.on_chunk(1, 1316, Some(2));
    c.on_tick(3, Some(4));
    assert!(
        c.stats().clock_regressions >= 3,
        "{}",
        c.stats().clock_regressions
    );
    assert_eq!(c.stats().est.samples, before.est.samples);
    // extremes everywhere
    for v in [u64::MAX, u64::MAX - 1, 0, 1 << 63] {
        c.on_chunk(v, u32::MAX, Some(v));
        c.on_media(v, v);
        c.on_tick(v, Some(v));
        c.on_hint(v, Hint::Reconnected);
        c.on_hint(v, Hint::SyncLoss);
        while c.poll_event().is_some() {}
        assert!((MIN..=MAX).contains(&c.target_us()));
        let _ = c.stats();
    }
    let mut fresh = BufferController::new(ControllerConfig::default(), None).unwrap();
    fresh.on_chunk(u64::MAX, 0, Some(u64::MAX));
    fresh.on_media(u64::MAX, u64::MAX);
    fresh.on_tick(u64::MAX, Some(0));
    fresh.on_tick(0, Some(u64::MAX));
    assert!((MIN..=MAX).contains(&fresh.target_us()));
}

#[test]
fn pcr_less_mode_estimates_from_media_end_and_matches_pcr_mode() {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(20 * SEC, 1_400_000)];
    let chunks = s.generate();
    let o = opts(1_800_000);
    // PCR mode, media_end = PCR - offset
    let mut a = BufferController::new(o.cfg, None).unwrap();
    // PCR-less mode: no PCR ever, the media end feeds the estimator
    let mut b = BufferController::new(o.cfg, None).unwrap();
    for ch in &chunks {
        let pcr = ch.pcr.unwrap();
        let end = shift27(pcr, -(777 * 27_000));
        a.on_chunk(ch.arrival_us, ch.bytes, Some(pcr));
        a.on_media(ch.arrival_us, end);
        b.on_chunk(ch.arrival_us, ch.bytes, None);
        b.on_media(ch.arrival_us, end);
        a.on_tick(ch.arrival_us, None);
        b.on_tick(ch.arrival_us, None);
    }
    let (ea, eb) = (a.stats().est, b.stats().est);
    assert_eq!(ea.baseline_us, eb.baseline_us);
    assert_eq!(ea.stall_peak_us, eb.stall_peak_us);
    assert_eq!(ea.samples, eb.samples);
    assert_eq!(a.target_us(), b.target_us());
    assert!(ea.stall_peak_us >= 1_300_000);
}

#[test]
fn with_a_pcr_present_media_end_never_feeds_the_estimator() {
    let mut s = Synth::new(40 * SEC);
    let chunks = s.generate();
    s.seed = 1;
    let mut a = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut b = BufferController::new(ControllerConfig::default(), None).unwrap();
    let mut rng = Rng(3);
    for ch in &chunks {
        let pcr = ch.pcr.unwrap();
        a.on_chunk(ch.arrival_us, ch.bytes, Some(pcr));
        a.on_media(ch.arrival_us, shift27(pcr, -1));
        b.on_chunk(ch.arrival_us, ch.bytes, Some(pcr));
        // wildly wrong media ends: only the lead may change, never the estimate
        b.on_media(ch.arrival_us, rng.below(WRAP_27MHZ));
    }
    assert_eq!(a.stats().est, b.stats().est);
    assert_eq!(a.target_us(), b.target_us());
}

#[test]
fn first_pcr_after_pcr_less_samples_rebaselines_the_estimator_once() {
    let s = Synth::new(20 * SEC);
    let chunks = s.generate();
    let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
    for (i, ch) in chunks.iter().enumerate() {
        let pcr = ch.pcr.unwrap();
        // the PTS domain is offset by a huge constant so a mixed feed would read a giant PDV
        let end = shift27(pcr, -5_000_000_000);
        c.on_chunk(
            ch.arrival_us,
            ch.bytes,
            if i < 100 { None } else { Some(pcr) },
        );
        c.on_media(ch.arrival_us, end);
    }
    assert!(c.stats().est.stall_peak_us < 100_000);
    assert_eq!(c.stats().discontinuities, 0);
}
