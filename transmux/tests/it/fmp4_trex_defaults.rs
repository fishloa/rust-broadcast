//! A fragment whose `trun` carries sample sizes only takes its sample
//! durations (and flags) from the init movie's `trex`, as ISO/IEC 14496-12:2026
//! §8.8.7 says. Real GPAC output: `fixtures/dash/multiperiod-restart/`
//! (`PROVENANCE`: `fixtures/dash/PROVENANCE.md`), whose first video fragment
//! has `trun` flags 0x205 (data offset, first-sample flags, sample size) and
//! no `tfhd` defaults. It came out with every sample at `dts` 0, duration 0.

const VIDEO_TIMESCALE_TICKS_PER_FRAME: i64 = 512;
const FRAGMENT_TICKS: i64 = 25_600;

fn demux(init: &str, segments: &[&str]) -> transmux::Track {
    let mut whole = test_fixtures::read(&format!("dash/multiperiod-restart/{init}"));
    for segment in segments {
        whole.extend(test_fixtures::read(&format!(
            "dash/multiperiod-restart/{segment}"
        )));
    }
    test_harness::collect::media_from_mp4(
        whole.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
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
        assert_eq!(
            sample.timing.and_then(|t| t.duration()),
            Some(512),
            "sample {i} duration"
        );
        assert_eq!(
            sample.timing.and_then(|t| t.dts()),
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
    let dts: Vec<i64> = track
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.dts()).unwrap())
        .collect();
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

// ── `trex.default_sample_size` (ISO/IEC 14496-12:2026 §8.8.3) ──────────────
//
// A `trun` that carries no per-sample size takes the `tfhd`
// `default_sample_size`, which in turn defaults to the track's `trex` value
// (§8.8.7) — and a `trex` value of 0 means "not declared", never a real
// zero-byte sample.

const SAMPLE_COUNT: u32 = 3;
const TRACK_ID: u32 = 1;
const BOX_HEADER_LEN: usize = 8;
const TREX_DEFAULT_SAMPLE_SIZE_OFFSET: usize = 16;

fn boxed(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + BOX_HEADER_LEN) as u32)
        .to_be_bytes()
        .to_vec();
    b.extend_from_slice(fourcc);
    b.extend_from_slice(body);
    b
}

fn avc_spec() -> transmux::pipeline::TrackSpec {
    let record = isobmff::avc_config::AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![isobmff::nalu_types::AvcSps(vec![
            0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
        ])],
        pps: vec![isobmff::nalu_types::AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    transmux::pipeline::TrackSpec::new(
        transmux::TrackId::new(TRACK_ID),
        broadcast_common::timescale!(90_000),
        transmux::pipeline::CodecConfig::Avc {
            config: isobmff::avc_config::AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

/// An init segment whose `trex` really declares `size` as the per-sample
/// default (`build_init_segment` writes 0, the "not declared" value).
fn init_with_trex_sample_size(size: u32) -> Vec<u8> {
    let mut init =
        transmux::pipeline::build_init_segment(&[avc_spec()], broadcast_common::timescale!(90_000))
            .expect("the AVC init segment builds");
    let at = init
        .windows(4)
        .position(|w| w == b"trex")
        .expect("the init segment carries a trex");
    let field = at + 4 + TREX_DEFAULT_SAMPLE_SIZE_OFFSET;
    init[field..field + 4].copy_from_slice(&size.to_be_bytes());
    init
}

fn assemble_moof(data_offset: u32, per_sample_size: bool) -> Vec<u8> {
    let mfhd = boxed(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]);
    let mut tfhd_body = isobmff::movie_fragment::TFHD_DEFAULT_BASE_IS_MOOF
        .to_be_bytes()
        .to_vec();
    tfhd_body.extend_from_slice(&TRACK_ID.to_be_bytes());
    let mut flags = isobmff::movie_fragment::TRUN_DATA_OFFSET_PRESENT;
    if per_sample_size {
        flags |= isobmff::movie_fragment::TRUN_SAMPLE_SIZE_PRESENT;
    }
    let mut trun_body = flags.to_be_bytes().to_vec();
    trun_body.extend_from_slice(&SAMPLE_COUNT.to_be_bytes());
    trun_body.extend_from_slice(&data_offset.to_be_bytes());
    if per_sample_size {
        for _ in 0..SAMPLE_COUNT {
            trun_body.extend_from_slice(&8u32.to_be_bytes());
        }
    }
    let mut traf_body = boxed(b"tfhd", &tfhd_body);
    traf_body.extend_from_slice(&boxed(b"trun", &trun_body));
    [mfhd, boxed(b"traf", &traf_body)].concat()
}

/// The moof's `data_offset` (measured from the `moof` box's own first byte) is
/// the distance to the `mdat` payload: the `moof` box plus the `mdat` header.
/// Its value does not change the `moof`'s length, so build it once with 0 to
/// measure, then again with the real offset.
fn moof_and_mdat(per_sample_size: bool) -> (Vec<u8>, Vec<u8>) {
    let probe = assemble_moof(0, per_sample_size);
    let data_offset = (BOX_HEADER_LEN + probe.len() + BOX_HEADER_LEN) as u32;
    let moof = boxed(b"moof", &assemble_moof(data_offset, per_sample_size));
    let mdat = boxed(b"mdat", &vec![0u8; SAMPLE_COUNT as usize * BOX_HEADER_LEN]);
    (moof, mdat)
}

fn demux_assembled(trex_size: u32, per_sample_size: bool) -> transmux::Media {
    let mut file = init_with_trex_sample_size(trex_size);
    let (moof, mdat) = moof_and_mdat(per_sample_size);
    file.extend_from_slice(&moof);
    file.extend_from_slice(&mdat);
    test_harness::collect::media_from_mp4(
        file.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("a fragment whose sizes come from trex demuxes")
}

#[test]
fn a_trun_without_sizes_takes_the_trex_default_sample_size() {
    let media = demux_assembled(8, false);
    let track = &media.tracks[0];
    assert!(media.degraded.is_empty(), "degraded: {:?}", media.degraded);
    assert_eq!(track.samples.len(), SAMPLE_COUNT as usize);
    for (i, sample) in track.samples.iter().enumerate() {
        assert_eq!(sample.data.len(), 8, "sample {i}");
    }
}

#[test]
fn a_zero_trex_sample_size_is_not_applied_as_a_zero_length_sample() {
    // `trex` size 0 means "not declared": the per-sample `trun` sizes stand.
    let media = demux_assembled(0, true);
    let track = &media.tracks[0];
    assert_eq!(track.samples.len(), SAMPLE_COUNT as usize);
    for (i, sample) in track.samples.iter().enumerate() {
        assert_eq!(sample.data.len(), 8, "sample {i}");
    }
}
