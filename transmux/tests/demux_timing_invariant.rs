//! The per-timed-track dts/duration invariant, checked across EVERY demuxer
//! in this crate (media plane step-2 fix wave 1, FIX C).
//!
//! The aggregate review of the whole step-2 range found three defects
//! (B5/S2/S3) that a single, precise invariant would have caught immediately
//! — audio dts re-anchored per PES-boundary rather than advancing by the
//! intrinsic per-frame duration (B5), and `Fmp4Demux`/`ProgressiveDemux`
//! diverging on whether one unmodelled track fails the whole file (S2/S3).
//! Rather than one ad hoc regression test per bug, this file asserts the
//! underlying invariant every demuxer must hold for every *timed* track it
//! produces (an untimed, section-carried track — `dts.is_none()` — is
//! explicitly out of scope and skipped):
//!
//! 1. `Track::start_decode_time == samples[0].dts.unwrap()` — the anchor a
//!    demuxer publishes must be exactly the first sample's own dts, not an
//!    independently-rounded approximation of it.
//! 2. `sum(sample.duration) == dts_last - dts_first + duration_last` — the
//!    duration timeline and the dts timeline must agree: walking the samples
//!    by summing durations must land on exactly the same place the
//!    demuxer's own absolute dts sequence does. A demuxer that re-derives
//!    dts from a lossy wire clock independently of the intrinsic per-sample
//!    duration (the B5 bug) breaks this the moment two ticks disagree by
//!    even 1; a demuxer that fails outright on one unmodelled track (S2)
//!    never gets far enough to produce the track at all — either failure
//!    mode is caught here, on real fixtures, for every demuxer in the crate.
//!
//! Every fixture below is a real capture/encode already committed to the
//! workspace (never fabricated bytes) — the same ones `tests/absolute_timing.rs`
//! and each demuxer's own dedicated test file already use.

use broadcast_common::{Package, Unpackage};

use transmux::rtp::{RtpInput, RtpInputStream, RtpMediaKind};
use transmux::{
    FlvDemux, Fmp4Demux, Media, ProgressiveDemux, PsDemux, RtmpDemux, RtmpMux, RtpDepacketiser,
    RtpPacketiser, TsDemux, WebmDemux,
};

fn workspace_fixture(rel: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// The invariant itself, checked for every timed track in `media`. `what`
/// names the demuxer for a legible failure message. Returns the number of
/// tracks actually checked (so callers can assert at least one track — and
/// at least one sample — was exercised, guarding against the assertions
/// vacuously passing over an empty/all-skipped `Media`).
fn assert_timing_invariant(media: &Media, what: &str) -> usize {
    let mut checked = 0usize;
    for (ti, t) in media.tracks.iter().enumerate() {
        if t.samples.is_empty() {
            continue;
        }
        let Some(dts_first) = t.samples[0].dts else {
            // Untimed / section-carried track (e.g. SCTE-35/DSM-CC/private
            // sections) — explicitly out of scope, never fabricated.
            continue;
        };
        checked += 1;

        assert_eq!(
            t.start_decode_time as i64, dts_first,
            "{what}: track {ti} Track::start_decode_time ({}) must equal \
             samples[0].dts ({dts_first})",
            t.start_decode_time
        );

        let last = t.samples.last().expect("just checked non-empty");
        let dts_last = last
            .dts
            .unwrap_or_else(|| panic!("{what}: track {ti} first sample is timed but last isn't"));
        let duration_last = last.duration.unwrap_or(0) as i64;

        let sum_duration: i64 = t
            .samples
            .iter()
            .map(|s| {
                s.duration
                    .unwrap_or_else(|| panic!("{what}: track {ti} a timed sample has no duration"))
                    as i64
            })
            .sum();

        assert_eq!(
            sum_duration,
            dts_last - dts_first + duration_last,
            "{what}: track {ti} duration timeline disagrees with the dts timeline \
             (sum(duration)={sum_duration}, dts_last-dts_first+duration_last={}); \
             dts_first={dts_first} dts_last={dts_last} duration_last={duration_last}",
            dts_last - dts_first + duration_last
        );
    }
    checked
}

// ── 1. TsDemux ───────────────────────────────────────────────────────────

#[test]
fn ts_demux_timing_invariant_holds() {
    let ts = workspace_fixture("ts/h264_aac.ts");
    let media = TsDemux::new().unpackage(&ts[..]).expect("TS demux");
    let checked = assert_timing_invariant(&media, "TsDemux");
    assert!(
        checked >= 2,
        "expected at least video + audio tracks checked, got {checked}"
    );
}

// ── 2. Fmp4Demux ─────────────────────────────────────────────────────────

#[test]
fn fmp4_demux_timing_invariant_holds() {
    let file = workspace_fixture("mp4/frag/h264_high.frag.mp4");
    let media = Fmp4Demux::new().unpackage(&file).expect("fMP4 demux");
    let checked = assert_timing_invariant(&media, "Fmp4Demux");
    assert!(checked >= 1, "expected at least one timed track checked");
}

// ── 3. ProgressiveDemux ──────────────────────────────────────────────────

#[test]
fn progressive_demux_timing_invariant_holds() {
    let file = workspace_fixture("transmux/h264_aac_prog.mp4");
    let media = ProgressiveDemux::new()
        .unpackage(&file)
        .expect("progressive MP4 demux");
    let checked = assert_timing_invariant(&media, "ProgressiveDemux");
    assert!(
        checked >= 2,
        "expected at least video + audio tracks checked, got {checked}"
    );
}

// ── 4. FlvDemux ───────────────────────────────────────────────────────────

#[test]
fn flv_demux_timing_invariant_holds() {
    let flv = workspace_fixture("flv/av.flv");
    let media = FlvDemux::new().unpackage(&flv[..]).expect("FLV demux");
    let checked = assert_timing_invariant(&media, "FlvDemux");
    assert!(checked >= 1, "expected at least one timed track checked");
}

// ── 5. WebmDemux ──────────────────────────────────────────────────────────

#[test]
fn webm_demux_timing_invariant_holds() {
    let webm = workspace_fixture("webm/vp9_opus.webm");
    let media = WebmDemux::new().unpackage(&webm[..]).expect("WebM demux");
    let checked = assert_timing_invariant(&media, "WebmDemux");
    assert!(
        checked >= 2,
        "expected at least video + audio tracks checked, got {checked}"
    );
}

// ── 6. PsDemux ────────────────────────────────────────────────────────────

#[test]
fn ps_demux_timing_invariant_holds() {
    let ps = workspace_fixture("ps/h264_ac3.ps");
    let media = PsDemux::new().unpackage(&ps[..]).expect("PS demux");
    let checked = assert_timing_invariant(&media, "PsDemux");
    assert!(
        checked >= 2,
        "expected at least video + audio tracks checked, got {checked}"
    );
}

// ── 7. RtmpDemux (via a real FLV muxed to RTMP wire then demuxed back) ───

#[test]
fn rtmp_demux_timing_invariant_holds() {
    let flv = workspace_fixture("flv/av.flv");
    let ir = FlvDemux::new().unpackage(&flv[..]).expect("FLV demux");
    let wire = RtmpMux::default().package(&ir).expect("IR -> RTMP wire");
    let media = RtmpDemux::new()
        .unpackage(&wire[..])
        .expect("RTMP wire -> IR");
    let checked = assert_timing_invariant(&media, "RtmpDemux");
    assert!(checked >= 1, "expected at least one timed track checked");
}

// ── 8. RTP (via a real TS packetised to RTP then depacketised back) ──────

#[test]
fn rtp_depacketiser_timing_invariant_holds() {
    let ts = workspace_fixture("ts/h264_aac.ts");
    let ir = TsDemux::new().unpackage(&ts[..]).expect("TS demux");
    let out = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    }
    .package(&ir)
    .expect("IR -> RTP");
    let media = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: out
                .streams
                .iter()
                .map(|s| RtpInputStream {
                    kind: s.kind,
                    packets: s.packets.clone(),
                })
                .collect(),
        })
        .expect("RTP -> IR");
    assert!(
        out.streams
            .iter()
            .any(|s| matches!(s.kind, RtpMediaKind::H264)),
        "sanity: an H.264 RTP stream must have been produced"
    );
    let checked = assert_timing_invariant(&media, "RtpDepacketiser");
    assert!(
        checked >= 2,
        "expected at least video + audio tracks checked, got {checked}"
    );
}
