//! Refresh and failure decisions the existing live suites leave open: a
//! representation removed by a refresh, the MPD failure count, the refresh
//! interval after a changed `minimumUpdatePeriod`, a 425 answered early and
//! the sampler-corrected UTC accessor.

use crate::common;

use broadcast_common::stage::Timestamp;
use common::synth::{Addressing, Synth};
use common::world::{HttpResp, Kind, MICROS, Outcome, Sim};
use dash_runtime::client::{DashConfig, FailureClass, Output};

use crate::common::checks::*;
use crate::common::scenario::*;

const START: i64 = 41 * MICROS + 120_000;

#[test]
fn a_refresh_that_removes_the_playing_representation_switches_to_the_survivor() {
    let mut synth = with_utc(Addressing::Number);
    synth.second_video = true;
    let mut survivor = with_utc(Addressing::Number);
    survivor.seg_us = synth.seg_us;
    survivor.start_number = synth.start_number;
    let switch_at = AST_US + START + 20 * MICROS;
    synth.mpd_override = Some(Box::new(move |at| {
        (at > switch_at).then(|| survivor.mpd_at(at))
    }));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 40);
    let after: Vec<_> = segment_requests(&sim, geo)
        .into_iter()
        .filter(|r| r.send_utc_us > switch_at + 2 * MICROS)
        .collect();
    assert!(!after.is_empty(), "playback continues after the refresh");
    assert!(
        after.iter().any(|r| r.rep == "V"),
        "the surviving representation is requested: {:?}",
        after.iter().map(|r| r.rep.clone()).collect::<Vec<_>>()
    );
    assert!(
        after.iter().all(|r| r.rep == "V" || r.rep == "A"),
        "the removed representation is never requested again: {:?}",
        after.iter().map(|r| r.rep.clone()).collect::<Vec<_>>()
    );
    let first_v = after
        .iter()
        .find(|r| r.rep == "V")
        .map(|r| r.idx)
        .expect("the survivor is requested");
    assert!(
        first_v > 0,
        "the survivor resumes at the switch instant, not from 0: {first_v}"
    );
    no_fatal(&sim);
}

#[test]
fn the_mpd_failure_count_is_the_number_of_consecutive_failures() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(2 * MICROS);
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    let mut hits = 0u32;
    sim.fault = Some(Box::new(move |_req, kind, _at| {
        (*kind == Kind::Mpd && hits < 2).then(|| {
            hits += 1;
            Outcome::Resp(HttpResp::status(500))
        })
    }));
    sim.run_for(20 * MICROS as u64);
    let attempts: Vec<u32> = sim
        .classes
        .iter()
        .filter_map(|c| match c {
            FailureClass::Retry { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert!(attempts.contains(&1), "{attempts:?}");
    assert!(
        attempts.contains(&2),
        "the second failure is attempt 2: {attempts:?}"
    );
}

#[test]
fn a_changed_minimum_update_period_changes_the_refresh_interval() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(2 * MICROS);
    let mut slower = with_utc(Addressing::Number);
    slower.mup_us = Some(8 * MICROS);
    slower.seg_us = synth.seg_us;
    slower.start_number = synth.start_number;
    let switch_at = AST_US + START + 20 * MICROS;
    synth.mpd_override = Some(Box::new(move |at| {
        (at > switch_at).then(|| slower.mpd_at(at))
    }));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 60);
    let times: Vec<i64> = sim
        .requests(Kind::Mpd)
        .iter()
        .map(|l| l.send_utc_us)
        .filter(|t| *t > switch_at + MICROS)
        .collect();
    assert!(times.len() >= 3, "{times:?}");
    for w in times.windows(2) {
        let gap = (w[1] - w[0]) as f64 / MICROS as f64;
        assert!(gap >= 7.5, "the new minimumUpdatePeriod is honoured: {gap}");
    }
}

#[test]
fn a_segment_answered_425_is_retried_without_counting_a_failure() {
    let synth = with_utc(Addressing::Number);
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    let mut hits = 0u32;
    sim.fault = Some(Box::new(move |req, kind, _at| {
        let first = *kind == Kind::Segment && req.url.contains("/V/") && hits == 0;
        if first {
            hits += 1;
        }
        first.then(|| Outcome::Resp(HttpResp::status(425)))
    }));
    run_checked(&mut sim, geo, 30);
    assert!(
        sim.classes
            .iter()
            .any(|c| matches!(c, FailureClass::NotYetAvailable { .. })),
        "the early answer is a NotYetAvailable: {:?}",
        sim.classes
    );
    assert!(
        !sim.classes.iter().any(|c| matches!(
            c,
            FailureClass::Retry {
                kind: dash_runtime::client::FailureKind::TooEarly,
                ..
            }
        )),
        "an early answer is not a counted failure: {:?}",
        sim.classes
    );
}

#[test]
fn utc_at_is_none_before_the_first_sample_and_then_the_sampler_correction() {
    let mut synth = with_utc(Addressing::Number);
    synth.utc.skew_us = 5 * MICROS;
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    let now = Timestamp::from_micros(sim.world.mono());
    assert_eq!(sim.client.utc_at(now), None, "no sample yet");
    sim.run_for(5 * MICROS as u64);
    let now = Timestamp::from_micros(sim.world.mono());
    let utc = sim.client.utc_at(now).expect("a sample is held");
    let host = sim.world.host_wall();
    let delta = utc.as_microsecond() - host.as_microsecond();
    assert!(
        (delta - 5 * MICROS).abs() < 2 * MICROS,
        "the sampler correction is applied: {delta}"
    );
}

fn two_period_synth() -> Synth {
    let mut synth = with_utc(Addressing::Number);
    synth.period_starts_us = vec![0, 30 * MICROS];
    synth
}

fn manifest_of(sim: &Sim, period: usize) -> dash_runtime::client::ManifestSelection {
    sim.outputs
        .iter()
        .filter_map(|o| match o {
            Output::Manifest(m) if m.period == period => Some(m.clone()),
            _ => None,
        })
        .next_back()
        .unwrap_or_else(|| panic!("no Manifest for Period {period}"))
}

#[test]
fn a_period_boundary_keeps_the_audio_rendition_chosen() {
    let (mut sim, _geo) = sim_with(two_period_synth(), START, DashConfig::default());
    sim.run_for(50 * MICROS as u64);
    no_fatal(&sim);
    let m = manifest_of(&sim, 1);
    assert_eq!(
        m.chosen_audio,
        Some(0),
        "the audio rendition is still the chosen one"
    );
    assert_eq!(
        m.variants[0].audio_group.as_deref(),
        Some("1"),
        "the audio group named on the video variant is the audio set"
    );
}

#[test]
fn a_period_without_ids_is_remapped_by_its_start() {
    let mut synth = with_utc(Addressing::Number);
    let mut probe = with_utc(Addressing::Number);
    synth.mpd_override = Some(Box::new(move |at| {
        Some(probe.mpd_at(at).replace(" id=\"P0\"", ""))
    }));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 30);
    assert_eq!(
        sim.count(|o| matches!(o, Output::Discontinuity)),
        0,
        "the same Period is matched by its start, not treated as new"
    );
}

#[test]
fn a_changed_period_id_resets_the_presentation() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(2 * MICROS);
    let mut probe = with_utc(Addressing::Number);
    probe.mup_us = Some(2 * MICROS);
    let switch_at = AST_US + START + 12 * MICROS;
    synth.mpd_override = Some(Box::new(move |at| {
        let text = probe.mpd_at(at);
        (at > switch_at).then(|| text.replace("id=\"P0\"", "id=\"Q0\""))
    }));
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(25 * MICROS as u64);
    assert!(
        sim.count(|o| matches!(o, Output::Discontinuity)) >= 1,
        "a new Period identity is a Reset"
    );
}

#[test]
fn a_candidate_rotation_forces_a_refresh_of_a_live_mpd() {
    let mut synth = with_utc(Addressing::Number);
    synth.mup_us = Some(30 * MICROS);
    synth.extra_mpd_children =
        "<BaseURL>http://dash.test/live-broken/</BaseURL><BaseURL>http://dash.test/live/</BaseURL>"
            .to_owned();
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 20);
    let mpd_requests = sim.requests(Kind::Mpd).len();
    assert!(
        mpd_requests >= 2,
        "a rotation after the initial MPD forces a refresh: {mpd_requests}"
    );
}

fn two_set_mpd(with_audio: bool) -> String {
    let audio = if with_audio {
        r#"<AdaptationSet contentType="audio" mimeType="audio/mp4"><SegmentTemplate timescale="1000" duration="2000" initialization="a-init.mp4" media="a-$Number$.m4s"/><Representation id="A" bandwidth="1" codecs="mp4a.40.2"/></AdaptationSet>"#
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2026-01-01T00:00:00Z" minimumUpdatePeriod="PT2S" timeShiftBufferDepth="PT30S" minBufferTime="PT2S"><Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4"><SegmentTemplate timescale="1000" duration="2000" initialization="v-init.mp4" media="v-$Number$.m4s"/><Representation id="V" bandwidth="2" codecs="avc1.4d400d"/></AdaptationSet>{audio}</Period><UTCTiming schemeIdUri="urn:mpeg:dash:utc:direct:2014" value="2026-01-01T00:00:00Z"/></MPD>"#
    )
}

#[test]
fn a_refresh_that_drops_the_last_set_does_not_index_past_the_list() {
    let mut synth = with_utc(Addressing::Number);
    let switch_at = AST_US + START + 8 * MICROS;
    synth.mpd_override = Some(Box::new(move |at| Some(two_set_mpd(at <= switch_at))));
    let (mut sim, _geo) = sim_with(synth, START, DashConfig::default());
    sim.run_for(20 * MICROS as u64);
    let audio_after: Vec<_> = sim
        .log
        .iter()
        .filter(|l| l.kind == Kind::Segment && l.send_utc_us > switch_at + 8 * MICROS)
        .filter(|l| l.url.contains("/a-"))
        .collect();
    assert!(
        audio_after.is_empty(),
        "the vanished audio set is not requested: {:?}",
        audio_after
            .iter()
            .map(|l| l.url.clone())
            .collect::<Vec<_>>()
    );
}

fn two_variant_mpd() -> String {
    format!(
        r#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="{}" minimumUpdatePeriod="PT2S" timeShiftBufferDepth="PT120S" suggestedPresentationDelay="PT6S" minBufferTime="PT2S"><Period id="P0" start="PT0S"><AdaptationSet contentType="video" mimeType="video/mp4"><SegmentTemplate timescale="1000" duration="2000" initialization="seg/$RepresentationID$/init.mp4" media="seg/$RepresentationID$/$Number$.m4s"/><Representation id="V9" bandwidth="62440" codecs="avc1.4d400d"/><Representation id="V8" bandwidth="262440" codecs="avc1.4d400d"/></AdaptationSet></Period><UTCTiming schemeIdUri="urn:mpeg:dash:utc:http-xsdate:2014" value="http://utc.test/xsdate"/></MPD>"#,
        common::world::xs_datetime_ms(AST_US)
    )
}

#[test]
fn a_vanish_picks_the_selected_variant_and_resumes_at_the_switch() {
    let mut synth = with_utc(Addressing::Number);
    synth.second_video = true;
    synth.tsb_us = Some(120 * MICROS);
    let switch_at = AST_US + START + 20 * MICROS;
    synth.mpd_override = Some(Box::new(move |at| (at > switch_at).then(two_variant_mpd)));
    let (mut sim, geo) = sim_with(synth, START, DashConfig::default());
    run_checked(&mut sim, geo, 40);
    let all: Vec<(String, i64)> = sim
        .log
        .iter()
        .filter(|l| l.kind == Kind::Segment)
        .filter_map(|l| common::synth::parse_segment_url(&l.url))
        .map(|(rep, n)| (rep, i64::try_from(n).unwrap_or(i64::MAX)))
        .collect();
    let v8: Vec<i64> = all
        .iter()
        .filter(|(rep, _)| rep == "V8")
        .map(|(_, n)| *n)
        .collect();
    assert!(!v8.is_empty(), "the selection's highest variant is played");
    assert!(
        v8.iter().all(|n| *n > 5),
        "the survivor resumes at the switch instant, not from 0: {v8:?}"
    );
    assert!(
        all.iter().all(|(rep, _)| rep != "V9"),
        "the lower variant is never played: {all:?}"
    );
}
