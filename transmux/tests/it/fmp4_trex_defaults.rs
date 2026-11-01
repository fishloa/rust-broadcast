//! A fragment whose `trun` carries sample sizes only takes its sample
//! durations (and flags) from the init movie's `trex`, as ISO/IEC 14496-12
//! §8.8.7 says. Real GPAC output: `fixtures/dash/multiperiod-restart/`
//! (`PROVENANCE`: `fixtures/dash/PROVENANCE.md`), whose first video fragment
//! has `trun` flags 0x205 (data offset, first-sample flags, sample size) and
//! no `tfhd` defaults. It came out with every sample at `dts` 0, duration 0.

use broadcast_common::Unpackage;
use transmux::media::Fmp4Demux;

const VIDEO_TIMESCALE_TICKS_PER_FRAME: i64 = 512;
const FRAGMENT_TICKS: i64 = 25_600;

fn demux(init: &str, segments: &[&str]) -> transmux::Track {
    let mut whole = test_fixtures::read(&format!("dash/multiperiod-restart/{init}"));
    for segment in segments {
        whole.extend(test_fixtures::read(&format!(
            "dash/multiperiod-restart/{segment}"
        )));
    }
    Fmp4Demux::new()
        .unpackage(whole.as_slice())
        .expect("demux")
        .tracks
        .remove(0)
}

#[test]
fn a_trun_without_durations_takes_the_trex_default() {
    let track = demux("p1_dash_track1_init.mp4", &["p1_dash_track1_1.m4s"]);
    assert_eq!(track.samples.len(), 50);
    for (i, sample) in track.samples.iter().enumerate() {
        let i = i64::try_from(i).unwrap();
        assert_eq!(sample.duration, Some(512), "sample {i} duration");
        assert_eq!(
            sample.dts,
            Some(i * VIDEO_TIMESCALE_TICKS_PER_FRAME),
            "sample {i} dts"
        );
    }
}

#[test]
fn consecutive_fragments_join_without_a_gap_once_durations_are_known() {
    let track = demux(
        "p1_dash_track1_init.mp4",
        &["p1_dash_track1_1.m4s", "p1_dash_track1_2.m4s"],
    );
    let dts: Vec<i64> = track.samples.iter().map(|s| s.dts.unwrap()).collect();
    assert!(dts.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
    assert_eq!(
        dts[50], FRAGMENT_TICKS,
        "the second fragment's tfdt is 50 frames in"
    );
}

#[test]
fn only_the_first_sample_of_the_fragment_is_a_sync_sample() {
    let track = demux("p1_dash_track1_init.mp4", &["p1_dash_track1_1.m4s"]);
    assert!(track.samples[0].flags.is_sync);
    assert!(
        track.samples[1..].iter().any(|s| !s.flags.is_sync),
        "the trex default flags mark the rest as dependent frames"
    );
}
