//! The post-fix server path: a 1.0-1.5 s delivery stall followed by a
//! line-rate burst of the backlog (10x here: a 1.4 s backlog drains in about
//! 140 ms) that returns the lead to its pre-gap value. Two stalls, 20 s apart.
//! The real traces (`tests/real_traces.rs`) are the old no-burst server and
//! stay as they are; a real post-fix trace is to be added by zenith-tv.

mod support;

use live_buffer::{Event, State};
use support::*;

const MAX: u64 = 3_000_000;

fn opts(initial: u64) -> Opts {
    let mut o = Opts::new(initial);
    o.send_hints = false;
    o
}

fn burst_stream() -> Vec<Chunk> {
    let mut s = Synth::new(60 * SEC);
    s.gaps = vec![(8 * SEC, 1_400_000), (28 * SEC, 1_400_000)];
    s.catch_up = true;
    s.generate()
}

/// Run, also returning the minimum Playing lead seen inside each gap window.
fn windows_min_lead(chunks: &[Chunk], o: &Opts) -> (Run, [Option<i64>; 2]) {
    let mut s = Session::new(o);
    let mut out = [None, None];
    let mut armed = [false, false];
    for ch in chunks {
        let t = ch.arrival_us;
        for (i, start) in [8 * SEC, 28 * SEC].into_iter().enumerate() {
            if !armed[i] && t >= start {
                s.min_lead = None;
                armed[i] = true;
            }
            if out[i].is_none() && t >= start + 4 * SEC {
                out[i] = s.min_lead;
            }
        }
        s.feed(ch);
    }
    (s.finish(), out)
}

#[test]
fn burst_with_the_default_initial_has_no_freeze_and_restores_the_lead() {
    let chunks = burst_stream();
    let (r, w) = windows_min_lead(&chunks, &opts(1_800_000));
    // (a) no freeze: the lead dips to ~initial - gap and stays above the starve threshold
    assert!(r.pauses().is_empty(), "{:?}", r.pauses());
    let (m1, m2) = (w[0].unwrap(), w[1].unwrap());
    assert!((300_000..=500_000).contains(&m1), "gap 1 min lead {m1}");
    // (d) the repeated gap 20 s later behaves identically (the burst restored the lead)
    assert!((m1 - m2).abs() <= 60_000, "gap 1 {m1} vs gap 2 {m2}");
    // the burst returns the lead to its pre-gap value
    let lead_end = r.ctl.stats().lead_us;
    assert!(
        (1_700_000..=1_900_000).contains(&lead_end),
        "lead after the bursts {lead_end}"
    );
    // (b) the stall memory records the gap depth; the burst's near-zero PDV is not baseline
    let st = r.ctl.stats();
    assert!(
        st.est.stall_peak_us >= 1_300_000,
        "peak {}",
        st.est.stall_peak_us
    );
    assert!(
        st.est.baseline_us <= 50_000,
        "baseline {}",
        st.est.baseline_us
    );
    // (c) the target rose once (k2 x depth), held, and did not ratchet on the repeat
    let max_t = *r.targets.iter().max().unwrap();
    assert!(
        (1_600_000..=1_800_000).contains(&max_t),
        "max target {max_t}"
    );
    let n = r.targets.len();
    assert!(
        r.targets[n * 45 / 60] >= 1_600_000,
        "still held 17 s after the second gap"
    );
    assert_eq!(r.ctl.stats().stalls, 0);
}

#[test]
fn burst_with_a_short_initial_freezes_once_for_a_bounded_time() {
    let chunks = burst_stream();
    for initial in [500_000u64, 1_000_000] {
        let (r, w) = windows_min_lead(&chunks, &opts(initial));
        let p = r.pauses();
        assert_eq!(
            p.len(),
            1,
            "initial {initial}: only the first gap stalls: {p:?}"
        );
        let (a, b) = (p[0].0, p[0].1.expect("resumed"));
        let freeze = b - a;
        let gap = 1_400_000u64;
        // starved from `initial` into the gap until the burst: gap - initial. After the burst the
        // lead is `gap` (the backlog) and the resume bar is 1.5 x depth: 0.5 x gap more real time.
        let bound = (gap - initial) + gap / 2 + 250_000;
        assert!(
            freeze <= bound,
            "initial {initial}: freeze {freeze} > bound {bound}"
        );
        assert!(
            freeze >= gap - initial,
            "cannot resume before the data is back"
        );
        // the repeat 20 s later is survived by the post-resume lead: no second pause, ever
        assert_eq!(r.ctl.stats().stalls, 1);
        assert!(w[1].unwrap() > 20_000, "second gap min lead {:?}", w[1]);
        assert_eq!(r.ctl.state(), State::Playing);
        for (_, e) in &r.events {
            if let Event::Resume { lead_us, .. } = e {
                assert!(*lead_us <= MAX + 40_000);
            }
        }
        let st = r.ctl.stats();
        assert!(st.est.baseline_us <= 50_000 && st.est.stall_peak_us >= 1_300_000);
    }
}
