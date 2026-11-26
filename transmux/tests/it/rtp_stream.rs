//! Real-fixture gate for the streaming RTP depayloader (#700).
//!
//! Demuxes the real `h264-aac.ts` fixture, packetises it to RTP (the same
//! `TsDemux` + `RtpPacketiserStage` calls `tests/it/rtp.rs` uses), then feeds
//! the video stream's packets through [`RtpStreamDepacketiser`] fed with the
//! codec config recovered from the *generated SDP* (exercising the P2
//! `avc_config_from_sprop`/`aac_config_from_asc_hex` round-trip), and checks the
//! recovered timing/config/sync against the demuxed oracle, then builds a
//! valid fMP4 init + media segment from the recovered samples.
//!
//! Also gates loss/reorder detection (issue #779): §"Loss/reorder gate"
//! below drops/reorders/duplicates real packets from the same fixture-driven
//! `video_stream.packets` list (real NAL content, synthetically perturbed
//! delivery — there is no committed raw RTP capture to drop a packet from,
//! and even one would only ever be perturbed the same way for this kind of
//! test) plus two small hand-built streams (PROVENANCE noted at each) for
//! the sequence-wrap and reorder-buffer-bound cases, which need sequence
//! numbers no real short fixture naturally produces.
#![cfg(feature = "std")]

use crate::sdp_session;
fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

#[cfg(feature = "std")]
use broadcast_common::timescale::Timescale;
use broadcast_common::{Demand, Stage};
use std::collections::HashSet;
use test_harness::collect::Collected as Media;
use transmux::TrackId;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::{aac_config_from_asc_hex, avc_config_from_sprop};
use transmux::{
    FragmentTrackData, RtpLossEvent, RtpStreamDepacketiser, RtpStreamTrack, Severity,
    WallSkewExceeded, build_init_segment, build_media_segment, validate::SegmentOrigin,
    validate_init_segment, validate_media_segment,
};

const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;

// ── Step 0 plumbing, copied verbatim from tests/it/rtp.rs ──────────────────────

fn demux_fixture() -> Media {
    let path = test_fixtures::path("ts/h264-aac.ts");
    let data = std::fs::read(path).expect("h264-aac.ts fixture must exist");

    test_harness::collect::media_from_ts(&data[..]).expect("demux TS → IR")
}

fn packetise(media: &Media) -> crate::rtp_run::RtpOutput {
    crate::rtp_run::packetise(media, crate::rtp_run::stage(MTU, SSRC))
}

fn video_stream(out: &crate::rtp_run::RtpOutput) -> &crate::rtp_run::RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap()
}

fn audio_stream(out: &crate::rtp_run::RtpOutput) -> &crate::rtp_run::RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .unwrap()
}

// Pull one fmtp attribute value ("sprop-parameter-sets=" / "config=") out of an
// SDP string. Test-only crude extraction.
fn fmtp_value<'a>(sdp: &'a str, key: &str) -> Option<&'a str> {
    for line in sdp.lines() {
        if let Some(idx) = line.find(key) {
            let rest = &line[idx + key.len()..];
            let end = rest.find([';', ' ', '\r', '\n']).unwrap_or(rest.len());
            return Some(&rest[..end]);
        }
    }
    None
}

fn errors(issues: &[transmux::ConformanceIssue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

#[test]
fn ts_round_trip_recovers_timing_config_and_builds_fmp4() {
    let media = demux_fixture();
    let out = packetise(&media);

    // Original per-track truth from the demuxed Media.
    let orig_video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let orig_video_syncs = orig_video
        .samples
        .iter()
        .filter(|s| s.flags.is_sync)
        .count();
    // Build codec config from the generated SDP (exercises P2).
    let sdp = sdp_session::local_session(out.medias.clone());
    let sprop = fmtp_value(&sdp, "sprop-parameter-sets=").expect("sprop");
    let avc = avc_config_from_sprop(sprop).expect("avc from sprop");
    // SPS/PPS bytes recovered from SDP must equal the fixture's.
    if let CodecConfig::Avc { config, .. } = &orig_video.spec.config {
        assert_eq!(avc.config.sps.len(), config.config.sps.len());
        assert_eq!(
            avc.config.sps[0].0, config.config.sps[0].0,
            "SPS bytes round-trip"
        );
        assert_eq!(
            avc.config.pps[0].0, config.config.pps[0].0,
            "PPS bytes round-trip"
        );
    } else {
        panic!("expected video track to carry CodecConfig::Avc");
    }

    // Feed the packetised RTP for the video stream through the streaming depayloader.
    let video_stream = video_stream(&out);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        TrackId::new(1),
        RtpMediaKind::H264,
        CodecConfig::Avc {
            config: avc.clone(),
            width: 0,
            height: 0,
        },
        timescale(90_000),
    )]);
    let mut recovered = Vec::new();
    for pkt in &video_stream.packets {
        let c = pkt.as_contiguous();
        recovered.extend(d.push(TrackId::new(1), &c).unwrap());
    }
    recovered.extend(d.flush(TrackId::new(1)).unwrap());

    // Recovered sample count within 1 of the original (last-AU flush edge).
    assert!(
        (recovered.len() as i64 - orig_video.samples.len() as i64).abs() <= 1,
        "recovered {} vs original {}",
        recovered.len(),
        orig_video.samples.len()
    );
    // Sync points preserved.
    let rec_syncs = recovered.iter().filter(|s| s.flags.is_sync).count();
    assert_eq!(rec_syncs, orig_video_syncs, "keyframe count preserved");
    // Literal expectations for `h264-aac.ts` (75 access units at 90 kHz, a
    // B-frame encode whose wire timestamps are presentation times in decode
    // order). Each AU's duration is its step to the next decode instant, the
    // source's own 3600-tick (25 fps) grid, the first one included. The
    // numbers are asserted literally so this test cannot pass by
    // re-implementing the depacketiser's rule and comparing it with itself.
    const EXPECTED_FIRST_DURATIONS: [u32; 12] = [
        3600, 3600, 3600, 3600, 3600, 3600, 3600, 3600, 3600, 3600, 3600, 3600,
    ];
    const EXPECTED_TOTAL: u64 = 270_000;
    let got_durations: Vec<u32> = recovered
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0))
        .collect();
    assert_eq!(
        &got_durations[..EXPECTED_FIRST_DURATIONS.len()],
        &EXPECTED_FIRST_DURATIONS[..],
        "the first durations must be the fixture's own presentation steps"
    );
    let rec_total: u64 = got_durations.iter().map(|d| u64::from(*d)).sum();
    assert_eq!(
        rec_total, EXPECTED_TOTAL,
        "the recovered duration total must be the fixture's own"
    );

    // AAC: SDP config= → CodecConfig::Aac, rate/channels sane.
    let cfg_hex = fmtp_value(&sdp, "config=")
        .expect("SDP must carry AAC config= for the fixture's AAC track");
    let aac = aac_config_from_asc_hex(cfg_hex).expect("aac from config");
    match aac {
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => {
            assert!((8_000..=96_000).contains(&sample_rate));
            assert!((1..=8).contains(&channel_count));
        }
        _ => panic!("expected AAC"),
    }

    // Recovered video samples build a valid fMP4 init + media segment + part.
    let specs = d.track_specs();
    let init =
        build_init_segment(&specs, timescale(90_000)).expect("build_init_segment must succeed");
    assert!(!init.is_empty(), "init segment non-empty");
    let init_issues = validate_init_segment(&init);
    assert!(
        errors(&init_issues).is_empty(),
        "init segment must validate clean: {:?}",
        errors(&init_issues)
    );

    // Split the recovered samples across two "parts" to also exercise a
    // multi-segment (part) build, as CMAF/LL-* tests in this crate do.
    let mid = recovered.len() / 2;
    let (part1, part2) = recovered.split_at(mid);
    let seg1 = build_media_segment(1, &[FragmentTrackData::new(TrackId::new(1), 0, part1)])
        .expect("build_media_segment (part 1) must succeed");
    let part1_total: u64 = part1
        .iter()
        .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
        .sum();
    let seg2 = build_media_segment(
        2,
        &[FragmentTrackData::new(TrackId::new(1), part1_total, part2)],
    )
    .expect("build_media_segment (part 2) must succeed");

    for (label, seg) in [("part 1", &seg1), ("part 2", &seg2)] {
        assert!(!seg.is_empty(), "{label} segment non-empty");
        let issues = validate_media_segment(seg, SegmentOrigin::FromIr);
        assert!(
            errors(&issues).is_empty(),
            "{label} segment must validate clean: {:?}",
            errors(&issues)
        );
    }
}

// ── Loss/reorder gate (issue #779) ──────────────────────────────────────────
//
// Tests 1-4 perturb the *real* fixture-derived RTP packet list
// (`video_stream(&out).packets`, real NAL content from `h264-aac.ts`) by
// dropping/reordering/duplicating entries — there is no committed raw RTP
// capture to lose a packet from, and even one would only ever be perturbed
// the same synthetic way for this kind of test. Tests 5-6 are hand-built
// (PROVENANCE noted on each): neither a 16-bit sequence wrap nor a
// several-hundred-packet flood occurs in this short fixture.

const RTP_HEADER_LEN: usize = 12;

fn nal_type_of(pkt: &[u8]) -> u8 {
    pkt[RTP_HEADER_LEN] & 0x1F
}

/// (start index, length) of the longest run of consecutive FU-A-fragment
/// packets (RFC 6184 §5.8, NAL type 28 on the wire — the *fragment*
/// indicator, not the original NAL's type) — the most heavily fragmented
/// access unit in the stream, a real multi-packet NAL to drop/reorder a
/// middle fragment of. A run sharing one RTP timestamp can also include
/// non-FU-A single-NAL packets (e.g. a leading SEI/AUD before the large
/// IDR slice starts fragmenting), so this scans for the FU-A tag
/// specifically rather than just the shared timestamp.
fn longest_fu_a_run(packets: &[Vec<u8>]) -> (usize, usize) {
    const NAL_TYPE_FU_A: u8 = 28;
    let mut best = (0usize, 0usize);
    let mut i = 0;
    while i < packets.len() {
        if nal_type_of(&packets[i]) != NAL_TYPE_FU_A {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < packets.len() && nal_type_of(&packets[j]) == NAL_TYPE_FU_A {
            j += 1;
        }
        if j - i > best.1 {
            best = (i, j - i);
        }
        i = j;
    }
    best
}

/// Real AVC config + original (demuxed) video sample byte data + real RTP
/// packets for the video stream, from the shared `h264-aac.ts` fixture.
fn video_fixture() -> (CodecConfig, Vec<bytes::Bytes>, Vec<Vec<u8>>) {
    let media = demux_fixture();
    let out = packetise(&media);
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let config = video.spec.config.clone();
    let originals: Vec<bytes::Bytes> = video.samples.iter().map(|s| s.data.clone()).collect();
    let packets = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    (config, originals, packets)
}

/// Feed `pkts` through a fresh depayloader, returning the recovered samples'
/// byte data (in emission order) and whether any *loss* event was raised.
///
/// "Loss" is `SequenceGap`/`DamagedAccessUnit` specifically, not any signal:
/// this fixture (`h264-aac.ts`) is a real B-frame encode, so a faithful
/// depacketiser also reports `NonMonotonicTimestamp` (audit r04-W29) — the
/// wire's sampling timestamps step backward. That is not loss, and the
/// issue-#779 tests below are about loss, so it is filtered out here rather
/// than conflated with it.
fn run_video(
    config: &CodecConfig,
    pkts: &[Vec<u8>],
    reorder_depth: usize,
) -> (Vec<bytes::Bytes>, bool) {
    let mut d = RtpStreamDepacketiser::new(vec![
        RtpStreamTrack::new(
            TrackId::new(1),
            RtpMediaKind::H264,
            config.clone(),
            timescale(90_000),
        )
        .with_reorder_depth(reorder_depth),
    ]);
    let mut out = Vec::new();
    for pkt in pkts {
        out.extend(d.push(TrackId::new(1), pkt).unwrap());
    }
    out.extend(d.flush(TrackId::new(1)).unwrap());
    let mut had_loss = false;
    while let Some(e) = d.poll_loss_event() {
        if matches!(
            e,
            RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
        ) {
            had_loss = true;
        }
    }
    (out.into_iter().map(|s| s.data).collect(), had_loss)
}

/// Acceptance 1 — A committed-fixture-derived RTP stream with a dropped packet mid-FU-A
/// must yield no corrupt sample (issue #779 acceptance #1) — pre-#779 code
/// silently concatenates the fragments either side of the drop into a
/// malformed NAL and hands it downstream with no diagnostic trail; this test
/// fails against that code (no `SequenceGap` event exists to poll for, and
/// the malformed sample is emitted anyway).
#[test]
fn dropped_fu_a_fragment_yields_no_corrupt_sample() {
    let (config, originals, packets) = video_fixture();
    let (start, len) = longest_fu_a_run(&packets);
    assert!(
        len >= 3,
        "fixture must contain an access unit fragmented into >=3 RTP packets"
    );
    let drop_idx = start + len / 2;
    assert_eq!(
        nal_type_of(&packets[drop_idx]),
        28,
        "expected the dropped packet to be an FU-A fragment (RFC 6184 §5.8)"
    );

    let mut lossy = packets.clone();
    lossy.remove(drop_idx);

    let (recovered, had_loss) = run_video(&config, &lossy, 4);
    assert!(
        had_loss,
        "expected a loss event (SequenceGap) for the dropped fragment"
    );

    let original_set: HashSet<bytes::Bytes> = originals.iter().cloned().collect();
    for data in &recovered {
        assert!(
            original_set.contains(data),
            "recovered a sample that does not byte-match any original access \
             unit — this is exactly the silent corruption issue #779 fixes"
        );
    }
    assert!(
        recovered.len() < originals.len(),
        "expected at least the damaged access unit to be missing from the \
         recovered set (recovered {} vs original {})",
        recovered.len(),
        originals.len()
    );
}

/// Acceptance 2 — A reordered-within-window run reassembles byte-identically to the
/// in-order capture, and does so *silently* — no loss event, because
/// nothing was actually lost (see test 4 for why a spurious event here would
/// be its own bug).
#[test]
fn reordered_within_window_matches_in_order_byte_for_byte() {
    let (config, _originals, packets) = video_fixture();
    let (start, len) = longest_fu_a_run(&packets);
    assert!(
        len >= 2,
        "need a fragmented access unit with room to swap two fragments"
    );
    let mut reordered = packets.clone();
    reordered.swap(start, start + 1);

    let (in_order, in_order_loss) = run_video(&config, &packets, 4);
    let (from_reordered, reordered_loss) = run_video(&config, &reordered, 4);

    assert!(
        !in_order_loss,
        "a clean in-order run must not raise a loss event"
    );
    assert!(
        !reordered_loss,
        "a reorder fully recovered within the window must not raise a loss event"
    );
    assert_eq!(
        from_reordered, in_order,
        "a reordered-within-window capture must reassemble byte-identically \
         to the in-order one"
    );
}

/// Acceptance 3 — A duplicated packet changes nothing — RFC 3550 §A.1's "duplicate or
/// reordered packet" fall-through, this project's contract (issue #779) is
/// explicit: discard silently, no loss event, no change to the output.
#[test]
fn duplicated_packets_change_nothing() {
    let (config, _originals, packets) = video_fixture();
    let dup_idx = packets.len() / 2;
    let mut duplicated = packets.clone();
    duplicated.insert(dup_idx, packets[dup_idx].clone());

    let (in_order, in_order_loss) = run_video(&config, &packets, 4);
    let (from_duplicated, dup_loss) = run_video(&config, &duplicated, 4);

    assert!(!in_order_loss);
    assert!(!dup_loss, "a legal duplicate must not raise a loss event");
    assert_eq!(
        from_duplicated, in_order,
        "a duplicated packet must change nothing"
    );
}

/// Acceptance 4 — **Load-bearing**: a clean capture (video AND audio, full push+flush)
/// must emit ZERO loss signals end to end. A false-positive-prone detector
/// is worse than none — this project has already shipped exactly that
/// failure once (`PtsCheck`); a clean-stream negative would have caught it
/// then, and this is that same discipline applied here.
#[test]
fn clean_capture_emits_zero_loss_signals() {
    let media = demux_fixture();
    let out = packetise(&media);

    let vconfig = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
        .spec
        .config
        .clone();
    let aconfig = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .spec
        .config
        .clone();

    let mut d = RtpStreamDepacketiser::new(vec![
        RtpStreamTrack::new(
            TrackId::new(1),
            RtpMediaKind::H264,
            vconfig,
            timescale(90_000),
        ),
        // Clock rate is irrelevant to sequence-gate correctness (only
        // timing derivation, not under test here).
        RtpStreamTrack::new(
            TrackId::new(2),
            RtpMediaKind::Aac,
            aconfig,
            timescale(48_000),
        ),
    ]);

    for pkt in &video_stream(&out).packets {
        let c = pkt.as_contiguous();
        d.push(TrackId::new(1), &c).unwrap();
    }
    d.flush(TrackId::new(1)).unwrap();
    for pkt in &audio_stream(&out).packets {
        let c = pkt.as_contiguous();
        d.push(TrackId::new(2), &c).unwrap();
    }
    d.flush(TrackId::new(2)).unwrap();

    // Zero *loss* signals end to end. The fixture's presentation timestamps
    // are reordered (it is a real B-frame encode), so
    // `NonMonotonicTimestamp` is expected and is not loss — see `run_video`.
    let mut loss = Vec::new();
    let mut other = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        if matches!(
            e,
            RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
        ) {
            loss.push(e);
        } else {
            other.push(e);
        }
    }
    assert!(
        loss.is_empty(),
        "a clean capture must emit zero loss signals end to end, got {loss:?}"
    );
}

/// Minimal stub AVC config for the hand-built tests below — only
/// `RtpMediaKind::H264` dispatch and FU-A/single-NAL framing are exercised
/// (never the SPS/PPS content).
fn tiny_avc_config() -> CodecConfig {
    use isobmff::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0x42,
            profile_compatibility: 0,
            level_indication: 0x1E,
            length_size_minus_one: 3,
            sps: vec![],
            pps: vec![],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        }),
        width: 0,
        height: 0,
    }
}

/// Builds one single-NAL, marker-set RTP packet with an explicit sequence
/// number (wire format matching `rtp_stream`'s own unit tests).
fn wrap_vpkt(seq: u16, ts: u32, nal: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, 0x80 | 96];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0]);
    p.extend_from_slice(nal);
    p
}

/// Acceptance 5 — Sequence wrap 65535 → 0 must not be treated as a gap — PROVENANCE:
/// hand-built, since no capture this short crosses the 16-bit wrap point.
/// Wrapping arithmetic (never `>`) is exactly what issue #779 requires here.
#[test]
fn sequence_wrap_is_not_treated_as_a_gap() {
    let config = tiny_avc_config();
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        TrackId::new(1),
        RtpMediaKind::H264,
        config,
        timescale(90_000),
    )]);

    let seqs: [u16; 5] = [65533, 65534, 65535, 0, 1];
    let nals: [[u8; 2]; 5] = [
        [0x65, 0xAA],
        [0x41, 0xBB],
        [0x41, 0xCC],
        [0x41, 0xDD],
        [0x41, 0xEE],
    ];
    let mut recovered = Vec::new();
    for (i, seq) in seqs.iter().enumerate() {
        let ts = 1000 + i as u32 * 3000;
        recovered.extend(
            d.push(TrackId::new(1), &wrap_vpkt(*seq, ts, &nals[i]))
                .unwrap(),
        );
    }
    recovered.extend(d.flush(TrackId::new(1)).unwrap());

    assert!(
        d.poll_loss_event().is_none(),
        "sequence wrap 65535 -> 0 must not be treated as a gap"
    );
    assert_eq!(recovered.len(), 5, "all 5 access units must be recovered");
}

/// Acceptance 6 — The reorder buffer is bounded: a flood of out-of-order packets cannot
/// grow it without limit. PROVENANCE: hand-built — proving a bound holds
/// needs a flood far bigger than any short real fixture provides.
///
/// Black-box proof (no access to the private buffer from an integration
/// test): each flood packet jumps far ahead of `expected`, spaced 50 apart
/// so none ever land exactly consecutively. If the buffer were unbounded,
/// none of these would ever force a decision, so *zero* `SequenceGap`
/// events would fire no matter how long the flood ran — exactly what
/// pre-#779 code (no reorder buffer, no gap detection at all) would do.
/// With the bound in place, the first `DEPTH` packets fill it for free and
/// every packet after that forces exactly one resolution, so the resulting
/// count is deterministic, not just "at least one".
#[test]
fn reorder_buffer_is_bounded_under_a_flood_of_out_of_order_packets() {
    const DEPTH: usize = 8;
    const FLOOD_LEN: u16 = 200;

    let config = tiny_avc_config();
    let mut d = RtpStreamDepacketiser::new(vec![
        RtpStreamTrack::new(
            TrackId::new(1),
            RtpMediaKind::H264,
            config,
            timescale(90_000),
        )
        .with_reorder_depth(DEPTH),
    ]);

    // Establish `expected` with one in-order packet (seq 0).
    d.push(TrackId::new(1), &wrap_vpkt(0, 1000, &[0x65, 0xAA]))
        .unwrap();

    for k in 1..=FLOOD_LEN {
        let seq = 1000u16.wrapping_add(k.wrapping_mul(50));
        d.push(TrackId::new(1), &wrap_vpkt(seq, 1000, &[0x41, 0xBB]))
            .unwrap();
    }

    let mut gap_events = 0usize;
    while let Some(e) = d.poll_loss_event() {
        if matches!(e, RtpLossEvent::SequenceGap { .. }) {
            gap_events += 1;
        }
    }

    let expected_gap_events = usize::from(FLOOD_LEN) - DEPTH;
    assert_eq!(
        gap_events, expected_gap_events,
        "reorder buffer must force a bounded, deterministic number of \
         resolutions, proving it never grows past its configured depth"
    );
}

/// A duplicate of an already-delivered packet and a duplicate of a *held*
/// (future) packet are both dropped, exactly as RFC 3550 §A.1's fall-through
/// "duplicate or reordered packet" arm prescribes — and neither is delivered
/// twice or shifts the timeline.
#[test]
fn duplicates_behind_and_within_the_buffer_are_dropped() {
    let mut pairs: Vec<(u16, u32)> = Vec::new();
    // A run with a hole, so the reorder buffer holds a future packet: seq 1003
    // is not sent yet.
    for k in [0u16, 1, 2, 4, 5] {
        pairs.push((1000 + k, 3000 * u32::from(k)));
    }
    // Now send seq 1003 (fills the hole: 1003 and 1004 are held/released)...
    pairs.push((1003, 12_000));
    // ...then *duplicate* a packet that is already delivered (seq 1000) and one
    // that is currently held or already released from the buffer (seq 1005).
    pairs.push((1000, 0));
    pairs.push((1005, 15_000));
    // A couple more in order, to show the run continues unaffected.
    pairs.push((1005, 15_000));
    pairs.push((1006, 18_000));

    // Expected: one AU per *distinct* sequence number, in order. The
    // identifying byte is the packet's position among the distinct ones, which
    // `run_seqs` assigns by feed order — so the expectation is the fed series
    // with the duplicate feeds removed.
    let mut distinct: Vec<usize> = Vec::new();
    let mut seen: Vec<u16> = Vec::new();
    for (i, &(seq, _)) in pairs.iter().enumerate() {
        if !seen.contains(&seq) {
            seen.push(seq);
            distinct.push(i);
        }
    }
    // The distinct sequence numbers, in wire order: the hole at 1003 is filled
    // late, so it arrives after 1004/1005 as §A.1 requires.
    assert_eq!(seen, vec![1000, 1001, 1002, 1004, 1005, 1003, 1006]);
    // The delivered AUs are those packets, one each, and nothing is delivered
    // twice: seven AUs for ten fed packets, and their bytes are the *feed*
    // indices of the packets that carried them.
    let (recovered, _) = run_seqs(&pairs);
    assert_eq!(recovered.len(), 7, "one AU per distinct sequence number");
    let bytes: Vec<u8> = recovered.iter().map(|s| s[5]).collect();
    assert_eq!(
        bytes,
        vec![0u8, 1, 2, 5, 3, 4, 9],
        "the delivered AUs must be the distinct packets, in wire order"
    );
}

/// A stream that crosses the 16-bit wrap mid-run (`65535 -> 0`) in steady state
/// must lose nothing and raise no signal: §A.1 counts a 64k cycle
/// (`s->cycles += RTP_SEQ_MOD`) and accepts the number.
#[test]
fn steady_state_wrap_mid_run_loses_nothing() {
    const RUN: usize = 300; // crosses 65535 -> 0 part-way
    let start = 65_400u16;
    let pairs: Vec<(u16, u32)> = (0..RUN)
        .map(|k| (start.wrapping_add(k as u16), 3000 * k as u32))
        .collect();

    let (recovered, events) = run_seqs(&pairs);
    assert_eq!(
        recovered.len(),
        RUN,
        "every packet must be delivered across the wrap"
    );
    assert!(
        events.is_empty(),
        "a wrap is not loss or a restart, got {events:?}"
    );
    // The wrap is inside the run (it started at 65400 and ran 300 packets).
    assert!(
        usize::from(start) + RUN > 65_536,
        "the run must cross the boundary"
    );
    // And the emitted payloads are in wire order, including across the wrap
    // (the identifying byte is the packet's feed index, modulo 256).
    let expected: Vec<Vec<u8>> = (0..RUN)
        .map(|i| vec![0x00, 0x00, 0x00, 0x02, 0x41, i as u8])
        .collect();
    assert_eq!(recovered, expected, "delivery must be uninterrupted");
}

// ── RFC 3550 §A.1 source (re-)validation ────────────────────────────────────
//
// PROVENANCE: hand-built. A sender-side sequence-number reset, a 70 000-packet
// run and a 16-bit wrap are all events no committed short capture contains.
//
// The constants below are transcribed from RFC 3550 §A.1's own `update_seq`
// (fetched from https://www.rfc-editor.org/rfc/rfc3550.txt, Appendix A.1):
//
//     const int MAX_DROPOUT = 3000;
//     const int MAX_MISORDER = 100;
//     const int MIN_SEQUENTIAL = 2;
//     ... s->bad_seq = (seq + 1) & (RTP_SEQ_MOD-1); ...
//
// where `RTP_SEQ_MOD` is `(1<<16)`.

/// §A.1 `MAX_MISORDER`.
const MAX_MISORDER: u16 = 100;

/// One single-NAL, marker-set packet at an explicit sequence number and
/// timestamp, carrying a distinct byte so the sample can be identified.
fn seq_pkt(seq: u16, ts: u32, byte: u8) -> Vec<u8> {
    wrap_vpkt(seq, ts, &[0x41, byte])
}

/// Feed `(seq, ts)` pairs, returning the recovered sample payload bytes in
/// emission order and the loss/reorder signals raised.
fn run_seqs(pairs: &[(u16, u32)]) -> (Vec<Vec<u8>>, Vec<RtpLossEvent>) {
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        TrackId::new(1),
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    let mut out = Vec::new();
    for (i, &(seq, ts)) in pairs.iter().enumerate() {
        out.extend(d.push(TrackId::new(1), &seq_pkt(seq, ts, i as u8)).unwrap());
    }
    out.extend(d.flush(TrackId::new(1)).unwrap());
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    (out.into_iter().map(|s| s.data.to_vec()).collect(), events)
}

/// §A.1's wild-jump rule needs **two** sequential packets: a single packet
/// arriving far behind the live run is discarded (`bad_seq` is armed) and the
/// run continues untouched.
///
/// This is the defect the range-based design had: one stray packet more than
/// `MAX_MISORDER` behind moved the baseline immediately, and the *live* run —
/// which is still arriving — was then the thing discarded.
#[test]
fn one_stray_far_behind_packet_does_not_resync() {
    // Establish a live run at 1000.. and get it past probation.
    let mut pairs: Vec<(u16, u32)> = (0..8).map(|k| (1000 + k, 3000 * u32::from(k))).collect();
    // One stray packet 101+ behind: behind the live `expected` by more than
    // MAX_MISORDER, and — because it is 101 behind — not a wild jump either,
    // it is exactly the "duplicate or reordered" or "bad_seq" case. Either
    // way it must not move the baseline.
    pairs.push((1000u16.wrapping_sub(101), 900));
    // The live run continues with the next numbers.
    pairs.extend((8..14).map(|k| (1000 + k, 3000 * u32::from(k))));

    let (recovered, events) = run_seqs(&pairs);
    let loss: Vec<RtpLossEvent> = events
        .iter()
        .copied()
        .filter(|e| {
            matches!(
                e,
                RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
            )
        })
        .collect();
    assert!(
        loss.is_empty(),
        "a single stray packet must not resync or declare loss, got {loss:?}"
    );
    // 14 packets → 14 AUs; the stray one carries byte 8 and must not appear
    // anywhere in the output (it was discarded, not adopted).
    assert_eq!(
        recovered.len(),
        14,
        "the live run must continue uninterrupted: {} AUs of 14 recovered",
        recovered.len()
    );
    let payloads: Vec<Vec<u8>> = recovered.to_vec();
    let stray = vec![0x00, 0x00, 0x00, 0x02, 0x41, 8u8];
    assert!(
        !payloads.contains(&stray),
        "the stray packet must be discarded, not delivered"
    );
}

/// A real backward seek is a *run* at the new numbering: the first packet is
/// discarded and arms `bad_seq`, and the second, being `bad_seq` itself,
/// re-syncs the source (§A.1's "Two sequential packets"). The new run must
/// then be delivered in full however long it is — including past the old
/// numbering and past 32 768 packets, the blackout window the pre-fix design
/// could not cross.
#[test]
fn confirmed_backward_seek_resyncs_and_delivers_a_long_run() {
    // A 70 000-packet new run: far past the old `expected` (which was ~1000)
    // and past the 32 768-packet blackout, and long enough to wrap the 16-bit
    // counter (70 000 > 65 536).
    const RUN: u32 = 70_000;
    let mut pairs: Vec<(u16, u32)> = (0..4).map(|k| (1000 + k, 3000 * u32::from(k))).collect();
    // The seek: numbering restarts at 0 with a fresh timestamp origin.
    for k in 0..RUN {
        pairs.push((k as u16, 1_000_000 + 3000 * k));
    }

    let (recovered, events) = run_seqs(&pairs);
    // Exactly: the pre-seek run's four AUs (bytes 0..3), then the new run's
    // first packet discarded arming `bad_seq` (byte 4, i.e. run index 0) and
    // every following packet of it delivered.
    assert_eq!(
        recovered.len(),
        4 + (RUN as usize - 1),
        "the whole new run must be delivered, got {}",
        recovered.len()
    );
    // Spot-check identity at the wrap: the run crosses 65535 -> 0.
    let first_new = 4;
    assert_eq!(
        recovered[first_new].as_ref(),
        [0x00, 0x00, 0x00, 0x02, 0x41, 5u8],
        "the packet after the arming one must be the new run's second packet"
    );
    assert_eq!(
        recovered[first_new + 1].as_ref(),
        [0x00, 0x00, 0x00, 0x02, 0x41, 6u8],
        "and the third, so the run is delivered contiguously from there"
    );
    assert_eq!(
        recovered[first_new - 1].as_ref(),
        [0x00, 0x00, 0x00, 0x02, 0x41, 3u8],
        "the last pre-seek AU delivered is the fourth packet's"
    );
    // The delivered prefix is the pre-seek packets only. (A `contains` check on its byte
    // would not mean anything here — the identifying byte is the packet index
    // modulo 256, so 3 recurs later in a 70 000-packet run.)
    assert_eq!(
        &recovered[..first_new],
        &[
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 0],
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 1],
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 2],
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 3],
        ][..],
        "every pre-seek AU is delivered, the held seam AU included"
    );
    // Delivery continues across the 16-bit wrap: the identifying byte is the
    // packet index modulo 256, so at index `first_new + k` of the output the
    // byte is `(k + 5) mod 256` for the k-th packet of the new run (its first
    // was dropped). Checked at the wrap and at the end of the run.
    for k in [65_534usize, RUN as usize - 2] {
        let expected_byte = (5u16.wrapping_add(k as u16)) as u8;
        assert_eq!(
            recovered[first_new + k].as_ref(),
            [0x00, 0x00, 0x00, 0x02, 0x41, expected_byte],
            "delivery must continue at output index {}",
            first_new + k
        );
    }
    let loss: Vec<RtpLossEvent> = events
        .iter()
        .copied()
        .filter(|e| {
            matches!(
                e,
                RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
            )
        })
        .collect();
    assert!(
        loss.is_empty(),
        "a source renumbering is not loss, got {loss:?}"
    );

    // The run crosses the 16-bit wrap: the last sequence numbers must still
    // be delivered (0..RUN includes the wrap at 65536).
    assert!(
        RUN > u32::from(u16::MAX),
        "the run must cross the 16-bit wrap for this to test it"
    );
}

/// A **second** seek after a first must resync exactly the same way — nothing
/// about the first seam may persist (the sticky-bound defect).
///
/// Both jumps are wild ones in §A.1's terms: the second run's numbering (100)
/// is more than `MAX_MISORDER` behind the first run's `max_seq` (1005), and the
/// third's (50_000) is more than `MAX_DROPOUT` ahead of the second's (119).
#[test]
fn a_second_seek_after_a_first_resyncs_too() {
    let mut pairs: Vec<(u16, u32)> = Vec::new();
    let mut ts = 3000u32;
    // Run 1: seq 1000..1005.
    for k in 0..6u16 {
        pairs.push((1000 + k, ts));
        ts += 3000;
    }
    // Seek 1: numbering jumps back to 100, 20 packets.
    for k in 0..20u16 {
        pairs.push((100 + k, ts));
        ts += 3000;
    }
    // Seek 2: numbering jumps forward past MAX_DROPOUT to 50_000, 20 packets.
    for k in 0..20u16 {
        pairs.push((50_000 + k, ts));
        ts += 3000;
    }

    let (recovered, events) = run_seqs(&pairs);
    // Exactly: run 1's packets (indices 0..5, the seam AU included), then
    // seek 1's arming packet dropped (index 6) and the rest delivered
    // (7..25, seam AU included), then seek 2's arming packet dropped
    // (index 26) and the rest delivered (27..45).
    let expected: Vec<Vec<u8>> = (0..6u8)
        .chain(7..26u8)
        .chain(27..46u8)
        .map(|b| vec![0x00, 0x00, 0x00, 0x02, 0x41, b])
        .collect();
    assert_eq!(
        recovered, expected,
        "both post-seek runs must be delivered, with only the arming packets dropped"
    );
    let loss: Vec<RtpLossEvent> = events
        .iter()
        .copied()
        .filter(|e| {
            matches!(
                e,
                RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
            )
        })
        .collect();
    assert!(loss.is_empty(), "neither seek is loss, got {loss:?}");
}

/// §A.1: a jump *ahead* of `max_seq` by `MAX_DROPOUT` (3000) or more is a wild
/// jump needing confirmation, and a two-packet run at the new numbering
/// restarts the source — the forward-restart case (a sender that restarts with
/// a higher numbering).
#[test]
fn confirmed_forward_jump_restarts_the_source() {
    let mut pairs: Vec<(u16, u32)> = (0..6).map(|k| (1000 + k, 3000 * u32::from(k))).collect();
    // A jump well past MAX_DROPOUT (3000): a restart at 20 000.
    let mut ts = 100_000u32;
    for k in 0..12u16 {
        pairs.push((20_000 + k, ts));
        ts += 3000;
    }

    let (recovered, events) = run_seqs(&pairs);
    // Exactly: every pre-jump AU (bytes 0..5, the held seam AU flushed), the
    // first new-run packet discarded while §A.1's `bad_seq` armed (byte 6),
    // and then every following packet of the restarted run (bytes 7..17).
    let expected: Vec<Vec<u8>> = (0..6u8)
        .chain(7..18u8)
        .map(|b| vec![0x00, 0x00, 0x00, 0x02, 0x41, b])
        .collect();
    assert_eq!(
        recovered, expected,
        "the restarted run must be delivered with only the arming packet dropped"
    );
    let loss: Vec<RtpLossEvent> = events
        .iter()
        .copied()
        .filter(|e| {
            matches!(
                e,
                RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
            )
        })
        .collect();
    assert!(
        loss.is_empty(),
        "a source restart is not loss, got {loss:?}"
    );
}

/// A *small* forward gap (under `MAX_DROPOUT`) is ordinary loss, not a
/// restart: §A.1's "in order, with permissible gap". The packets after it
/// must still be delivered.
#[test]
fn small_forward_gap_is_permissible_loss_not_a_restart() {
    let mut pairs: Vec<(u16, u32)> = (0..5).map(|k| (1000 + k, 3000 * u32::from(k))).collect();
    // Skip 20 numbers (well under MAX_DROPOUT): a plain loss.
    let mut ts = 15_000u32;
    for k in 0..5u16 {
        pairs.push((1025 + k, ts));
        ts += 3000;
    }

    let (recovered, _events) = run_seqs(&pairs);
    assert_eq!(
        recovered.len(),
        10,
        "a gap under MAX_DROPOUT must not lose the packets after it, got {}",
        recovered.len()
    );
}

/// A legal misorder within `MAX_MISORDER` is accepted ("duplicate or reordered
/// packet"), not treated as a restart: two packets arriving swapped must both
/// be delivered, in order.
#[test]
fn small_misorder_is_accepted_not_a_restart() {
    let pairs: Vec<(u16, u32)> = vec![
        (1000, 0),
        (1001, 3000),
        // Swapped pair, 2 apart in time: within MAX_MISORDER.
        (1003, 9000),
        (1002, 6000),
        (1004, 12000),
    ];
    let (recovered, events) = run_seqs(&pairs);
    assert_eq!(
        recovered.len(),
        5,
        "a reordered pair must be reassembled, got {}",
        recovered.len()
    );
    let loss: Vec<RtpLossEvent> = events
        .iter()
        .copied()
        .filter(|e| matches!(e, RtpLossEvent::SequenceGap { .. }))
        .collect();
    assert!(
        loss.is_empty(),
        "a reorder inside the buffer must not declare a gap, got {loss:?}"
    );
}

/// The §A.1 bound constants must stay the RFC's own values, and the narrowed
/// `RTP_SEQ_MOD - MAX_MISORDER` must not underflow.
#[test]
fn a1_constants_match_the_rfc() {
    assert_eq!(MAX_MISORDER, 100, "RFC 3550 §A.1: MAX_MISORDER = 100");
    // The other transcribed constants (`MAX_DROPOUT` = 3000, `MIN_SEQUENTIAL`
    // = 2, `RTP_SEQ_MOD` = 1 << 16) are private to `rtp_stream.rs`; their
    // effect is what the resync tests above exercise, and `MAX_MISORDER` is
    // re-stated here because the boundary between "misordered" and "the source
    // restarted" is the one a reader is most likely to expect the RFC's number
    // for.
    const RTP_SEQ_MOD: u32 = 1 << 16;
    assert!(
        RTP_SEQ_MOD > u32::from(MAX_MISORDER),
        "MAX_MISORDER must be far below the 16-bit space, or the wild-jump          bound could not be represented"
    );
}

fn dropped_pts(events: &[RtpLossEvent]) -> Vec<i64> {
    events
        .iter()
        .filter_map(|e| match e {
            RtpLossEvent::DroppedAccessUnit { pts, .. } => Some(*pts),
            _ => None,
        })
        .collect()
}

#[test]
fn resync_accounts_for_every_held_reorder_window_au() {
    const PRE_SEEK_TS: [u32; 9] = [
        0, 12_000, 6_000, 3_000, 9_000, 24_000, 18_000, 15_000, 21_000,
    ];
    const SEEK_ORIGIN: u32 = 1_000_000;
    let mut pairs: Vec<(u16, u32)> = PRE_SEEK_TS
        .iter()
        .enumerate()
        .map(|(k, &ts)| (1000 + k as u16, ts))
        .collect();
    pairs.extend((0u16..4).map(|k| (k, SEEK_ORIGIN + 3000 * u32::from(k))));

    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        TrackId::new(1),
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    let mut pts = Vec::new();
    for (i, &(seq, ts)) in pairs.iter().enumerate() {
        for s in d.push(TrackId::new(1), &seq_pkt(seq, ts, i as u8)).unwrap() {
            pts.push(s.timing.and_then(|t| t.pts()).unwrap());
        }
    }
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    let completed_before_seek = PRE_SEEK_TS.len();
    let emitted_before_seek = pts.iter().filter(|&&p| p < i64::from(SEEK_ORIGIN)).count();
    let dropped = dropped_pts(&events);
    assert_eq!(
        emitted_before_seek + dropped.len(),
        completed_before_seek,
        "emitted {emitted_before_seek}, dropped {dropped:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RtpLossEvent::NonMonotonicTimestamp { .. })),
        "the stream must have exercised the reorder window"
    );
}

#[test]
fn reorder_window_overflow_reports_every_dropped_au() {
    const AUS: u32 = 300;
    const WINDOW: usize = 256;
    const START_TS: u32 = 2_000_000;
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        TrackId::new(1),
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    let mut emitted_before_flush = 0usize;
    for k in 0..AUS {
        emitted_before_flush += d
            .push(
                TrackId::new(1),
                &seq_pkt(k as u16, START_TS - 1000 * k, k as u8),
            )
            .unwrap()
            .len();
    }
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    let emitted = d.flush(TrackId::new(1)).unwrap().len() + emitted_before_flush;
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    let dropped = dropped_pts(&events);
    assert!(!dropped.is_empty(), "the window overflowed {WINDOW}");
    assert_eq!(
        dropped[0],
        i64::from(START_TS),
        "oldest AU is dropped first"
    );
    assert_eq!(emitted + dropped.len(), AUS as usize);
}

// ── audit 23: AU buffer cap, Stage::demand, SSRC change, SR sync ───────────

/// One single-NAL RTP packet of exactly `total` bytes, with an explicit SSRC.
fn sized_packet(seq: u16, ts: u32, ssrc: u32, marker: bool, total: usize) -> Vec<u8> {
    assert!(total > 13, "need room for an RTP header and one NAL byte");
    let mut p = vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    p.push(if marker { 0x65 } else { 0x41 });
    p.resize(total, 0xAA);
    assert_eq!(p.len(), total);
    p
}

/// The in-progress-AU cap is 4 MiB: exactly that much is accepted, one byte
/// more errors with that cap, `Stage::demand` saturates at the cap and clears
/// once the buffer is reset, and a 2 MiB AU is well under the cap.
#[test]
fn au_buffer_cap_is_four_mebibytes_and_demand_tracks_it() {
    const CAP: usize = 4 * 1024 * 1024;
    const CHUNK: usize = 1400;
    let id = TrackId::new(1);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    assert_eq!(Stage::demand(&d), Demand::default());

    let mut seq = 0u16;
    let mut written = 0usize;
    while CAP - written > CHUNK {
        let pkt = sized_packet(seq, 1000, 0, false, CHUNK);
        assert!(d.push(id, &pkt).unwrap().is_empty());
        written += pkt.len();
        seq += 1;
    }
    let last = sized_packet(seq, 1000, 0, false, CAP - written);
    assert!(
        d.push(id, &last).unwrap().is_empty(),
        "exactly {CAP} bytes is within the cap"
    );
    written += last.len();
    seq += 1;
    assert_eq!(written, CAP);
    assert_eq!(
        Stage::demand(&d),
        Demand::saturated(),
        "the buffer is at the cap"
    );

    let over = sized_packet(seq, 1000, 0, false, 16);
    let err = d.push(id, &over).expect_err("one byte over the cap");
    assert!(
        matches!(err, transmux::error::Error::BufferCapExceeded { cap, .. } if cap == CAP),
        "unexpected error: {err:?}"
    );
    assert_eq!(
        Stage::demand(&d),
        Demand::default(),
        "the overflow reset the in-progress buffer"
    );
    seq += 1;

    // The next packet starts a fresh AU.
    assert!(
        d.push(id, &wrap_vpkt(seq, 9000, &[0x65, 0xAA]))
            .unwrap()
            .is_empty()
    );
    let mut s = d
        .push(id, &wrap_vpkt(seq + 1, 12_000, &[0x41, 0xBB]))
        .unwrap();
    s.extend(d.flush(id).unwrap());
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].timing.and_then(|t| t.duration()), Some(3000));

    // A 2 MiB access unit is accepted end to end.
    const HALF: usize = 2 * 1024 * 1024;
    let mut big = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    let pkt = sized_packet(0, 1000, 0, true, HALF);
    assert!(
        big.push(id, &pkt).unwrap().is_empty(),
        "a 2 MiB AU is accepted"
    );
    assert_eq!(Stage::demand(&big), Demand::default());
}

fn ssrc_packet(seq: u16, ts: u32, ssrc: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&ssrc.to_be_bytes());
    p.extend_from_slice(nal);
    p
}

/// A mid-stream SSRC change is a new source (RFC 3550 §8.2): it re-initialises
/// sequence tracking and delivers the packet, with no gap reported.
#[test]
fn ssrc_change_reinitialises_without_a_gap() {
    let id = TrackId::new(1);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    let mut n = 0usize;
    for (seq, ts, ssrc, nal) in [
        (10u16, 1000u32, 0x1111u32, [0x65u8, 0xAA]),
        (11, 4000, 0x1111, [0x41, 0xBB]),
        (5000, 7000, 0x2222, [0x41, 0xCC]),
    ] {
        n += d
            .push(id, &ssrc_packet(seq, ts, ssrc, true, &nal))
            .unwrap()
            .len();
    }
    n += d.flush(id).unwrap().len();
    assert_eq!(n, 3, "the post-SSRC-change packet must be delivered");
    assert!(d.poll_loss_event().is_none(), "an SSRC change is not loss");
}

/// A single wild forward jump is discarded (§A.1 arms `bad_seq`) and never
/// enters the reorder buffer; the live run continues with no gap.
#[test]
fn wild_forward_jump_is_discarded_not_held() {
    const WILD: [u8; 2] = [0x41, 0x77];
    let id = TrackId::new(1);
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        RtpMediaKind::H264,
        tiny_avc_config(),
        timescale(90_000),
    )]);
    for (seq, ts) in [(10u16, 1000u32), (11, 4000), (12, 7000)] {
        d.push(id, &ssrc_packet(seq, ts, 0x1111, true, &[0x41, seq as u8]))
            .unwrap();
    }
    assert!(
        d.push(id, &ssrc_packet(30_000, 100_000, 0x1111, true, &WILD))
            .unwrap()
            .is_empty(),
        "the wild packet is discarded"
    );
    let mut samples = d
        .push(id, &ssrc_packet(13, 10_000, 0x1111, true, &[0x41, 13]))
        .unwrap();
    samples.extend(d.flush(id).unwrap());
    assert_eq!(samples.len(), 4, "10, 11, 12 and 13");
    assert!(
        samples.iter().all(|s| s.data.as_ref() != WILD),
        "the wild packet must never be delivered"
    );
    let mut loss = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        if matches!(
            e,
            RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
        ) {
            loss.push(e);
        }
    }
    assert!(loss.is_empty(), "the wild jump is not loss: {loss:?}");
}

use transmux::rtcp::SenderReport;

fn sender_report(rtp_ts: u32, ntp_msw: u32, ntp_lsw: u32) -> SenderReport {
    SenderReport::new(
        rtp_packet::Ssrc::new(0),
        ntp_msw,
        ntp_lsw,
        rtp_ts,
        0,
        0,
        Vec::new(),
    )
}

fn two_track(clock_a: u32, clock_b: u32) -> RtpStreamDepacketiser {
    RtpStreamDepacketiser::new(vec![
        RtpStreamTrack::new(
            TrackId::new(1),
            RtpMediaKind::H264,
            tiny_avc_config(),
            timescale(clock_a),
        ),
        RtpStreamTrack::new(
            TrackId::new(2),
            RtpMediaKind::H264,
            tiny_avc_config(),
            timescale(clock_b),
        ),
    ])
}

fn feed_first_au(d: &mut RtpStreamDepacketiser, track: u32, ts: u32) {
    d.push(
        TrackId::new(track),
        &ssrc_packet(0, ts, track, true, &[0x65, 0xAA]),
    )
    .unwrap();
}

fn start_for(starts: &[(TrackId, u64)], track: u32) -> u64 {
    starts
        .iter()
        .find(|(id, _)| *id == TrackId::new(track))
        .map(|(_, s)| *s)
        .expect("track is anchored")
}

/// B's first AU is 0.5 s after A's, both anchored by Sender Reports whose RTP
/// timestamps differ from their first AUs (so the sign of the wallclock
/// offset matters): A is the origin, B gets exactly half a second in its own
/// clock.
#[test]
fn sr_offset_preserves_the_sign_of_the_wallclock_delta() {
    let mut d = two_track(90_000, 48_000);
    feed_first_au(&mut d, 1, 9000);
    feed_first_au(&mut d, 2, 96_000);
    d.push_sender_report(TrackId::new(1), sender_report(9000, 1000, 0));
    d.push_sender_report(TrackId::new(2), sender_report(48_000, 999, 2_147_483_648));
    let starts = d.try_sync_start_decode_times().unwrap();
    assert_eq!(start_for(&starts, 1), 0);
    assert_eq!(start_for(&starts, 2), 24_000);
}

/// A half-tick offset rounds half up: 0.25 s at a 2 Hz clock is 0.5 ticks.
#[test]
fn sr_start_ticks_round_half_up() {
    let mut d = two_track(90_000, 2);
    feed_first_au(&mut d, 1, 9000);
    feed_first_au(&mut d, 2, 9);
    d.push_sender_report(TrackId::new(1), sender_report(9000, 1000, 0));
    d.push_sender_report(TrackId::new(2), sender_report(9, 1000, 1_073_741_824));
    let starts = d.try_sync_start_decode_times().unwrap();
    assert_eq!(start_for(&starts, 1), 0);
    assert_eq!(start_for(&starts, 2), 1, "0.5 ticks rounds up");
}

/// Fewer than two anchored tracks yields the empty "not enough anchors" vec.
#[test]
fn fewer_than_two_anchors_returns_empty() {
    let mut d = two_track(90_000, 48_000);
    feed_first_au(&mut d, 1, 9000);
    feed_first_au(&mut d, 2, 96_000);
    assert!(d.try_sync_start_decode_times().unwrap().is_empty());
    d.push_sender_report(TrackId::new(1), sender_report(9000, 1000, 0));
    assert!(
        d.try_sync_start_decode_times().unwrap().is_empty(),
        "one anchor is not enough"
    );
}

/// A spread over one hour is refused and reports the spread; exactly one
/// hour is accepted.
#[test]
fn sr_skew_over_an_hour_is_refused_with_the_spread() {
    let mut d = two_track(48_000, 48_000);
    feed_first_au(&mut d, 1, 4800);
    feed_first_au(&mut d, 2, 4800);
    d.push_sender_report(TrackId::new(1), sender_report(4800, 1000, 0));
    d.push_sender_report(TrackId::new(2), sender_report(4800, 4601, 0));
    match d.try_sync_start_decode_times() {
        Err(WallSkewExceeded { skew_seconds }) => assert_eq!(skew_seconds, 3601.0),
        other => panic!("expected WallSkewExceeded, got {other:?}"),
    }
    d.push_sender_report(TrackId::new(2), sender_report(4800, 4600, 0));
    assert_eq!(d.try_sync_start_decode_times().unwrap().len(), 2);
}

/// A zero-NTP Sender Report is rejected and counted.
#[test]
fn zero_ntp_sender_reports_are_counted() {
    let mut d = two_track(90_000, 48_000);
    assert_eq!(d.rejected_sender_reports(), 0);
    d.push_sender_report(TrackId::new(1), sender_report(9000, 0, 0));
    assert_eq!(d.rejected_sender_reports(), 1);
    d.push_sender_report(TrackId::new(2), sender_report(4800, 0, 0));
    assert_eq!(d.rejected_sender_reports(), 2);
}

// ── audit 23: declared durations, reorder depth, known_duration precedence ──

use crate::bit_writer::BitWriter;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

/// A Main-profile SPS with VUI `timing_info` and an optional
/// `bitstream_restriction` declaring `max_num_reorder_frames`.
fn vui_sps(num_units_in_tick: u32, time_scale: u32, reorder: Option<u32>) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u(8, 77); // profile_idc
    w.u(8, 0); // constraint flags
    w.u(8, 30); // level_idc
    w.ue(0); // seq_parameter_set_id
    w.ue(0); // log2_max_frame_num_minus4 -> 4 bits
    w.ue(0); // pic_order_cnt_type
    w.ue(2); // log2_max_pic_order_cnt_lsb_minus4 -> 6 bits
    w.ue(1); // max_num_ref_frames
    w.u(1, 0); // gaps_in_frame_num_value_allowed_flag
    w.ue(19); // pic_width_in_mbs_minus1 -> 320
    w.ue(14); // pic_height_in_map_units_minus1 -> 240
    w.u(1, 1); // frame_mbs_only_flag
    w.u(1, 1); // direct_8x8_inference_flag
    w.u(1, 0); // frame_cropping_flag
    w.u(1, 1); // vui_parameters_present_flag
    w.u(1, 0); // aspect_ratio_info_present_flag
    w.u(1, 0); // overscan_info_present_flag
    w.u(1, 0); // video_signal_type_present_flag
    w.u(1, 0); // chroma_loc_info_present_flag
    w.u(1, 1); // timing_info_present_flag
    w.u(32, u64::from(num_units_in_tick));
    w.u(32, u64::from(time_scale));
    w.u(1, 1); // fixed_frame_rate_flag
    w.u(1, 0); // nal_hrd_parameters_present_flag
    w.u(1, 0); // vcl_hrd_parameters_present_flag
    w.u(1, 0); // pic_struct_present_flag
    w.u(1, u64::from(reorder.is_some())); // bitstream_restriction_flag
    if let Some(reorder) = reorder {
        w.u(1, 0); // motion_vectors_over_pic_boundaries_flag
        w.ue(0); // max_bytes_per_pic_denom
        w.ue(0); // max_bits_per_mb_denom
        w.ue(0); // log2_max_mv_length_horizontal
        w.ue(0); // log2_max_mv_length_vertical
        w.ue(u64::from(reorder)); // max_num_reorder_frames
        w.ue(u64::from(reorder) + 2); // max_dec_frame_buffering
    }
    let mut nal = vec![0x67u8];
    nal.extend(w.trailing());
    nal
}

fn avc_cfg(sps: &[u8]) -> CodecConfig {
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 77,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![isobmff::AvcSps(sps.to_vec())],
            pps: vec![],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        }),
        width: 0,
        height: 0,
    }
}

fn single_track_rtp(
    id: TrackId,
    kind: RtpMediaKind,
    config: CodecConfig,
    clock: u32,
) -> RtpStreamDepacketiser {
    RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        id,
        kind,
        config,
        timescale(clock),
    )])
}

/// SPS VUI `num_units_in_tick` 1001 / `time_scale` 60000 at 90 kHz is
/// `90000 * 2 * 1001 / 60000 = 3003` ticks per frame.
#[test]
fn h264_vui_frame_period_is_the_declared_duration() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1001, 60_000, None)),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 10_000, &[0x65, 0xAA])).unwrap();
    let out = d.flush(id).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].timing.and_then(|t| t.duration()), Some(3003));
}

/// A VUI with `time_scale` 0 must not divide by zero: the duration falls back
/// to the observed step.
#[test]
fn h264_vui_with_zero_time_scale_falls_back() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1001, 0, None)),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 1000, &[0x65, 0xAA])).unwrap();
    let mut out = d.push(id, &wrap_vpkt(1, 4000, &[0x41, 0xBB])).unwrap();
    out.extend(d.flush(id).unwrap());
    assert_eq!(out[0].timing.and_then(|t| t.duration()), Some(3000));
}

/// A frame period that rounds to zero ticks is not a usable declared
/// duration: nothing is emitted without a real one.
#[test]
fn h264_vui_rounding_to_zero_ticks_declares_nothing() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 90_000_000, None)),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 1000, &[0x65, 0xAA])).unwrap();
    assert!(d.flush(id).unwrap().is_empty());
}

fn aac_asc_packet(seq: u16, ts: u32, aus: &[&[u8]]) -> Vec<u8> {
    let mut p = vec![0x80u8, 96];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0]);
    let headers_bits = (aus.len() * 16) as u16;
    p.extend_from_slice(&headers_bits.to_be_bytes());
    for au in aus {
        p.extend_from_slice(&((au.len() as u16) << 3).to_be_bytes());
    }
    for au in aus {
        p.extend_from_slice(au);
    }
    p
}

/// AAC-LC 44.1 kHz (1024 samples/frame) on a 90 kHz track is
/// `(1024 * 90000 + 22050) / 44100 = 2090` ticks.
#[test]
fn aac_asc_frame_length_scales_to_the_track_clock() {
    let id = TrackId::new(1);
    let cfg = transmux::rtp_sdp::aac_config_from_asc_bytes(vec![0x12, 0x10]).unwrap();
    let mut d = single_track_rtp(id, RtpMediaKind::Aac, cfg, 90_000);
    d.push(id, &aac_asc_packet(0, 10_000, &[&[1, 2]])).unwrap();
    let out = d.flush(id).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].timing.and_then(|t| t.duration()), Some(2090));
}

/// An SPS declaring `max_num_reorder_frames = 2` shifts decode time two frame
/// periods behind presentation time from the first sample, and an in-order
/// stream never reports `ReorderDepthExceeded`.
#[test]
fn declared_reorder_depth_shifts_dts_by_its_frame_periods() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 50, Some(2))),
        90_000,
    );
    let mut out = Vec::new();
    for (seq, ts) in [(0u16, 10_000u32), (1, 13_600), (2, 17_200)] {
        out.extend(
            d.push(
                id,
                &wrap_vpkt(
                    seq,
                    ts,
                    if seq == 0 {
                        &[0x65, 0xAA]
                    } else {
                        &[0x41, 0xBB]
                    },
                ),
            )
            .unwrap(),
        );
    }
    out.extend(d.flush(id).unwrap());
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].timing.and_then(|t| t.pts()), Some(10_000));
    assert_eq!(
        out[0].timing.and_then(|t| t.dts()),
        Some(10_000 - 2 * 3600),
        "declared depth 2 = two frame periods behind"
    );
    assert_eq!(out[1].timing.and_then(|t| t.dts()), Some(10_000 - 3600));
    assert_eq!(out[2].timing.and_then(|t| t.dts()), Some(10_000));
    assert!(d.poll_loss_event().is_none());
}

/// An odd `time_scale` distinguishes the half-denominator term from a
/// remainder: 90000 * 2 * 1001 / 60001 still rounds to 3003.
#[test]
fn h264_vui_odd_time_scale_rounds_to_the_frame_period() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1001, 60_001, None)),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 10_000, &[0x65, 0xAA])).unwrap();
    assert_eq!(
        d.flush(id).unwrap()[0].timing.and_then(|t| t.duration()),
        Some(3003)
    );
}

/// An aggregated AAC group whose forward step is smaller than its AU count
/// is emitted from the declared frame length without setting a last duration;
/// the following flush still uses the declared duration (AAC prefers it to
/// the smaller observed step).
#[test]
fn aac_declared_duration_wins_over_a_smaller_min_step_at_flush() {
    let id = TrackId::new(1);
    let cfg = transmux::rtp_sdp::aac_config_from_asc_bytes(vec![0x11, 0x90]).unwrap();
    let mut d = single_track_rtp(id, RtpMediaKind::Aac, cfg, 48_000);
    let group: [&[u8]; 3] = [&[1], &[2], &[3]];
    assert!(
        d.push(id, &aac_asc_packet(0, 10_000, &group))
            .unwrap()
            .is_empty()
    );
    let mut out = d.push(id, &aac_asc_packet(1, 10_002, &[&[4]])).unwrap();
    out.extend(d.flush(id).unwrap());
    for s in &out {
        assert_eq!(s.timing.and_then(|t| t.duration()), Some(1024));
    }
}

/// With no declared AAC frame length, the smallest positive step is the only
/// duration left, so the final pending AU is emitted with it.
#[test]
fn aac_without_declared_duration_flushes_with_the_min_step() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::Aac,
        avc_cfg(&vui_sps(1, 50, None)),
        48_000,
    );
    let group: [&[u8]; 3] = [&[1], &[2], &[3]];
    assert!(
        d.push(id, &aac_asc_packet(0, 10_000, &group))
            .unwrap()
            .is_empty()
    );
    assert!(
        d.push(id, &aac_asc_packet(1, 10_002, &[&[4]]))
            .unwrap()
            .is_empty()
    );
    let out = d.flush(id).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].timing.and_then(|t| t.duration()), Some(2));
}

// ── audit 23: flush spacing of several pending AUs ──────────────────────────

/// Three aggregated AAC AUs sharing one timestamp are spaced by the declared
/// frame length (960 samples at 48 kHz on a 96 kHz track = 1920 ticks).
#[test]
fn flush_of_aggregated_aac_aus_spaces_them_by_the_declared_duration() {
    let id = TrackId::new(1);
    let cfg = transmux::rtp_sdp::aac_config_from_asc_bytes(vec![0x11, 0x94]).unwrap();
    let mut d = single_track_rtp(id, RtpMediaKind::Aac, cfg, 96_000);
    let aus: [&[u8]; 3] = [&[1], &[2], &[3]];
    d.push(id, &aac_asc_packet(0, 10_000, &aus)).unwrap();
    let out = d.flush(id).unwrap();
    let pts: Vec<_> = out.iter().map(|s| s.timing.and_then(|t| t.pts())).collect();
    assert_eq!(pts, vec![Some(10_000), Some(11_920), Some(13_840)]);
}

/// A non-reordered H.264 track's final pending AU is emitted with the last
/// real duration (3000), not the reorder window's declared frame period
/// (3600).
#[test]
fn flush_of_a_non_reordered_h264_pending_au_uses_the_known_duration() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 50, None)),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 10_000, &[0x65, 0xAA])).unwrap();
    let mut out = d.push(id, &wrap_vpkt(1, 13_000, &[0x41, 0xBB])).unwrap();
    out.extend(d.flush(id).unwrap());
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].timing.and_then(|t| t.duration()), Some(3000));
}

// ── audit 23: reorder-window release thresholds, pruning, drop bound ────────

/// With a known frame period and a head whose successor is further away than
/// one period, the window releases only once pending exceeds
/// `MAX_REORDER_DEPTH` (16).
#[test]
fn reorder_release_waits_for_pending_to_exceed_max_reorder_depth() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 50, Some(0))),
        90_000,
    );
    let mut emitted = 0usize;
    for k in 0..16u32 {
        let sal = if k == 0 { &[0x65, 0xAA] } else { &[0x41, 0xBB] };
        emitted += d
            .push(id, &wrap_vpkt(k as u16, 40_000 - k * 1000, sal))
            .unwrap()
            .len();
    }
    assert_eq!(emitted, 0, "16 pending is not past the depth bound");
    emitted += d
        .push(id, &wrap_vpkt(16, 40_000 - 16 * 1000, &[0x41, 0xBB]))
        .unwrap()
        .len();
    assert_eq!(
        emitted, 16,
        "17 pending trips the bound and the window resolves, less the unit held for its duration"
    );
    emitted += d.flush(id).unwrap().len();
    assert_eq!(emitted, 17, "the flush emits the held unit");
}

/// Resolving the window prunes `seen_pts` only below the oldest pending AU:
/// the second AU resolved in the same call still finds its successor.
#[test]
fn resolving_window_prunes_only_below_the_oldest_pending_au() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 50, Some(0))),
        90_000,
    );
    d.push(id, &wrap_vpkt(0, 10_000, &[0x65, 0xAA])).unwrap();
    d.push(id, &wrap_vpkt(1, 8_000, &[0x41, 0xBB])).unwrap();
    let mut out = d.push(id, &wrap_vpkt(2, 11_000, &[0x41, 0xCC])).unwrap();
    out.extend(d.flush(id).unwrap());
    let got: Vec<_> = out
        .iter()
        .map(|s| {
            (
                s.timing.and_then(|t| t.pts()),
                s.timing.and_then(|t| t.duration()),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (Some(10_000), Some(1)),
            (Some(8_000), Some(1)),
            (Some(11_000), Some(1))
        ]
    );
}

/// Pending AUs are dropped only once the window exceeds 256, oldest first.
#[test]
fn reorder_window_drops_only_beyond_max_reorder_window() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(id, RtpMediaKind::H264, tiny_avc_config(), 90_000);
    let push = |d: &mut RtpStreamDepacketiser, k: u32| {
        let sal = if k == 0 { &[0x65, 0xAA] } else { &[0x41, 0xBB] };
        d.push(id, &wrap_vpkt(k as u16, 400_000 - k * 1000, sal))
            .unwrap();
    };
    for k in 0..256u32 {
        push(&mut d, k);
    }
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    assert!(dropped_pts(&events).is_empty(), "256 pending is in bounds");
    push(&mut d, 256);
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    assert_eq!(
        dropped_pts(&events),
        vec![400_000],
        "the oldest AU is dropped past the bound"
    );
}

// ── audit 23: non-monotonic events, min step, reorder bookkeeping ───────────

fn pts_and_durations(samples: &[transmux::pipeline::Sample]) -> Vec<(i64, u32)> {
    samples
        .iter()
        .map(|s| {
            (
                s.timing.and_then(|t| t.pts()).expect("timed sample"),
                s.timing.and_then(|t| t.duration()).expect("duration"),
            )
        })
        .collect()
}

/// A repeated timestamp is not a reorder: no `NonMonotonicTimestamp`, and each
/// AU keeps the shared frame duration.
#[test]
fn repeated_timestamp_is_not_a_non_monotonic_event() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(
        id,
        RtpMediaKind::H264,
        avc_cfg(&vui_sps(1, 50, Some(0))),
        90_000,
    );
    let mut out = Vec::new();
    let mut per_push = Vec::new();
    for (seq, ts) in [(0u16, 0u32), (1, 3000), (2, 3000), (3, 6000)] {
        let sal = if seq == 0 {
            &[0x65, 0xAA]
        } else {
            &[0x41, 0xBB]
        };
        let s = d.push(id, &wrap_vpkt(seq, ts, sal)).unwrap();
        per_push.push(s.len());
        out.extend(s);
    }
    assert_eq!(
        per_push,
        vec![0, 1, 1, 1],
        "an equal timestamp still emits the previous AU immediately"
    );
    out.extend(d.flush(id).unwrap());
    assert_eq!(
        pts_and_durations(&out),
        vec![(0, 3000), (3000, 3000), (3000, 3000), (6000, 3000)]
    );
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RtpLossEvent::NonMonotonicTimestamp { .. })),
        "an equal timestamp is not a backward step: {events:?}"
    );
}

/// An I P B B wire order reports exactly one backward step and yields the
/// presentation-ordered durations, with the smallest positive step (1500,
/// not the first 12000 or 3000) as the final head's duration.
#[test]
fn reordered_wire_order_min_step_drives_the_resolved_duration() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(id, RtpMediaKind::H264, tiny_avc_config(), 90_000);
    let mut out = Vec::new();
    for (seq, ts) in [
        (0u16, 0u32),
        (1, 12_000),
        (2, 6_000),
        (3, 9_000),
        (4, 10_500),
    ] {
        let sal = if seq == 0 {
            &[0x65, 0xAA]
        } else {
            &[0x41, 0xBB]
        };
        out.extend(d.push(id, &wrap_vpkt(seq, ts, sal)).unwrap());
    }
    out.extend(d.flush(id).unwrap());
    assert_eq!(
        pts_and_durations(&out),
        vec![
            (0, 1500),
            (12_000, 6000),
            (6_000, 3000),
            (9_000, 1500),
            (10_500, 1500)
        ]
    );
    let mut events = Vec::new();
    while let Some(e) = d.poll_loss_event() {
        events.push(e);
    }
    let non_monotonic: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RtpLossEvent::NonMonotonicTimestamp { .. }))
        .collect();
    assert_eq!(
        non_monotonic,
        vec![&RtpLossEvent::NonMonotonicTimestamp {
            track_id: id,
            previous_pts: 12_000,
            pts: 6_000,
        }]
    );
}

/// The frame-period estimate is the *smallest* positive step, not the last:
/// after a 1000 then 2000 step, an unresolved head takes 1000.
#[test]
fn min_step_keeps_the_smallest_positive_step() {
    let id = TrackId::new(1);
    let mut d = single_track_rtp(id, RtpMediaKind::H264, tiny_avc_config(), 90_000);
    let mut out = Vec::new();
    for (seq, ts) in [(0u16, 5000u32), (1, 1000), (2, 2000), (3, 4000)] {
        let sal = if seq == 0 {
            &[0x65, 0xAA]
        } else {
            &[0x41, 0xBB]
        };
        out.extend(d.push(id, &wrap_vpkt(seq, ts, sal)).unwrap());
    }
    out.extend(d.flush(id).unwrap());
    assert_eq!(
        pts_and_durations(&out),
        vec![(5000, 1000), (1000, 1000), (2000, 2000), (4000, 1000)]
    );
}

// ── audit 23: aggregated AAC group duration split ───────────────────────────

fn aac_1024_track() -> (TrackId, RtpStreamDepacketiser) {
    let id = TrackId::new(1);
    let cfg = transmux::rtp_sdp::aac_config_from_asc_bytes(vec![0x11, 0x90]).unwrap();
    (id, single_track_rtp(id, RtpMediaKind::Aac, cfg, 48_000))
}

fn durations(samples: &[transmux::pipeline::Sample]) -> Vec<u32> {
    samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).expect("duration"))
        .collect()
}

/// A group's forward step is the delta even when a smaller step was seen
/// earlier: 3000 over three AUs is 1000 each, not a share of the 1000-tick
/// minimum.
#[test]
fn aggregated_group_uses_the_forward_step_not_a_smaller_earlier_step() {
    let (id, mut d) = aac_1024_track();
    d.push(id, &aac_asc_packet(0, 10_000, &[&[1]])).unwrap();
    d.push(id, &aac_asc_packet(1, 11_000, &[&[2]])).unwrap();
    d.push(id, &aac_asc_packet(2, 14_000, &[&[3], &[4], &[5]]))
        .unwrap();
    d.push(id, &aac_asc_packet(3, 17_000, &[&[6]])).unwrap();
    let out = d.push(id, &aac_asc_packet(4, 20_000, &[&[7]])).unwrap();
    assert_eq!(durations(&out), vec![1000, 1000, 1000]);
}

/// A group delta that does not divide evenly leaves the remainder on the last
/// AU: 3001 over three AUs is `[1000, 1000, 1001]`.
#[test]
fn aggregated_group_gives_the_remainder_to_the_last_au() {
    let (id, mut d) = aac_1024_track();
    d.push(id, &aac_asc_packet(0, 10_000, &[&[1], &[2], &[3]]))
        .unwrap();
    d.push(id, &aac_asc_packet(1, 13_001, &[&[4]])).unwrap();
    let out = d.push(id, &aac_asc_packet(2, 16_000, &[&[5]])).unwrap();
    assert_eq!(durations(&out), vec![1000, 1000, 1001]);
}

/// A repeated/backward timestamp has no forward delta: with no step seen, the
/// declared frame length times the AU count is split evenly.
#[test]
fn aggregated_group_without_a_forward_step_uses_the_declared_length() {
    let (id, mut d) = aac_1024_track();
    d.push(id, &aac_asc_packet(0, 10_000, &[&[1], &[2]]))
        .unwrap();
    d.push(id, &aac_asc_packet(1, 8_000, &[&[3]])).unwrap();
    let out = d.push(id, &aac_asc_packet(2, 9_000, &[&[4]])).unwrap();
    assert_eq!(durations(&out), vec![1024, 1024]);
}

/// With a known smallest step and no forward delta, that step (split over the
/// group) wins over the declared frame length.
#[test]
fn aggregated_group_without_a_forward_step_uses_the_min_step() {
    let (id, mut d) = aac_1024_track();
    d.push(id, &aac_asc_packet(0, 10_000, &[&[1]])).unwrap();
    d.push(id, &aac_asc_packet(1, 12_000, &[&[2]])).unwrap();
    d.push(id, &aac_asc_packet(2, 14_000, &[&[3], &[4]]))
        .unwrap();
    d.push(id, &aac_asc_packet(3, 8_000, &[&[5]])).unwrap();
    let out = d.push(id, &aac_asc_packet(4, 9_000, &[&[6]])).unwrap();
    assert_eq!(durations(&out), vec![1000, 1000]);
}
