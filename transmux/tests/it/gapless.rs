//! Gapless trim reaches the IR, and is read only where a container states it.
//!
//! Each expected value below is the one `ffprobe` reports for the same file:
//! `ffprobe -v error -select_streams a:0 -show_entries
//! stream=initial_padding -of default=nw=1:nk=1`, or (for an `elst` file,
//! where `ffprobe` applies the edit list) the magnitude of the first packet's
//! negative `pts` under `-ignore_editlist 1`.

use transmux::{GaplessTrim, Sample, TrackId, TrackSpec};

fn audio_spec(specs: &[TrackSpec]) -> &TrackSpec {
    specs
        .iter()
        .find(|spec| spec.config.audio_sample_rate().is_some())
        .expect("an audio track")
}

fn mp4_specs(path: &str) -> Vec<TrackSpec> {
    let media = test_harness::collect::media_from_mp4(
        &test_fixtures::read(path),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux the fixture");
    media.tracks.into_iter().map(|track| track.spec).collect()
}

#[test]
fn an_mp4_opus_track_takes_its_priming_from_d_ops() {
    let specs = mp4_specs("mp4/opus.mp4");
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(312, None),
        "dOps PreSkip is the priming (ffprobe: initial_padding=312)"
    );
}

#[test]
fn a_fragmented_mp4_opus_track_takes_it_from_d_ops_too() {
    let specs = mp4_specs("mp4/frag/opus.frag.mp4");
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(312, None),
        "no elst here either; dOps states it (ffprobe: initial_padding=312)"
    );
}

#[test]
fn an_mp4_aac_track_takes_its_priming_from_the_edit_list() {
    let specs = mp4_specs("ts/heaac/heaac-v1.mp4");
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(2112, None),
        "elst media_time 2112, in the media timescale (ffprobe: first packet pts=-2112)"
    );
}

#[test]
fn a_matroska_opus_track_takes_its_priming_from_codec_delay() {
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let media = test_harness::collect::media_from_matroska(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux the fixture");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|track| track.spec).collect();
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(312, None),
        "CodecDelay 6.5 ms at 48 kHz (ffprobe: initial_padding=312)"
    );
}

#[test]
fn an_mp4_opus_remux_keeps_its_d_ops_priming() {
    let bytes = test_fixtures::read("mp4/opus.mp4");
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("demux");
    let out = test_harness::collect::cmaf_from_media(&media, std::time::Duration::from_secs(1))
        .expect("repackage");
    let back = test_harness::collect::media_from_mp4(
        &out.to_contiguous(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("re-demux");
    let specs: Vec<TrackSpec> = back.tracks.into_iter().map(|track| track.spec).collect();
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(312, None),
        "dOps pre_skip is written back unchanged"
    );
}

#[test]
fn the_stage_conversion_carries_padding_both_ways() {
    let specs = mp4_specs("mp4/opus.mp4");
    let spec = audio_spec(&specs).clone();
    let parameters = transmux::codec_parameters_from_track(&spec).expect("converts");
    assert_eq!(parameters.initial_padding(), 312);
    assert_eq!(parameters.trailing_padding(), None);
    let back =
        transmux::track_spec_from_codec_parameters(TrackId::new(9), &parameters).expect("converts");
    assert_eq!(back.padding, GaplessTrim::new(312, None));
}

#[test]
fn a_sample_carries_no_trim_unless_the_container_states_it() {
    let sample: Sample = Sample::timed_raw(std::vec![1u8, 2, 3], 0, 0, None);
    assert!(sample.trim.is_none());
}
