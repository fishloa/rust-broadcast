//! MPD refresh, update rules, Reset, dynamic-to-static and Period changes of
//! the live client through the `World` simulation (DESIGN 5.2, 5.5).

use crate::common;

use common::checks::*;
use common::scenario::*;
use common::synth::{Addressing, Synth};
use common::world::{Kind, MICROS, Sim};
use dash_runtime::client::{DashConfig, Notice, Output};

const START: i64 = 41 * MICROS + 120_000;

fn mpd_times(sim: &Sim) -> Vec<f64> {
    sim.requests(Kind::Mpd)
        .iter()
        .map(|l| l.send_mono_us as f64 / 1e6)
        .collect()
}

fn gaps(v: &[f64]) -> Vec<f64> {
    v.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn minimum_update_period_zero_is_floored_at_one_second() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.mup_us = Some(0);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 30);
    let g = gaps(&mpd_times(&sim));
    assert!(!g.is_empty());
    assert!(g.iter().all(|d| *d >= 0.99), "{g:?}");
    assert!(g.iter().all(|d| *d <= 1.2), "{g:?}");
}

#[test]
fn the_refresh_interval_is_the_minimum_update_period_from_the_exchange_midpoint() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(4 * MICROS);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 40);
    let g = gaps(&mpd_times(&sim));
    assert!(g.len() >= 8);
    // sent at t, answered at t + rtt: the next request goes at (t + rtt / 2) + MUP.
    assert!(g.iter().all(|d| (d - 4.0).abs() < 0.03), "{g:?}");
}

#[test]
fn no_minimum_update_period_means_no_refresh() {
    let mut synth = with_utc(Addressing::Number);
    synth.min_update_attr = false;
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 60);
    assert_eq!(sim.requests(Kind::Mpd).len(), 1);
}

#[test]
fn a_timeline_that_runs_out_before_the_next_refresh_forces_one() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.mup_us = Some(500 * MICROS);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 60);
    let g = gaps(&mpd_times(&sim));
    assert!(g.len() >= 15, "{g:?}");
    assert!(g.iter().all(|d| *d >= 0.99 && *d <= 3.2), "{g:?}");
}

#[test]
fn an_unchanged_mpd_answers_304_and_the_conditional_headers_are_sent() {
    let mut synth = with_utc(Addressing::Number);
    synth.send_etag = true;
    let fixed = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.mpd_at(AST_US + START)
    };
    synth.mpd_override = Some(Box::new(move |_| Some(fixed.clone())));
    let (mut sim, geo, handle) = sim_shared(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 30);
    assert!(handle.borrow().if_none_match_hits >= 10);
    assert_eq!(sim.errors.len(), 0);
}

#[test]
fn a_byte_identical_mpd_without_validators_is_detected_and_not_replanned() {
    let mut synth = with_utc(Addressing::Number);
    let fixed = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.mpd_at(AST_US + START)
    };
    synth.mpd_override = Some(Box::new(move |_| Some(fixed.clone())));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 30);
    assert!(sim.requests(Kind::Mpd).len() >= 10);
    assert_eq!(sim.count(|o| matches!(o, Output::Discontinuity)), 0);
}

#[test]
fn a_stale_publish_time_is_ignored_and_retried_at_the_floor() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(10 * MICROS);
    let stale_text = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.mup_us = Some(10 * MICROS);
        probe.window = 1;
        probe.mpd_at(AST_US + 20 * MICROS)
    };
    let mut calls = 0u32;
    synth.mpd_override = Some(Box::new(move |_| {
        calls += 1;
        (calls == 2).then(|| stale_text.clone())
    }));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 40);
    let t = mpd_times(&sim);
    let g = gaps(&t);
    assert!(g[0] >= 9.9, "natural interval first: {g:?}");
    assert!(
        (g[1] - 1.0).abs() < 0.1,
        "the stale answer is retried after the 1 s floor: {g:?}"
    );
}

#[test]
fn a_location_element_replaces_the_mpd_update_url() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.location = Some("http://dash.test/live/updates.mpd".into());
    let (mut sim, geo, handle) = sim_shared(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 20);
    let urls = handle.borrow().mpd_urls.clone();
    assert!(urls[0].ends_with("/manifest.mpd"));
    assert!(
        urls[1..].iter().all(|u| u.ends_with("/updates.mpd")),
        "{urls:?}"
    );
}

#[test]
fn a_redirect_target_becomes_the_update_url_when_there_is_no_location() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.redirect_to = Some("http://dash.test/live/moved.mpd".into());
    let (mut sim, geo, handle) = sim_shared(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 20);
    let urls = handle.borrow().mpd_urls.clone();
    assert!(urls[0].ends_with("/manifest.mpd"));
    assert!(
        urls[1..].iter().all(|u| u.ends_with("/moved.mpd")),
        "{urls:?}"
    );
}

#[test]
fn location_wins_over_a_redirect() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.location = Some("http://dash.test/live/updates.mpd".into());
    synth.redirect_to = Some("http://dash.test/live/moved.mpd".into());
    let (mut sim, geo, handle) = sim_shared(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 20);
    let urls = handle.borrow().mpd_urls.clone();
    assert!(
        urls[1..].iter().all(|u| u.ends_with("/updates.mpd")),
        "{urls:?}"
    );
}

#[test]
fn dynamic_to_static_plays_the_rest_and_ends() {
    for a in [Addressing::Number, Addressing::TimelineNumber] {
        let mut synth = with_utc(a);
        synth.static_from_utc_us = Some(AST_US + 60 * MICROS);
        let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
        sim.run_for(120 * MICROS as u64);
        no_fatal(&sim);
        assert!(sim.client.presentation_ended(), "{a:?}");
        assert_eq!(sim.count(|o| matches!(o, Output::EndOfStream)), 1);
        let reqs = segment_requests(&sim, geo);
        assert_contiguous(&reqs);
        assert_no_duplicates(&reqs);
        let last_v = reqs
            .iter()
            .filter(|r| r.rep == "V")
            .map(|r| r.idx)
            .max()
            .unwrap();
        assert_eq!(
            last_v, 29,
            "{a:?}: the 30 segments of the 60 s presentation"
        );
        let after_end = sim
            .requests(Kind::Mpd)
            .iter()
            .filter(|l| l.send_utc_us > AST_US + 75 * MICROS)
            .count();
        assert_eq!(after_end, 0, "no refresh after the presentation ended");
    }
}

#[test]
fn an_availability_start_change_is_an_mpd_reset_and_rejoins_the_live_edge() {
    let (mut sim, geo, handle) =
        sim_shared(with_utc(Addressing::Number), START, DashConfig::default());
    sim.run_for(30 * MICROS as u64);
    let new_ast = AST_US + 50 * MICROS;
    handle.borrow_mut().ast_us = new_ast;
    let changed_at = sim.world.true_utc();
    let before_reset: Vec<_> = sim.seen_ids.clone();
    sim.run_for(40 * MICROS as u64);
    assert!(
        before_reset
            .iter()
            .filter(|id| matches!(id, dash_runtime::client::ResourceId::Segment { .. }))
            .all(|id| !sim.client.awaits(*id)),
        "the requests of the abandoned timeline are no longer awaited"
    );
    no_fatal(&sim);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
    let old = segment_requests(&sim, geo);
    let new_geo = handle.borrow().geo();
    let last = old.iter().rev().find(|r| r.rep == "V").unwrap();
    assert!(
        last.send_utc_us > new_ast,
        "the stream carried on after the reset"
    );
    let post: Vec<_> = segment_requests(&sim, new_geo)
        .into_iter()
        .filter(|r| r.send_utc_us > changed_at + 3 * MICROS && r.rep == "V")
        .collect();
    assert!(post.len() >= 3);
    assert!(early_requests(&post, new_geo, TOLERANCE_US).is_empty());
}

#[test]
fn the_reset_descriptor_restarts_the_presentation_even_with_the_same_start_time() {
    let (mut sim, geo, handle) =
        sim_shared(with_utc(Addressing::Number), START, DashConfig::default());
    sim.run_for(20 * MICROS as u64);
    handle.borrow_mut().extra_mpd_children =
        "<SupplementalProperty schemeIdUri=\"urn:mpeg:dash:reset:2016\"/>".into();
    sim.run_for(20 * MICROS as u64);
    no_fatal(&sim);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
    let _ = geo;
}

#[test]
fn an_mpd_id_change_is_a_reset() {
    let mut synth = with_utc(Addressing::Number);
    synth.mpd_id = Some("one".into());
    let (mut sim, _geo, handle) = sim_shared(synth, START, DashConfig::default());
    sim.run_for(20 * MICROS as u64);
    handle.borrow_mut().mpd_id = Some("two".into());
    sim.run_for(20 * MICROS as u64);
    no_fatal(&sim);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
}

#[test]
fn per_segment_timeline_entries_and_r_growth_play_the_same() {
    for per in [false, true] {
        let mut synth = with_utc(Addressing::TimelineNumber);
        synth.per_segment_s = per;
        let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
        run_checked(&mut sim, geo, 30);
    }
}

#[test]
fn a_low_latency_mpd_plays_at_segment_granularity_with_a_notice() {
    let mut synth = with_utc(Addressing::Number);
    let probe = {
        let mut p = Synth::new(AST_US, Addressing::Number);
        p.utc_elements = synth.utc_elements.clone();
        p.mpd_at(AST_US + START)
    };
    let ll = probe.replace(
        "<SegmentTemplate ",
        "<SegmentTemplate availabilityTimeOffset=\"1.5\" availabilityTimeComplete=\"false\" ",
    );
    synth.mpd_override = Some(Box::new(move |_| Some(ll.clone())));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    let reqs = run_checked(&mut sim, geo, 20);
    assert!(
        early_requests(&reqs, geo, TOLERANCE_US).is_empty(),
        "complete-segment time"
    );
    assert!(sim.count(|o| matches!(o, Output::Notice(Notice::LowLatencyIgnored))) == 1);
}

#[test]
fn use_availability_time_offset_requests_early_by_the_offset() {
    let mut synth = with_utc(Addressing::Number);
    synth.early_status = 425;
    let probe = {
        let mut p = Synth::new(AST_US, Addressing::Number);
        p.utc_elements = synth.utc_elements.clone();
        p.mpd_at(AST_US + START)
    };
    let ll = probe.replace(
        "<SegmentTemplate ",
        "<SegmentTemplate availabilityTimeOffset=\"1.5\" ",
    );
    synth.mpd_override = Some(Box::new(move |_| Some(ll.clone())));
    let mut dash = DashConfig::default();
    dash.use_availability_time_offset = true;
    let (mut sim, geo) = sim_with(synth, START, dash);
    sim.run_for(30 * MICROS as u64);
    let reqs = segment_requests(&sim, geo);
    let early: Vec<_> = early_requests(&reqs, geo, TOLERANCE_US);
    assert!(!early.is_empty());
    assert!(early_requests(&reqs, geo, 1_500_000 + TOLERANCE_US).is_empty());
}

#[test]
fn a_reset_descriptor_in_an_unchanged_mpd_resets_nothing() {
    let mut synth = with_utc(Addressing::Number);
    synth.extra_mpd_children =
        "<SupplementalProperty schemeIdUri=\"urn:mpeg:dash:reset:2016\"/>".into();
    let fixed = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.extra_mpd_children = synth.extra_mpd_children.clone();
        probe.mpd_at(AST_US + START)
    };
    synth.mpd_override = Some(Box::new(move |_| Some(fixed.clone())));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 30);
    assert!(sim.requests(Kind::Mpd).len() >= 10);
    assert_eq!(
        sim.count(|o| matches!(o, Output::Discontinuity)),
        0,
        "the same MPD fetched again is not a new reset"
    );
}

#[test]
fn a_trailing_negative_repeat_runs_to_the_fetch_time_plus_the_update_period() {
    let mut synth = with_utc(Addressing::TimelineNumber);
    synth.negative_r = true;
    synth.mup_us = Some(10 * MICROS);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 40);
    assert!(
        sim.requests(Kind::Mpd).len() <= 6,
        "the plan runs ahead to FetchTime + MUP, so the 10 s refresh interval is enough"
    );
    let per_track = sim.samples_per_track();
    assert!(per_track.values().all(|n| *n > 150), "{per_track:?}");
}

#[test]
fn a_304_nobody_asked_for_is_an_error() {
    let mut c = dash_runtime::client::DashClient::new(
        common::synth::MPD_URL,
        DashConfig::default(),
        dash_runtime::client::PullLimits::default(),
    );
    let meta = dash_runtime::client::ResponseMeta::at(0);
    assert!(c.on_mpd_unchanged(0, &meta).is_err());
}

#[test]
fn a_utc_step_found_by_a_resync_forces_an_mpd_refresh() {
    let mut dash = DashConfig::default();
    dash.utc_resync_us = 20 * MICROS as u64;
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(30 * MICROS);
    let (mut sim, _geo, handle) = sim_shared(synth, START, dash);
    sim.run_for(10 * MICROS as u64);
    handle.borrow_mut().utc.skew_us = 30 * MICROS;
    sim.run_for(30 * MICROS as u64);
    let utc = sim.requests(Kind::Utc);
    let second = utc[1].send_mono_us;
    let mpd_after = sim
        .requests(Kind::Mpd)
        .iter()
        .any(|l| l.send_mono_us > second && l.send_mono_us <= second + 1_500_000);
    assert!(mpd_after, "refresh right after the step");
}

#[test]
fn a_media_presentation_duration_ends_a_dynamic_presentation() {
    let mut synth = with_utc(Addressing::Number);
    let text = {
        let mut probe = Synth::new(AST_US, Addressing::Number);
        probe.utc_elements = synth.utc_elements.clone();
        probe.mpd_at(AST_US + START).replacen(
            "type=\"dynamic\"",
            "type=\"dynamic\" mediaPresentationDuration=\"PT50S\"",
            1,
        )
    };
    synth.mpd_override = Some(Box::new(move |_| Some(text.clone())));
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(60 * MICROS as u64);
    assert!(sim.client.presentation_ended(), "{:?}", sim.fatal());
}

#[test]
fn m2_reset_presentation_drops_cached_init_bytes() {
    use common::world::{HttpResp, Kind, Outcome};
    use std::cell::Cell;
    use std::rc::Rc;
    let mut synth = with_utc(Addressing::Number);
    synth.with_audio = false;
    synth.mpd_id = Some("one".into());
    let (mut sim, _geo, handle) = sim_shared(synth, START, DashConfig::default());
    let n = Rc::new(Cell::new(0u32));
    let nc = n.clone();
    sim.fault = Some(Box::new(move |_req, kind, _| {
        if *kind == Kind::Init {
            let i = nc.get();
            nc.set(i + 1);
            let bytes = if i == 0 {
                test_fixtures::read("dash/live-number/init-0.m4s")
            } else {
                test_fixtures::read("dash/live-number/init-1.m4s")
            };
            return Some(Outcome::Resp(HttpResp::ok(bytes)));
        }
        None
    }));
    sim.run_for(20 * MICROS as u64);
    handle.borrow_mut().mpd_id = Some("two".into());
    sim.run_for(20 * MICROS as u64);
    no_fatal(&sim);
    let inits: Vec<&Vec<u8>> = sim
        .outputs
        .iter()
        .filter_map(|o| match o {
            Output::Init(b) => Some(b),
            _ => None,
        })
        .collect();
    assert!(inits.len() >= 2, "{:?}", inits.len());
    let a = test_fixtures::read("dash/live-number/init-0.m4s");
    let b = test_fixtures::read("dash/live-number/init-1.m4s");
    assert_eq!(inits[0], &a);
    assert_eq!(inits[inits.len() - 1], &b);
}

#[test]
fn m3b_failed_replan_keeps_old_plan_and_lane_alive() {
    use std::cell::Cell;
    use std::rc::Rc;
    let mut synth = with_utc(Addressing::TimelineNumber);
    let good = {
        let mut p = Synth::new(AST_US, Addressing::TimelineNumber);
        p.utc_elements = synth.utc_elements.clone();
        p.mpd_at(AST_US + START)
    };
    let bad = good.replace(
        "r=\"7\"/></SegmentTimeline>",
        "r=\"7\"/><S t=\"0\" d=\"90000\"/></SegmentTimeline>",
    );
    let calls = Rc::new(Cell::new(0u32));
    let cc = calls.clone();
    synth.mpd_override = Some(Box::new(move |_| {
        let c = cc.get();
        cc.set(c + 1);
        if c == 1 {
            Some(bad.clone())
        } else {
            Some(good.clone())
        }
    }));
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(30 * MICROS as u64);
    no_fatal(&sim);
    assert_eq!(sim.count(|o| matches!(o, Output::TrackDropped { .. })), 0);
    assert!(sim.samples_per_track().values().any(|n| *n > 0));
}

#[test]
fn l3_slow_client_forward_jump_emits_discontinuity() {
    use std::cell::Cell;
    use std::rc::Rc;
    let mut synth = with_utc(Addressing::TimelineNumber);
    let good = {
        let mut p = Synth::new(AST_US, Addressing::TimelineNumber);
        p.utc_elements = synth.utc_elements.clone();
        p.mpd_at(AST_US + START)
    };
    let shifted = good
        .replace("startNumber=\"13\"", "startNumber=\"113\"")
        .replace(
            "<S t=\"2160000\" d=\"180000\" r=\"7\"/>",
            "<S t=\"21600000\" d=\"180000\" r=\"7\"/>",
        )
        .replace(
            "<S t=\"1058400\" d=\"88200\" r=\"7\"/>",
            "<S t=\"10584000\" d=\"88200\" r=\"7\"/>",
        );
    let calls = Rc::new(Cell::new(0u32));
    let cc = calls.clone();
    let good_c = good.clone();
    let shifted_c = shifted.clone();
    synth.mpd_override = Some(Box::new(move |_| {
        let c = cc.get();
        cc.set(c + 1);
        if c >= 5 {
            Some(shifted_c.clone())
        } else {
            Some(good_c.clone())
        }
    }));
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(30 * MICROS as u64);
    no_fatal(&sim);
    assert!(sim.count(|o| matches!(o, Output::Discontinuity)) >= 1);
}
