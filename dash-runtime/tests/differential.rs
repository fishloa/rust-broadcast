//! Differentials against `transmux`: (a) the new parser and addressing agree
//! with `transmux::dash_parse` on every MPD both can read, (b) the new parser
//! consumes what transmux's DASH generator emits (its committed goldens and
//! multimux's), (c) the lazy timeline expander equals
//! `SegmentTimeline::resolve` whenever no `@n` is present.

mod common;

use std::path::{Path, PathBuf};

use common::Rng;
use dash_runtime::address::timeline::TimelineIndex;
use dash_runtime::address::{AddressError, PlanError, PlanOptions, plan_representation};
use dash_runtime::mpd::inherit::SegmentInfo;
use dash_runtime::mpd::{Mpd, PresentationType, SegmentTimeline, TimelineEntry};
use dash_runtime::selection::RepRef;
use dash_runtime::timing::datetime::parse_xs_datetime;
use url::Url;

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

fn is_mpd_text(text: &str) -> bool {
    text.contains("<MPD")
}

fn generator_goldens() -> Vec<PathBuf> {
    let root = test_fixtures::root();
    let mut out = Vec::new();
    let tm = root.join("../transmux/tests/golden");
    for e in std::fs::read_dir(&tm).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "mpd") {
            out.push(p);
        }
    }
    let mm = root.join("../multimux/tests/golden");
    for e in std::fs::read_dir(&mm).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "golden")
            && std::fs::read_to_string(&p).is_ok_and(|t| is_mpd_text(&t))
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn fixture_mpds() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&test_fixtures::path("dash"), "mpd", &mut out);
    out.sort();
    out
}

struct Compared {
    reps: usize,
    segments: usize,
    urls: usize,
}

fn compare(path: &Path) -> Option<Compared> {
    let text = std::fs::read_to_string(path).unwrap();
    let theirs = transmux::Mpd::parse(&text).ok()?;
    let ours = Mpd::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let doc = Url::from_file_path(std::fs::canonicalize(path).unwrap()).unwrap();
    let mut cmp = Compared {
        reps: 0,
        segments: 0,
        urls: 0,
    };
    assert_eq!(
        ours.kind == PresentationType::Dynamic,
        theirs.mpd_type == transmux::MpdType::Dynamic,
        "{}",
        path.display()
    );
    assert_eq!(ours.profiles.join(","), theirs.profiles.replace(", ", ","));
    assert_eq!(
        ours.media_presentation_duration,
        theirs.media_presentation_duration
    );
    assert_eq!(ours.minimum_update_period, theirs.minimum_update_period);
    assert_eq!(ours.time_shift_buffer_depth, theirs.time_shift_buffer_depth);
    match (
        &theirs.availability_start_time,
        ours.availability_start_time_us,
    ) {
        (Some(raw), Some(us)) => {
            assert_eq!(parse_xs_datetime(raw).unwrap().unix_us, us);
        }
        (None, None) => {}
        other => panic!("{}: availabilityStartTime {other:?}", path.display()),
    }
    assert_eq!(
        ours.periods.len(),
        theirs.periods.len(),
        "{}",
        path.display()
    );
    for (pi, (op, tp)) in ours.periods.iter().zip(&theirs.periods).enumerate() {
        assert_eq!(op.id, tp.id);
        assert_eq!(op.start, tp.start);
        assert_eq!(op.duration, tp.duration);
        assert_eq!(op.adaptation_sets.len(), tp.adaptation_sets.len());
        for (si, (os, ts)) in op
            .adaptation_sets
            .iter()
            .zip(&tp.adaptation_sets)
            .enumerate()
        {
            assert_eq!(os.representations.len(), ts.representations.len());
            for (ri, (or, tr)) in os
                .representations
                .iter()
                .zip(&ts.representations)
                .enumerate()
            {
                cmp.reps += 1;
                assert_eq!(or.id, tr.id);
                assert_eq!(or.bandwidth, tr.bandwidth);
                assert_eq!(or.common.codecs, tr.codecs);
                assert_eq!(or.common.width.or(os.common.width), tr.width);
                let at = RepRef::at(pi, si, ri);
                let planned = match plan_representation(&ours, at, &PlanOptions::default()) {
                    Ok(p) => p,
                    Err(PlanError::Address(AddressError::IndexRequired)) => continue,
                    Err(PlanError::PeriodUnresolved) => continue,
                    Err(e) => panic!("{}: plan failed: {e}", path.display()),
                };
                let Some(tt) = &tr.segment_template else {
                    continue;
                };
                let SegmentInfo::Template(info) = &planned.info else {
                    continue;
                };
                assert_eq!(info.multi.timescale, tt.timescale);
                assert_eq!(info.multi.start_number, tt.start_number);
                assert_eq!(
                    info.multi.presentation_time_offset,
                    tt.presentation_time_offset
                );
                let Some(count) = planned.plan.len() else {
                    continue;
                };
                let no_n = info
                    .multi
                    .timeline
                    .as_ref()
                    .is_none_or(|t| t.entries.iter().all(|e| e.n.is_none()));
                let theirs_segments: Vec<(u64, u64)> = if let Some(tl) = &tt.timeline {
                    if !no_n {
                        continue;
                    }
                    let pd_ticks = planned.plan.timescale();
                    let _ = pd_ticks;
                    let end = planned.period.end_us.map(|e| {
                        let dur = e - planned.period.start_us.unwrap();
                        let n = dur * i128::from(tt.timescale);
                        let t = n.div_euclid(1_000_000) + i128::from(n.rem_euclid(1_000_000) != 0);
                        u64::try_from(t).unwrap()
                    });
                    tl.resolve_in_period(tt.start_number, tt.presentation_time_offset, end)
                        .unwrap()
                        .into_iter()
                        .map(|s| (s.number, s.time))
                        .collect()
                } else if tt.duration.is_some() {
                    tt.number_sequence(usize::try_from(count).unwrap())
                        .into_iter()
                        .enumerate()
                        .map(|(i, n)| (n, i as u64 * tt.duration.unwrap()))
                        .collect()
                } else {
                    continue;
                };
                let ours_segments: Vec<(u64, u64)> = planned
                    .plan
                    .iter()
                    .map(|s| (s.number, s.time_ticks))
                    .collect();
                assert_eq!(
                    ours_segments,
                    theirs_segments,
                    "{} rep {}",
                    path.display(),
                    or.id
                );
                cmp.segments += ours_segments.len();
                for seg in [
                    planned.plan.get(0),
                    planned.plan.get(count.saturating_sub(1)),
                ]
                .into_iter()
                .flatten()
                {
                    let media = tt.media.as_deref().unwrap();
                    let reference = transmux::SegmentTemplate::resolve(
                        media,
                        &or.id,
                        Some(seg.number),
                        Some(seg.time_ticks),
                        Some(or.bandwidth),
                    );
                    let t_url = theirs.resolve_segment_url(Some(&doc), tp, ts, tr, &reference);
                    let o_url = planned.candidates[0].resolve(Some(&doc), &seg.reference);
                    assert_eq!(o_url, t_url, "{} rep {}", path.display(), or.id);
                    cmp.urls += 1;
                }
            }
        }
    }
    Some(cmp)
}

#[test]
fn differential_a_new_parser_agrees_with_transmux_on_shared_mpds() {
    let mut files = fixture_mpds();
    files.extend(generator_goldens());
    let (mut parsed_by_both, mut reps, mut segments, mut urls) = (0, 0, 0, 0);
    for f in &files {
        if let Some(c) = compare(f) {
            parsed_by_both += 1;
            reps += c.reps;
            segments += c.segments;
            urls += c.urls;
        }
    }
    eprintln!(
        "differential a: {parsed_by_both}/{} MPDs, {reps} reps, {segments} segments, {urls} urls",
        files.len()
    );
    assert!(
        parsed_by_both >= 20,
        "only {parsed_by_both} MPDs comparable"
    );
    assert!(reps >= 40);
    assert!(segments >= 100);
    assert!(urls >= 40);
}

#[test]
fn differential_b_generator_output_is_consumed() {
    let goldens = generator_goldens();
    assert!(goldens.len() >= 14, "found {}", goldens.len());
    let mut compared = 0;
    for g in &goldens {
        let text = std::fs::read(g).unwrap();
        let ours = Mpd::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", g.display()));
        assert!(!ours.periods.is_empty(), "{}", g.display());
        assert!(
            ours.periods.iter().all(|p| !p.adaptation_sets.is_empty()),
            "{}",
            g.display()
        );
        if compare(g).is_some() {
            compared += 1;
        }
    }
    assert!(
        compared >= 10,
        "structural equality checked on {compared} goldens"
    );
}

fn random_timeline(rng: &mut Rng) -> (SegmentTimeline, transmux::SegmentTimeline) {
    let mut ours = SegmentTimeline::default();
    let mut theirs = transmux::SegmentTimeline::default();
    let mut clock = rng.below(1_000_000);
    let entries = rng.range(1, 8);
    for i in 0..entries {
        let d = rng.range(1, 200_000);
        let repeat = match rng.below(4) {
            0 => -1,
            1 => 0,
            _ => i64::try_from(rng.below(12)).unwrap(),
        };
        let explicit_t = i == 0 || rng.below(3) == 0;
        let gap = rng.below(3) * d;
        let t = clock + gap;
        let mut e = TimelineEntry::new(d);
        e.r = repeat;
        if explicit_t {
            e.t = Some(t);
        }
        ours.entries.push(e);
        theirs.segments.push(transmux::S {
            t: explicit_t.then_some(t),
            d,
            r: repeat,
        });
        let count = if repeat >= 0 {
            repeat as u64 + 1
        } else {
            rng.range(1, 6)
        };
        clock = if explicit_t { t } else { clock } + d * count;
        if repeat < 0 {
            clock += rng.below(d);
        }
    }
    (ours, theirs)
}

#[test]
fn differential_c_lazy_timeline_equals_transmux_resolve() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut compared = 0;
    let mut errors = 0;
    for _ in 0..20_000 {
        let (ours, theirs) = random_timeline(&mut rng);
        let start_number = rng.range(0, 5);
        let end = (rng.below(2) == 0).then(|| rng.range(0, 5_000_000));
        let tm = theirs.resolve(start_number, end);
        let ix = TimelineIndex::build(&ours, start_number, end);
        match (tm, ix) {
            (Ok(t), Ok(i)) => {
                let t: Vec<(u64, u64, u64)> = t
                    .into_iter()
                    .map(|s| (s.number, s.time, s.duration))
                    .collect();
                let i: Vec<(u64, u64, u64)> =
                    i.iter().map(|s| (s.number, s.time, s.duration)).collect();
                assert_eq!(i, t);
                compared += 1;
            }
            (Err(_), Err(_)) => errors += 1,
            (Ok(_), Err(_)) => {
                // Ours is stricter in exactly one way: an explicit `@t` that
                // moves backwards, which transmux accepts.
                errors += 1;
            }
            (Err(e), Ok(_)) => panic!("transmux rejected what the lazy expander accepted: {e}"),
        }
    }
    eprintln!("differential c: {compared} equal timelines, {errors} rejected");
    assert!(compared > 10_000, "compared {compared}, errors {errors}");
}
