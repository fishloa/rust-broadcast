//! Demux-side display geometry (Phase 4a-2): an fMP4 / MP4 sample entry's
//! `pasp` and `colr nclx` are applied to `TrackSpec::display`.

use broadcast_common::Package;
use broadcast_common::Unpackage;
use broadcast_common::timescale::Timescale;
use transmux::TrackSpec;
use transmux::ir::{
    AspectRatioIdc, CodecConfig, ColourDescription, DisplayGeometry, VideoFormat, VideoSignalType,
};
use transmux::media::Fmp4Demux;
use transmux::pipeline;
use transmux::ts_demux::TsDemux;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn avc_spec(fixture: &str) -> Option<TrackSpec> {
    let data = test_fixtures::require_bytes(fixture, "display_demux")?;
    let media = TsDemux::new().demux(&data).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec)
}

fn dims(spec: &TrackSpec) -> (u32, u32) {
    match &spec.config {
        CodecConfig::Avc { width, height, .. } => (*width as u32, *height as u32),
        CodecConfig::Hevc { width, height, .. } => (*width as u32, *height as u32),
        _ => (0, 0),
    }
}

fn demux_display(bytes: &[u8]) -> Option<DisplayGeometry> {
    let mut d = Fmp4Demux::new();
    let media = d.unpackage(bytes).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))?
        .spec
        .display
}

fn build(spec: TrackSpec) -> Vec<u8> {
    pipeline::build_init_segment(&[spec], timescale(90000)).expect("build_init_segment")
}
#[test]
fn pasp_fills_unspecified_sar() {
    let Some(mut spec) = avc_spec("suites/tsduck/input/test-062.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

/// ISO/IEC 14496-12:2026 §12.1.4.1: `pasp`, when present, over-rides the
/// declarations in the video codec's own structures. This fixture's SPS VUI
/// carries 16:11 and the sample entry is built with a `pasp` of 64:45.
#[test]
fn pasp_conflict_overrides_vui_sar() {
    let Some(mut spec) = avc_spec("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

#[test]
fn pasp_agreement_unchanged() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

#[test]
fn colr_overrides_colour() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let mut g = DisplayGeometry::new(w, h, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        true,
        Some(ColourDescription::from_wire(1, 13, 6)),
    ));
    spec = spec.with_display(Some(g));
    let bytes = build(spec);
    let got = demux_display(&bytes).expect("display");
    let vs = got.video_signal.expect("video_signal");
    assert!(vs.full_range);
    let c = vs.colour.expect("colour");
    assert_eq!(c.primaries.code(), 1);
    assert_eq!(c.transfer.code(), 13);
    assert_eq!(c.matrix.code(), 6);
}

#[test]
fn colr_codepoint_over_255_ignored() {
    let Some(mut spec) = avc_spec("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (w, h) = dims(&spec);
    let mut g = DisplayGeometry::new(w, h, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Unspecified,
        true,
        Some(ColourDescription::from_wire(1, 13, 6)),
    ));
    spec = spec.with_display(Some(g));
    let mut bytes = build(spec);
    // patch colr primaries to 257 (>255)
    let i = bytes.windows(4).position(|w| w == b"colr").expect("colr");
    bytes[i + 8] = 0x01;
    bytes[i + 9] = 0x01;
    let got = demux_display(&bytes).expect("display");
    let vs = got.video_signal.expect("video_signal");
    let c = vs.colour.expect("colour");
    // VUI colour 5/5/5 kept
    assert_eq!(c.primaries.code(), 5);
    assert_eq!(c.transfer.code(), 5);
    assert_eq!(c.matrix.code(), 5);
}

#[test]
fn neither_box_display_unchanged() {
    let data = match test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_demux") {
        Some(d) => d,
        None => return,
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc");
    let orig = track.spec.display.clone();
    let spec = track.spec.clone();
    let ts = spec.timescale;
    let bytes = pipeline::build_init_segment(&[spec], ts).expect("build");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(Some(got), orig);
}

#[test]
fn display_none_with_pasp_creates_display() {
    let data = match test_fixtures::require_bytes("ts/gulli-opengop.ts", "display_demux") {
        Some(d) => d,
        None => return,
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("avc");
    let mut spec = track.spec.clone();
    // corrupt the SPS so TrackSpec::new yields no display
    if let CodecConfig::Avc { config, .. } = &mut spec.config
        && let Some(sps) = config.config.sps.first_mut()
    {
        sps.0.fill(0);
    }
    let (w, h) = dims(&spec);
    let g = DisplayGeometry::new(w, h, Some(AspectRatioIdc::from_wire(255, Some((64, 45)))));
    spec = spec.with_display(Some(g));
    let ts = spec.timescale;
    let bytes = pipeline::build_init_segment(&[spec], ts).expect("build");
    let got = demux_display(&bytes).expect("display");
    assert_eq!(got.coded_width, w);
    assert_eq!(got.coded_height, h);
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
}

/// Environment variable CI sets in the job that installs `ffmpeg`: a missing
/// ffmpeg is then a hard failure, never a skip.
const REQUIRE_FFMPEG_ENV: &str = "REQUIRE_FFMPEG";

/// ffmpeg's OWN remux of a private capture (its anamorphic SAR and colour land
/// in `pasp`/`colr`) demuxed back through our fMP4 reader.
///
/// `None` on a loud skip: no private checkout, or no ffmpeg outside the job
/// that installs it.
fn ffmpeg_remux_display(fixture_rel: &str, tag: &str) -> Option<DisplayGeometry> {
    let fixture = test_fixtures::require(fixture_rel, "display_demux")?;
    if !test_fixtures::process::tool_available("ffmpeg", "-version") {
        assert!(
            std::env::var(REQUIRE_FFMPEG_ENV).is_err(),
            "{REQUIRE_FFMPEG_ENV}=1 and ffmpeg is not on PATH: a missing ffmpeg is a failure \
             here, not a skip"
        );
        eprintln!(
            "SKIP display_demux: ffmpeg is not on PATH (set {REQUIRE_FFMPEG_ENV}=1 to make this \
             a failure)"
        );
        return None;
    }
    let tmp = std::env::temp_dir().join(format!("display_demux_{tag}_{}.mp4", std::process::id()));
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&fixture)
        .args([
            "-t",
            "3",
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+write_colr",
        ])
        .arg(&tmp)
        .status()
        .expect("ffmpeg");
    assert!(
        status.success(),
        "{fixture_rel}: ffmpeg could not remux the capture"
    );
    let bytes = std::fs::read(&tmp).expect("read mp4");
    let _ = std::fs::remove_file(&tmp);
    Some(demux_display(&bytes).expect("display"))
}

#[test]
fn ffmpeg_melodie_pasp_colr() {
    let Some(got) = ffmpeg_remux_display("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts", "melodie")
    else {
        return;
    };
    assert_eq!(got.sample_aspect_ratio(), Some((64u16, 45u16)));
    let vs = got.video_signal.expect("video_signal");
    assert!(!vs.full_range);
    let c = vs.colour.expect("colour");
    assert_eq!(c.primaries.code(), 5);
    assert_eq!(c.transfer.code(), 5);
    assert_eq!(c.matrix.code(), 5);
}

#[test]
fn ffmpeg_krone_pasp() {
    let Some(got) = ffmpeg_remux_display("ts/krone-anamorphic-sar-16-11-zenith-tv.ts", "krone")
    else {
        return;
    };
    assert_eq!(got.sample_aspect_ratio(), Some((16u16, 11u16)));
}

// ---------------------------------------------------------------------------
// Promotion (audit 23 finding 240): the `TrackSpec` geometry is authoritative
// ---------------------------------------------------------------------------

/// A `mb` x `mb` macroblock H.264 SPS (16 luma samples per MB), no crop, SAR
/// `idc` (1 = 1:1).
fn avc_sps_nal(sps_id: u32, mb: u32, idc: u32) -> Vec<u8> {
    let mut w = crate::bit_writer::BitWriter::new();
    w.ue(u64::from(sps_id)); // seq_parameter_set_id
    w.ue(0); // log2_max_frame_num_minus4
    w.ue(2); // pic_order_cnt_type
    w.ue(1); // max_num_ref_frames
    w.bit(false); // gaps_in_frame_num_value_allowed_flag
    w.ue(u64::from(mb - 1)); // pic_width_in_mbs_minus1
    w.ue(u64::from(mb - 1)); // pic_height_in_map_units_minus1
    w.bit(true); // frame_mbs_only_flag
    w.bit(true); // direct_8x8_inference_flag
    w.bit(false); // frame_cropping_flag
    w.bit(true); // vui_parameters_present_flag
    w.bit(true); // aspect_ratio_info_present_flag
    w.u(8, u64::from(idc)); // aspect_ratio_idc
    w.bit(false); // overscan_info_present_flag
    w.bit(true); // video_signal_type_present_flag
    w.u(3, 5); // video_format
    w.bit(false); // video_full_range_flag
    w.bit(false); // colour_description_present_flag
    let mut v = vec![0x67u8, 66, 0x00, 30];
    v.extend(w.trailing());
    v
}

fn avc_pps_nal(pps_id: u32, sps_id: u32) -> Vec<u8> {
    let mut w = crate::bit_writer::BitWriter::new();
    w.ue(u64::from(pps_id)); // pic_parameter_set_id
    w.ue(u64::from(sps_id)); // seq_parameter_set_id
    w.bit(false); // entropy_coding_mode_flag
    w.bit(false); // bottom_field_pic_order_in_frame_present_flag
    w.ue(0); // num_slice_groups_minus1
    w.ue(0); // num_ref_idx_l0_default_active_minus1
    w.ue(0); // num_ref_idx_l1_default_active_minus1
    w.bit(false); // weighted_pred_flag
    w.u(2, 0); // weighted_bipred_idc
    w.se(0); // pic_init_qp_minus26
    w.se(0); // pic_init_qs_minus26
    w.se(0); // chroma_qp_index_offset
    w.bit(false); // deblocking_filter_control_present_flag
    w.bit(false); // constrained_intra_pred_flag
    w.bit(false); // redundant_pic_cnt_present_flag
    let mut v = vec![0x68u8];
    v.extend(w.trailing());
    v
}

fn avc_idr_slice_nal(pps_id: u32) -> Vec<u8> {
    let mut w = crate::bit_writer::BitWriter::new();
    w.ue(0); // first_mb_in_slice
    w.ue(2); // slice_type = I
    w.ue(u64::from(pps_id)); // pic_parameter_set_id
    w.ue(0); // frame_num
    w.ue(0); // idr_pic_id
    let mut v = vec![0x65u8];
    v.extend(w.trailing());
    v
}

fn annexb_access_unit(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    out
}

/// An AVC track spec from the committed `ts/h264-aac.ts` fixture (its real
/// codec config), so only the samples are synthetic.
fn avc_track_spec() -> Option<TrackSpec> {
    let data = test_fixtures::require_bytes("ts/h264-aac.ts", "display_demux")?;
    let media = TsDemux::new().demux(&data).ok()?;
    media
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .map(|t| t.spec)
}

/// Mux AVC access units (Annex B) into a one-track TS with `TsMux`.
fn avc_ts(spec: TrackSpec, access_units: &[Vec<u8>]) -> Vec<u8> {
    let samples: Vec<transmux::Sample> = access_units
        .iter()
        .enumerate()
        .map(|(i, au)| {
            let dts = (i as i64) * 3000;
            transmux::Sample::from_annexb(au, Some(dts), Some(dts), Some(3000), true)
                .expect("valid timing")
        })
        .collect();
    let track = transmux::Track::new(spec, samples);
    transmux::TsMux::new()
        .package(&transmux::Media::new(vec![track], timescale(90000)))
        .expect("mux the synthetic TS")
}

/// Every video `DisplayChanged`, as `(initial, coded width)`.
fn display_changes(ts: &[u8]) -> Vec<(bool, Option<u32>)> {
    let mut demux = transmux::StreamingTsDemux::new();
    demux.feed(ts).expect("feed");
    demux.finish().expect("finish");
    let mut out = Vec::new();
    while let Some(ev) = demux.poll_event() {
        if let transmux::DemuxEvent::DisplayChanged {
            display, changed, ..
        } = ev
        {
            out.push((
                changed.initial,
                display.geometry.as_ref().map(|g| g.coded_width),
            ));
        }
    }
    out
}

/// The backlog can leave the tracker on a different SPS than the one the
/// codec config was recovered from (here one access unit carries two SPS with
/// the same id, and the probe keeps the first). `TrackSpec`'s geometry is
/// authoritative for the initial event, and the switch to the second SPS is
/// still reported for the access unit that activates it.
#[test]
fn promotion_restores_the_track_spec_geometry() {
    let Some(spec) = avc_track_spec() else {
        return;
    };
    let first = annexb_access_unit(&[
        avc_sps_nal(0, 8, 1),
        avc_sps_nal(0, 16, 1),
        avc_pps_nal(0, 0),
        avc_idr_slice_nal(0),
    ]);
    let second = annexb_access_unit(&[
        avc_sps_nal(0, 16, 1),
        avc_pps_nal(0, 0),
        avc_idr_slice_nal(0),
    ]);
    let ts = avc_ts(spec, &[first, second.clone(), second]);
    let changes = display_changes(&ts);
    assert_eq!(
        changes,
        vec![(true, Some(128)), (false, Some(256))],
        "the initial event carries the TrackSpec geometry, then the switch"
    );
}

/// The fixture's own SPS and PPS bytes (its real 320-pixel geometry).
fn avc_fixture_sets(spec: &TrackSpec) -> Option<(Vec<u8>, Vec<u8>)> {
    let CodecConfig::Avc { config, .. } = &spec.config else {
        return None;
    };
    let sps = config.config.sps.first()?.0.to_vec();
    let pps = config.config.pps.first()?.0.to_vec();
    Some((sps, pps))
}

/// A slice whose PPS has not arrived holds the state; the access unit that
/// carries the sets reports the change once, and the picture after it on the
/// same sets reports nothing.
#[test]
fn an_unseen_pps_is_reported_once_when_its_sets_arrive() {
    let Some(spec) = avc_track_spec() else {
        return;
    };
    let Some((fixture_sps, fixture_pps)) = avc_fixture_sets(&spec) else {
        return;
    };
    let dangling = annexb_access_unit(&[fixture_sps, fixture_pps, avc_idr_slice_nal(1)]);
    let resolving = annexb_access_unit(&[
        avc_sps_nal(0, 8, 1),
        avc_pps_nal(1, 0),
        avc_idr_slice_nal(1),
    ]);
    let settled = annexb_access_unit(&[
        avc_sps_nal(0, 8, 1),
        avc_pps_nal(1, 0),
        avc_idr_slice_nal(1),
    ]);
    let ts = avc_ts(spec, &[dangling, resolving, settled]);
    let changes = display_changes(&ts);
    assert_eq!(
        changes,
        vec![(true, Some(320)), (false, Some(128))],
        "one change, at the access unit carrying the sets"
    );
}
