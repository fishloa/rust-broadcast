//! Gapless trim reaches the IR, and is read only where a container states it.
//!
//! Each expected value below is the one `ffprobe` reports for the same file:
//! `ffprobe -v error -select_streams a:0 -show_entries
//! stream=initial_padding -of default=nw=1:nk=1`, or (for an `elst` file,
//! where `ffprobe` applies the edit list) the magnitude of the first packet's
//! negative `pts` under `-ignore_editlist 1`.

#[cfg(feature = "codec")]
use transmux::TrackId;
use transmux::{DecoderPreRoll, GaplessSignal, GaplessTrim, Sample, SampleTrim, TrackSpec};

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

#[cfg(feature = "codec")]
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

#[test]
fn an_apple_aac_track_takes_its_trim_from_itunsmpb() {
    let Some(bytes) =
        test_fixtures::require_bytes("mp4/tags/afconvert-aac.m4a", "gapless iTunSMPB")
    else {
        eprintln!("skipping: private fixtures are not checked out");
        return;
    };
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("demux the fixture");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|track| track.spec).collect();
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(2112, Some(756)),
        "iTunSMPB priming 2112 and padding 756 (ffprobe: packet skip_samples=2112, \
         the tag's original count 132300)"
    );
}

#[test]
fn an_mp4_opus_track_reports_its_roll_distance_as_pre_roll() {
    let specs = mp4_specs("mp4/opus.mp4");
    let spec = audio_spec(&specs);
    assert_eq!(
        spec.pre_roll,
        Some(DecoderPreRoll::from_samples(4).expect("a pre-roll")),
        "sgpd('roll') roll_distance -4 is a 4-sample decoder pre-roll"
    );
    assert_eq!(
        spec.padding,
        GaplessTrim::new(312, None),
        "the roll distance must not be folded into the gapless trim"
    );
}

#[test]
fn an_mp4_prol_sample_group_is_a_pre_roll_too() {
    let specs = mp4_specs("mp4/aac-prol.mp4");
    let spec = audio_spec(&specs);
    assert_eq!(
        spec.pre_roll,
        Some(DecoderPreRoll::from_samples(80).expect("a pre-roll")),
        "a positive prol roll_distance is a pre-roll"
    );
    assert!(
        spec.padding != GaplessTrim::new(80, None),
        "the pre-roll is not folded into the gapless trim"
    );
}

#[test]
fn a_matroska_block_discard_padding_is_a_per_sample_trim() {
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let media = test_harness::collect::media_from_matroska(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux the fixture");
    let track = media
        .tracks
        .iter()
        .find(|track| track.spec.config.audio_sample_rate().is_some())
        .expect("an audio track");
    let last = track.samples.last().expect("a last sample");
    assert_eq!(
        last.trim,
        SampleTrim::new(0, 648),
        "DiscardPadding 13.5 ms at 48 kHz (ffprobe: discard_padding=648)"
    );
    assert!(
        track.samples[..track.samples.len() - 1]
            .iter()
            .all(|sample| sample.trim.is_none()),
        "only the final Block states DiscardPadding"
    );
}

#[test]
fn a_matroska_seek_pre_roll_is_a_pre_roll_not_a_trim() {
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let media = test_harness::collect::media_from_matroska(
        &bytes,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux the fixture");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|track| track.spec).collect();
    let spec = audio_spec(&specs);
    assert_eq!(
        spec.pre_roll,
        Some(DecoderPreRoll::from_samples(3840).expect("a pre-roll")),
        "SeekPreRoll 80 ms at 48 kHz, rounded to nearest sample"
    );
    assert_eq!(
        spec.padding,
        GaplessTrim::new(312, None),
        "the pre-roll is not folded into the track trim"
    );
}

/// `opus.mp4` states its priming twice, in agreement. Patching the `dOps`
/// `PreSkip` to disagree leaves the edit list's value in force (it is the
/// normative signal) and reports the codec's value as one finding.
#[test]
fn a_disagreeing_codec_pre_skip_is_reported_and_the_edit_list_wins() {
    let mut bytes = test_fixtures::read("mp4/opus.mp4");
    let at = bytes
        .windows(4)
        .position(|window| window == b"dOps")
        .expect("a dOps box");
    // size(4) 'dOps'(4) version(1) channels(1) pre_skip(2 BE, ISO/IEC 14496-12 dOps)
    bytes[at + 6..at + 8].copy_from_slice(&100u16.to_be_bytes());
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .expect("demux");
    let specs: Vec<TrackSpec> = media.tracks.into_iter().map(|track| track.spec).collect();
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(312, None),
        "the edit list's media_time 312 wins"
    );
    assert_eq!(
        media.gapless_conflicts,
        vec![(
            transmux::TrackId::new(1),
            GaplessSignal::EditList,
            GaplessSignal::CodecPreSkip,
        )],
        "exactly one finding names the ignored signal"
    );
}

/// An MP3 track in MP4 whose first sample carries the LAME tag: the demuxer
/// reads the tag and states the padding, with the tag frame passed through
/// unchanged. The MP3 bytes are the real `audio/audio.mp3` elementary stream
/// (its first frame is the `Info` tag frame ffmpeg wrote).
#[test]
fn an_mp4_mp3_track_takes_its_padding_from_the_lame_tag() {
    use broadcast_bitstream::{MpegAudioFrameHeader, MpegAudioLayer};
    use broadcast_core::timescale::Timescale;
    use core::num::NonZeroU32;
    use isobmff::mp4esds::{
        DecoderConfigDescriptor, ESDescriptor, EsdsBox, OTI_MPEG1_AUDIO, SLConfigDescriptor,
    };
    use test_harness::collect::{Collected, CollectedTrack};
    use transmux::{CodecConfig, TrackId};

    let bytes = test_fixtures::read("audio/audio.mp3");
    let (start, header) = (0..bytes.len())
        .find_map(|offset| {
            MpegAudioFrameHeader::parse(&bytes[offset..])
                .ok()
                .map(|header| (offset, header))
        })
        .expect("an MPEG audio frame");
    let rate = header.sample_rate();
    let mut samples = Vec::new();
    let mut offset = start;
    let mut dts = 0i64;
    while let Ok(frame) = MpegAudioFrameHeader::parse(&bytes[offset..]) {
        let end = (offset + frame.frame_length()).min(bytes.len());
        if end <= offset {
            break;
        }
        if samples.is_empty() {
            assert!(
                broadcast_bitstream::parse_lame_tag(&bytes[offset..end])
                    .expect("a frame parses")
                    .is_some(),
                "the first frame carries the Info tag"
            );
        }
        samples.push(Sample::timed_raw(
            bytes[offset..end].to_vec(),
            dts,
            dts,
            NonZeroU32::new(frame.samples_per_frame()),
        ));
        offset = end;
        dts += i64::from(frame.samples_per_frame());
    }
    let esds = EsdsBox::new(ESDescriptor::new(
        1,
        0,
        Some(DecoderConfigDescriptor::new(
            OTI_MPEG1_AUDIO,
            0x05,
            false,
            0,
            128_000,
            128_000,
            None,
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ));
    let spec = TrackSpec::new(
        TrackId::new(1),
        Timescale::try_new(rate).expect("a timescale"),
        CodecConfig::MpegAudio {
            esds,
            layer: MpegAudioLayer::LayerIII,
            channel_count: u16::from(header.channels()),
            sample_rate: rate,
            sample_size: 16,
        },
    );
    let media = Collected::new(
        vec![CollectedTrack::new(spec, samples)],
        Timescale::try_new(1000).expect("a movie timescale"),
    );
    let out = test_harness::collect::cmaf_from_media(&media, std::time::Duration::from_secs(1))
        .expect("mux");
    let back = test_harness::collect::media_from_mp4(
        &out.to_contiguous(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux");
    let specs: Vec<TrackSpec> = back.tracks.into_iter().map(|track| track.spec).collect();
    assert_eq!(
        audio_spec(&specs).padding,
        GaplessTrim::new(1105, Some(431)),
        "LAME delay 576 + 529 decoder delay, padding 960 - 529 (ffprobe: skip_samples=1105, \
         discard_padding=431)"
    );
}
