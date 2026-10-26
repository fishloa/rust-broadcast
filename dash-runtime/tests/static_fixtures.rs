//! The expanded segment plan of every static fixture equals the files on
//! disk, checked against an independent oracle (MP4Box `-diso` output stored
//! next to each fixture, and the bytes of the real files).

mod common;

use common::{load, oracle_epct, oracle_tfdt};
use dash_runtime::address::base_index;
use dash_runtime::address::{AddressError, PlanError, PlanOptions, plan_representation};
use dash_runtime::mpd::inherit::SegmentInfo;
use dash_runtime::mpd::inherit::resolve_segment_info;
use dash_runtime::selection::RepRef;

const SECOND_US: i128 = 1_000_000;

fn plans_of(
    rel: &str,
) -> (
    common::Loaded,
    Vec<(RepRef, dash_runtime::address::PlannedRepresentation)>,
) {
    let l = load(rel);
    let plans = l.reps().into_iter().map(|r| (r, l.plan(r))).collect();
    (l, plans)
}

fn exists(
    l: &common::Loaded,
    p: &dash_runtime::address::PlannedRepresentation,
    reference: &str,
) -> bool {
    l.file_of(p, reference).is_file()
}

#[test]
fn ffmpeg_timeline_manifest_matches_disk() {
    let (l, plans) = plans_of("dash/manifest.mpd");
    assert_eq!(plans.len(), 2);
    let (_, video) = &plans[0];
    let (_, audio) = &plans[1];
    assert_eq!(video.plan.len(), Some(3));
    assert_eq!(audio.plan.len(), Some(4));
    for (p, prefix) in [(video, "chunk-stream0-"), (audio, "chunk-stream1-")] {
        for (i, seg) in p.plan.iter().enumerate() {
            assert_eq!(seg.number, i as u64 + 1);
            let file = l.file_of(p, &seg.reference);
            assert!(file.is_file(), "{}", file.display());
            assert_eq!(
                file.file_name().unwrap().to_str().unwrap(),
                format!("{prefix}{:05}.m4s", seg.number)
            );
        }
        let init = p.plan.init().expect("initialization");
        assert!(exists(&l, p, &init.reference));
    }
    // R7: the video timeline starts at S@t=2070 while the first tfdt is 0.
    let first = video.plan.get(0).unwrap();
    assert_eq!(first.time_ticks, 2070);
    assert_eq!(first.reference, "chunk-stream0-00001.m4s");
    assert_eq!(video.plan.get(1).unwrap().time_ticks, 2070 + 90_000);
    assert_eq!(first.start_us, 2070 * SECOND_US / 90_000);
    // Audio durations are 41984, 44032, 45056, 3072 samples at 44.1 kHz.
    let durations: Vec<u64> = audio.plan.iter().map(|s| s.duration_ticks).collect();
    assert_eq!(durations, vec![41_984, 44_032, 45_056, 3_072]);
}

#[test]
fn ffmpeg_quirk_time_is_the_sidx_value_not_tfdt() {
    let (l, plans) = plans_of("dash/timeline-ffmpeg-quirk/manifest.mpd");
    let tfdt = oracle_tfdt("dash/timeline-ffmpeg-quirk");
    let epct = oracle_epct("dash/timeline-ffmpeg-quirk");
    let (_, video) = &plans[0];
    let (_, audio) = &plans[1];
    assert_eq!((video.plan.len(), audio.plan.len()), (Some(3), Some(4)));
    for (p, video_stream) in [(video, true), (audio, false)] {
        for seg in p.plan.iter() {
            let name = l
                .file_of(p, &seg.reference)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                epct[&name], seg.time_ticks,
                "{name}: $Time$ is the sidx earliest_presentation_time"
            );
            let offset = seg.time_ticks - tfdt[&name][0];
            assert_eq!(offset, if video_stream { 2090 } else { 0 }, "{name}");
        }
    }
}

#[test]
fn gpac_timeline_video_matches_tfdt_and_audio_quirk_is_kept() {
    let (l, plans) = plans_of("dash/timeline-gpac/timeline.mpd");
    let tfdt = oracle_tfdt("dash/timeline-gpac");
    let (_, video) = &plans[0];
    let (_, audio) = &plans[1];
    assert_eq!(video.plan.len(), Some(6));
    assert_eq!(audio.plan.len(), Some(6));
    for seg in video.plan.iter() {
        let file = l.file_of(video, &seg.reference);
        assert!(file.is_file(), "{}", file.display());
        let name = file.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(
            tfdt[&name][0], seg.time_ticks,
            "{name}: video timeline equals tfdt"
        );
    }
    let mut mismatches = 0;
    for seg in audio.plan.iter() {
        let file = l.file_of(audio, &seg.reference);
        assert!(file.is_file(), "{}", file.display());
        let name = file.file_name().unwrap().to_str().unwrap().to_owned();
        if tfdt[&name][0] != seg.time_ticks {
            mismatches += 1;
        }
    }
    assert_eq!(
        mismatches, 5,
        "the real audio timeline disagrees with the media after segment 1"
    );
    let init = video.plan.init().unwrap();
    assert_eq!(init.reference, "seg-1-.mp4");
    assert!(exists(&l, video, &init.reference));
}

#[test]
fn segment_list_matches_disk() {
    let (l, plans) = plans_of("dash/segmentlist/list.mpd");
    assert_eq!(plans.len(), 2);
    for (n, (_, p)) in plans.iter().enumerate() {
        assert!(matches!(p.info, SegmentInfo::List(_)));
        assert_eq!(p.plan.len(), Some(6));
        for (i, seg) in p.plan.iter().enumerate() {
            assert_eq!(seg.number, i as u64 + 1);
            assert_eq!(seg.reference, format!("seg-{n}-{:03}.m4s", i + 1));
            assert!(exists(&l, p, &seg.reference));
            assert_eq!(seg.start_us, i as i128 * 2 * SECOND_US);
            assert_eq!(seg.duration_us, 2 * SECOND_US);
        }
        let init = p.plan.init().unwrap();
        assert_eq!(init.reference, format!("init-{n}.m4s"));
        assert!(exists(&l, p, &init.reference));
    }
}

#[test]
fn on_demand_sidx_ranges_tile_the_real_file() {
    for dir in ["dash/ondemand-sidx", "dash/ondemand-gpac-pto-quirk"] {
        let l = load(&format!("{dir}/ondemand.mpd"));
        for at in l.reps() {
            let first = plan_representation(&l.mpd, at, &PlanOptions::default());
            assert!(matches!(
                first,
                Err(PlanError::Address(AddressError::IndexRequired))
            ));
            let (period, set, rep) = at.resolve(&l.mpd).unwrap();
            let info = resolve_segment_info(period, set, rep).unwrap();
            let SegmentInfo::Base(base) = &info else {
                panic!("expected SegmentBase")
            };
            let index_range = base.index_range.expect("indexRange");
            let bytes = l.base_file_bytes(at);
            let (a, b) = (
                index_range.first as usize,
                index_range.last.unwrap() as usize,
            );
            let sidx = base_index::parse_index(&bytes[a..=b]).unwrap();
            let anchor = base_index::anchor_after(&index_range).unwrap();
            let planned = plan_representation(
                &l.mpd,
                at,
                &PlanOptions::default().with_index(&sidx, anchor),
            )
            .unwrap();
            assert_eq!(planned.plan.len(), Some(6), "{dir}");
            let mut expected_next = None;
            for seg in planned.plan.iter() {
                let r = seg.byte_range.unwrap();
                let (first, last) = (r.first as usize, r.last.unwrap() as usize);
                assert_eq!(
                    &bytes[first + 4..first + 8],
                    b"moof",
                    "{dir}: range starts at a moof"
                );
                if let Some(next) = expected_next {
                    assert_eq!(first, next, "{dir}: ranges are contiguous");
                }
                expected_next = Some(last + 1);
            }
            assert_eq!(
                expected_next,
                Some(bytes.len()),
                "{dir}: ranges end at the end of the file"
            );
            let init = planned.plan.init().unwrap();
            let range = init.byte_range.unwrap();
            assert_eq!(&bytes[4..8], b"ftyp");
            assert_eq!(range.first, 0);
            assert_eq!(&bytes[a + 4..a + 8], b"sidx");
        }
    }
}

#[test]
fn ondemand_pto_quirk_does_not_overflow() {
    let l = load("dash/ondemand-gpac-pto-quirk/ondemand.mpd");
    let at = RepRef::at(0, 0, 0);
    let (period, set, rep) = at.resolve(&l.mpd).unwrap();
    let info = resolve_segment_info(period, set, rep).unwrap();
    let SegmentInfo::Base(b) = &info else {
        panic!()
    };
    assert_eq!(b.presentation_time_offset, Some(5_354_157_010_056_287_232));
    let bytes = l.base_file_bytes(at);
    let r = b.index_range.unwrap();
    let sidx =
        base_index::parse_index(&bytes[r.first as usize..=r.last.unwrap() as usize]).unwrap();
    let planned = plan_representation(
        &l.mpd,
        at,
        &PlanOptions::default().with_index(&sidx, base_index::anchor_after(&r).unwrap()),
    )
    .unwrap();
    let first = planned.plan.get(0).unwrap();
    assert_eq!(first.time_ticks, 512);
    assert!(
        first.start_us < 0,
        "a nonsense PTO yields a hugely negative start, never a wrap"
    );
}

#[test]
fn multiperiod_restart_plans_each_period_and_keeps_the_tail_quirk() {
    let (l, plans) = plans_of("dash/multiperiod-restart/multiperiod.mpd");
    assert_eq!(plans.len(), 4);
    // Period 1 is declared 6.013 s, so the spec reading (A.3.3) is four
    // 2 s-nominal segments with a 13 ms tail; the real files stop at three.
    let (_, p1_video) = &plans[0];
    assert_eq!(p1_video.plan.len(), Some(4));
    let tail = p1_video.plan.get(3).unwrap();
    assert!(tail.duration_us < 14_000, "tail is {} us", tail.duration_us);
    assert!(exists(
        &l,
        p1_video,
        &p1_video.plan.get(2).unwrap().reference
    ));
    assert!(
        !exists(&l, p1_video, &tail.reference),
        "the 4th segment is not on disk"
    );
    // Period 2: start 6 s, duration 6 s, three segments each, all on disk.
    for (_, p) in &plans[2..] {
        assert_eq!(p.period.start_us, Some(6 * SECOND_US));
        assert_eq!(p.plan.len(), Some(3));
        for seg in p.plan.iter() {
            assert!(exists(&l, p, &seg.reference));
        }
        let first = p.plan.get(0).unwrap();
        assert_eq!(first.start_us, 0);
        assert_eq!(p.plan.period_start_us(), 6 * SECOND_US);
    }
}

#[test]
fn multiperiod_continuous_numbers_and_pto() {
    let (l, plans) = plans_of("dash/multiperiod-continuous/multiperiod.mpd");
    let tfdt = oracle_tfdt("dash/timeline-gpac");
    assert_eq!(plans.len(), 2);
    let (_, a) = &plans[0];
    let (_, b) = &plans[1];
    assert_eq!(a.plan.len(), Some(3));
    assert_eq!(b.plan.len(), Some(3));
    let numbers = |p: &dash_runtime::address::PlannedRepresentation| -> Vec<u64> {
        p.plan.iter().map(|s| s.number).collect()
    };
    assert_eq!(numbers(a), vec![1, 2, 3]);
    assert_eq!(numbers(b), vec![4, 5, 6]);
    for (p, base_time) in [(a, 0u64), (b, 76_800)] {
        for (i, seg) in p.plan.iter().enumerate() {
            let file = l.file_of(p, &seg.reference);
            assert!(file.is_file(), "{}", file.display());
            let name = file.file_name().unwrap().to_str().unwrap().to_owned();
            assert_eq!(tfdt[&name][0], seg.time_ticks, "{name}");
            assert_eq!(seg.time_ticks, base_time + i as u64 * 25_600);
        }
    }
    // Period b: media time 76800 with PTO 76800 is Period-relative 0.
    assert_eq!(b.plan.get(0).unwrap().start_us, 0);
    assert_eq!(b.plan.get(1).unwrap().start_us, 2 * SECOND_US);
    assert_eq!(b.plan.period_start_us(), 6 * SECOND_US);
}
