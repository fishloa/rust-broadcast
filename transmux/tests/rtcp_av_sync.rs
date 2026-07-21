//! Deterministic bite gate for RTCP SR wallclock A/V-sync (issue #722).
//!
//! Builds a synthetic video (90 kHz) + audio (48 kHz) RTP pair whose RTP
//! timestamps use DIFFERENT arbitrary bases (video 1,000,000; audio
//! 7,000,000) *and* whose real-world capture start times differ by
//! [`AUDIO_START_DELAY_S`] — the case RTP timestamps alone cannot recover,
//! because each SSRC's RTP clock has an arbitrary, unrelated origin (RFC 3550
//! §5.1). One "marker" access unit per track is engineered to land at the
//! same real wallclock instant.
//!
//! Two identically-fed [`RtpStreamDepacketiser`]s are compared: one fed the
//! RTCP Sender Reports that anchor each track's RTP clock to NTP wallclock
//! (RFC 3550 §6.4.1), one not. The marker offset must be sub-millisecond
//! WITH the reports and materially wrong (≈ [`AUDIO_START_DELAY_S`]) WITHOUT
//! them — proving `push_sender_report`/`sync_start_decode_times` actually
//! changes the recovered alignment rather than being inert.
#![cfg(feature = "std")]

use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::pipeline::{CodecConfig, Sample};
use transmux::rtcp::SenderReport;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::aac_config_from_asc_hex;
use transmux::{RtpStreamDepacketiser, RtpStreamTrack};

const VIDEO_TRACK: u32 = 1;
const AUDIO_TRACK: u32 = 2;

const VIDEO_CLOCK: u32 = 90_000;
const AUDIO_CLOCK: u32 = 48_000;

// Deliberately different arbitrary RTP timestamp bases (RFC 3550 §5.1: each
// SSRC picks a random initial timestamp with no relation to any other
// stream's).
const VIDEO_TS_BASE: u32 = 1_000_000;
const AUDIO_TS_BASE: u32 = 7_000_000;

const VIDEO_FRAME_TICKS: u32 = 4_500; // 50 ms @ 90 kHz (20 fps)
const AUDIO_FRAME_TICKS: u32 = 1_200; // 25 ms @ 48 kHz

const NUM_VIDEO_AUS: u32 = 25;
const NUM_AUDIO_AUS: u32 = 8;

/// The video AU whose start instant (`22 * 50ms` = 1.1s after the video
/// track's own first sample) is engineered to coincide with the audio
/// marker's real wallclock instant.
const MARKER_VIDEO_AU: usize = 22;
/// The audio AU whose start instant (`4 * 25ms` = 0.1s after the audio
/// track's own first sample, i.e. `AUDIO_START_DELAY_S + 0.1s` in the shared
/// real-world timeline) is the marker's audio half.
const MARKER_AUDIO_AU: usize = 4;

/// Arbitrary NTP-epoch wallclock instant (seconds) of the video track's first
/// sample. A non-integer value exercises the 32.32 fixed-point NTP
/// conversion's fractional half.
const VIDEO_START_WALL: f64 = 12_345.5;
/// How much later, in real wallclock seconds, the audio track's capture
/// actually started — the cross-track offset only RTCP SR correlation can
/// recover; RTP timestamps carry no information about it at all.
const AUDIO_START_DELAY_S: f64 = 1.0;

const ONE_MS: f64 = 0.001;

fn dummy_avc() -> CodecConfig {
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0x42,
            profile_compatibility: 0,
            level_indication: 0x1E,
            length_size_minus_one: 3,
            sps: Vec::new(),
            pps: Vec::new(),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: Vec::new(),
        }),
        width: 0,
        height: 0,
    }
}

/// 48 kHz stereo AAC-LC `AudioSpecificConfig` (`0x11 0x90`: audioObjectType=2,
/// samplingFrequencyIndex=3, channelConfiguration=2 — ISO/IEC 14496-3 §1.6.2.1
/// Table 1.8).
fn dummy_aac() -> CodecConfig {
    aac_config_from_asc_hex("1190").expect("valid 48 kHz stereo AAC-LC ASC")
}

/// One single-NAL H.264 AU as a bare RTP packet (marker always set, matching
/// `RtpStreamDepacketiser::push`'s immediate-drain-on-marker path for
/// [`RtpMediaKind::H264`]).
fn video_packet(seq: u16, ts: u32) -> Vec<u8> {
    const PT: u8 = 96;
    const IDR_NAL: [u8; 2] = [0x65, 0xAA];
    let mut p = vec![0x80u8, 0x80 | PT];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0x11, 0x11, 0x11, 0x11]);
    p.extend_from_slice(&IDR_NAL);
    p
}

/// One `AAC-hbr` (RFC 3640 §3.3.6) AU as a bare RTP packet: 2-byte
/// AU-headers-length (16 bits = one header) + 2-byte AU-header
/// (`AU-size(13) | AU-Index(3)=0`) + the AU payload.
fn audio_packet(seq: u16, ts: u32) -> Vec<u8> {
    const PT: u8 = 97;
    const AU_HEADERS_LEN_BITS: u16 = 16;
    const AU_INDEX_LENGTH: u16 = 3;
    const AU_DATA: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];
    let mut p = vec![0x80u8, 0x80 | PT];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0x22, 0x22, 0x22, 0x22]);
    p.extend_from_slice(&AU_HEADERS_LEN_BITS.to_be_bytes());
    let au_header = (AU_DATA.len() as u16) << AU_INDEX_LENGTH;
    p.extend_from_slice(&au_header.to_be_bytes());
    p.extend_from_slice(&AU_DATA);
    p
}

/// Convert a wallclock instant (seconds since an arbitrary epoch) to the
/// 32.32 fixed-point NTP timestamp an RTCP SR carries (RFC 3550 §6.4.1).
fn to_ntp(wall_seconds: f64) -> (u32, u32) {
    let msw = wall_seconds.floor();
    let frac = wall_seconds - msw;
    (msw as u32, (frac * 4_294_967_296.0).round() as u32)
}

/// Build a minimal Sender Report anchoring `rtp_timestamp` to `wall_seconds`
/// (the report blocks / packet+octet counts are irrelevant to sync and left
/// zeroed).
fn sender_report(rtp_timestamp: u32, wall_seconds: f64) -> SenderReport {
    let (ntp_msw, ntp_lsw) = to_ntp(wall_seconds);
    SenderReport {
        ssrc: 0,
        ntp_msw,
        ntp_lsw,
        rtp_timestamp,
        packet_count: 0,
        octet_count: 0,
        report_blocks: Vec::new(),
    }
}

/// Feed `count` uniformly RTP-timestamp-spaced AUs for one track and return
/// every recovered [`Sample`] (the `push` results plus the final `flush`).
fn feed_track(
    d: &mut RtpStreamDepacketiser,
    track_id: u32,
    base_ts: u32,
    frame_ticks: u32,
    count: u32,
    build_pkt: fn(u16, u32) -> Vec<u8>,
) -> Vec<Sample> {
    let mut out = Vec::new();
    for i in 0..count {
        let ts = base_ts.wrapping_add(i * frame_ticks);
        out.extend(d.push(track_id, &build_pkt(i as u16, ts)).unwrap());
    }
    out.extend(d.flush(track_id).unwrap());
    out
}

/// The wallclock instant (seconds) of `samples[marker_index]`, reconstructed
/// exactly as a real muxer would: `start_decode_time` (the track's tfdt
/// anchor — `0` in the v1/no-SR case) plus the running sum of every
/// preceding sample's duration, all divided by the track's clock rate.
fn marker_wall_seconds(
    samples: &[Sample],
    marker_index: usize,
    start_decode_time: u64,
    clock_rate: u32,
) -> f64 {
    let cumulative: u64 = samples[..marker_index]
        .iter()
        .map(|s| u64::from(s.duration))
        .sum();
    (start_decode_time + cumulative) as f64 / f64::from(clock_rate)
}

fn tracks() -> Vec<RtpStreamTrack> {
    vec![
        RtpStreamTrack::new(VIDEO_TRACK, RtpMediaKind::H264, dummy_avc(), VIDEO_CLOCK),
        RtpStreamTrack::new(AUDIO_TRACK, RtpMediaKind::Aac, dummy_aac(), AUDIO_CLOCK),
    ]
}

#[test]
fn rtcp_sr_recovers_true_av_sync_negative_control_without_sr_is_wrong() {
    // ---- WITH SR ------------------------------------------------------
    let mut with_sr = RtpStreamDepacketiser::new(tracks());
    let video_samples = feed_track(
        &mut with_sr,
        VIDEO_TRACK,
        VIDEO_TS_BASE,
        VIDEO_FRAME_TICKS,
        NUM_VIDEO_AUS,
        video_packet,
    );
    let audio_samples = feed_track(
        &mut with_sr,
        AUDIO_TRACK,
        AUDIO_TS_BASE,
        AUDIO_FRAME_TICKS,
        NUM_AUDIO_AUS,
        audio_packet,
    );
    assert!(
        video_samples.len() > MARKER_VIDEO_AU,
        "need the marker video AU to be recovered"
    );
    assert!(
        audio_samples.len() > MARKER_AUDIO_AU,
        "need the marker audio AU to be recovered"
    );

    let audio_start_wall = VIDEO_START_WALL + AUDIO_START_DELAY_S;
    with_sr.push_sender_report(VIDEO_TRACK, sender_report(VIDEO_TS_BASE, VIDEO_START_WALL));
    with_sr.push_sender_report(AUDIO_TRACK, sender_report(AUDIO_TS_BASE, audio_start_wall));

    let starts = with_sr.sync_start_decode_times();
    assert_eq!(
        starts.len(),
        2,
        "both tracks anchored -> both get a common-wallclock start_decode_time"
    );
    let video_start = starts.iter().find(|(id, _)| *id == VIDEO_TRACK).unwrap().1;
    let audio_start = starts.iter().find(|(id, _)| *id == AUDIO_TRACK).unwrap().1;
    // The video track began recording first, so it defines the common
    // origin (start_decode_time == 0); audio's start is pushed forward by
    // ~AUDIO_START_DELAY_S, expressed in audio's own 48 kHz clock.
    assert_eq!(video_start, 0, "earliest anchored track is the origin");
    assert!(
        (audio_start as f64 / f64::from(AUDIO_CLOCK) - AUDIO_START_DELAY_S).abs() < ONE_MS,
        "audio start_decode_time must reflect the real ~{AUDIO_START_DELAY_S}s capture-start delay"
    );

    let video_marker_wall =
        marker_wall_seconds(&video_samples, MARKER_VIDEO_AU, video_start, VIDEO_CLOCK);
    let audio_marker_wall =
        marker_wall_seconds(&audio_samples, MARKER_AUDIO_AU, audio_start, AUDIO_CLOCK);
    let offset_with_sr = (video_marker_wall - audio_marker_wall).abs();

    // ---- WITHOUT SR (negative control) ---------------------------------
    // A second, freshly-built depacketiser fed byte-identical packets, but
    // never given a Sender Report: `sync_start_decode_times` must opt out
    // (empty `Vec`), and reconstructing the marker offset with the v1
    // independent-rebase-to-0 model (`start_decode_time = 0` for both
    // tracks) must reproduce the real ~1s misalignment RTP timestamps alone
    // cannot see.
    let mut without_sr = RtpStreamDepacketiser::new(tracks());
    let video_samples_v1 = feed_track(
        &mut without_sr,
        VIDEO_TRACK,
        VIDEO_TS_BASE,
        VIDEO_FRAME_TICKS,
        NUM_VIDEO_AUS,
        video_packet,
    );
    let audio_samples_v1 = feed_track(
        &mut without_sr,
        AUDIO_TRACK,
        AUDIO_TS_BASE,
        AUDIO_FRAME_TICKS,
        NUM_AUDIO_AUS,
        audio_packet,
    );
    assert!(
        without_sr.sync_start_decode_times().is_empty(),
        "no SR fed -> v1 opt-out, empty Vec"
    );

    let video_marker_wall_v1 =
        marker_wall_seconds(&video_samples_v1, MARKER_VIDEO_AU, 0, VIDEO_CLOCK);
    let audio_marker_wall_v1 =
        marker_wall_seconds(&audio_samples_v1, MARKER_AUDIO_AU, 0, AUDIO_CLOCK);
    let offset_without_sr = (video_marker_wall_v1 - audio_marker_wall_v1).abs();

    eprintln!("marker offset WITH SR:    {offset_with_sr:.6} s (must be < {ONE_MS} s)");
    eprintln!(
        "marker offset WITHOUT SR: {offset_without_sr:.6} s (must be ~= {AUDIO_START_DELAY_S} s, materially wrong)"
    );

    assert!(
        offset_with_sr < ONE_MS,
        "SR-synced marker offset must be sub-millisecond, got {offset_with_sr}s"
    );
    // Bite proof: independent rebase-to-0 is NOT inert here — it is wrong by
    // essentially the full real inter-track start-time delta.
    assert!(
        offset_without_sr > 0.5,
        "independent rebase-to-0 must be materially wrong (not inert), got {offset_without_sr}s"
    );
    assert!(
        (offset_without_sr - AUDIO_START_DELAY_S).abs() < ONE_MS,
        "the v1 misalignment should equal the real inter-track start-time delta, got {offset_without_sr}s"
    );
}
