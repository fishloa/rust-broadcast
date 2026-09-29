//! Real-fixture gate for the streaming RTP depayloader (#700).
//!
//! Demuxes the real `h264_aac.ts` fixture, packetises it to RTP (the same
//! `TsDemux` + `RtpPacketiser::package` calls `tests/rtp.rs` uses), then feeds
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

use broadcast_common::{Package, Unpackage};
use std::collections::HashSet;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::{aac_config_from_asc_hex, avc_config_from_sprop};
use transmux::{
    FragmentTrackData, Media, RtpLossEvent, RtpOutput, RtpPacketiser, RtpStream,
    RtpStreamDepacketiser, RtpStreamTrack, Severity, TsDemux, build_init_segment,
    build_media_segment, validate_init_segment, validate_media_segment,
};

const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;

// ── Step 0 plumbing, copied verbatim from tests/rtp.rs ──────────────────────

fn demux_fixture() -> Media {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let data = std::fs::read(path).expect("h264_aac.ts fixture must exist");
    let mut demux = TsDemux::new();
    demux.unpackage(&data[..]).expect("demux TS → IR")
}

fn packetise(media: &Media) -> RtpOutput {
    let mut p = RtpPacketiser {
        mtu: MTU,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    p.package(media).expect("packetise IR → RTP")
}

fn video_stream(out: &RtpOutput) -> &RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap()
}

fn audio_stream(out: &RtpOutput) -> &RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .unwrap()
}

// Pull one fmtp attribute value ("sprop-parameter-sets=" / "config=") out of an
// SDP string. Test-only crude extraction (no sdp-types dependency in transmux).
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
    let sprop = fmtp_value(&out.sdp, "sprop-parameter-sets=").expect("sprop");
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
        1,
        RtpMediaKind::H264,
        CodecConfig::Avc {
            config: avc.clone(),
            width: 0,
            height: 0,
        },
        90_000,
    )]);
    let mut recovered = Vec::new();
    for pkt in &video_stream.packets {
        let c = pkt.as_contiguous();
        recovered.extend(d.push(1, &c).unwrap());
    }
    recovered.extend(d.flush(1).unwrap());

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
    // Total duration must match the *transmitted* presentation series, which
    // is the only timeline RTP carries: each AU's duration is the forward step
    // to the next AU's wire timestamp, so the sum is the span of that series.
    // It is NOT the demuxed track's own per-frame duration sum, because the
    // fixture is a real B-frame encode — its wire timestamps are in
    // presentation order, so their forward steps are distances to the next
    // presentation instant rather than frame periods (audit r04-W29; the batch
    // depacketiser is the path that recovers exact per-frame durations, since
    // it sees the whole stream).
    // Literal expectations, read off the fixture's own transmitted timestamp
    // series (`h264_aac.ts` is 75 access units at 90 kHz): the duration of
    // each AU is its forward step to the next presentation instant, a
    // backward step (the fixture is a B-frame encode) falls back to the
    // smallest positive step seen so far, and the AU still open at end of
    // stream reuses the last computed duration. The first twelve durations and
    // the total are asserted as numbers so this test cannot pass by
    // re-implementing the depacketiser's rule and comparing it with itself.
    const EXPECTED_FIRST_DURATIONS: [u32; 12] = [
        14400, 14400, 14400, 7200, 18000, 7200, 7200, 7200, 18000, 7200, 7200, 7200,
    ];
    const EXPECTED_TOTAL: u64 = 626_400;
    let got_durations: Vec<u32> = recovered.iter().map(|s| s.duration.unwrap_or(0)).collect();
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
    let cfg_hex = fmtp_value(&out.sdp, "config=")
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
    let init = build_init_segment(&specs, 90_000).expect("build_init_segment must succeed");
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
    let seg1 = build_media_segment(1, &[FragmentTrackData::new(1, 0, part1)])
        .expect("build_media_segment (part 1) must succeed");
    let part1_total: u64 = part1
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum();
    let seg2 = build_media_segment(2, &[FragmentTrackData::new(1, part1_total, part2)])
        .expect("build_media_segment (part 2) must succeed");

    for (label, seg) in [("part 1", &seg1), ("part 2", &seg2)] {
        assert!(!seg.is_empty(), "{label} segment non-empty");
        let issues = validate_media_segment(seg);
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
// (`video_stream(&out).packets`, real NAL content from `h264_aac.ts`) by
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
/// packets for the video stream, from the shared `h264_aac.ts` fixture.
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
/// this fixture (`h264_aac.ts`) is a real B-frame encode, so a faithful
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
        RtpStreamTrack::new(1, RtpMediaKind::H264, config.clone(), 90_000)
            .with_reorder_depth(reorder_depth),
    ]);
    let mut out = Vec::new();
    for pkt in pkts {
        out.extend(d.push(1, pkt).unwrap());
    }
    out.extend(d.flush(1).unwrap());
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
        RtpStreamTrack::new(1, RtpMediaKind::H264, vconfig, 90_000),
        // Clock rate is irrelevant to sequence-gate correctness (only
        // timing derivation, not under test here).
        RtpStreamTrack::new(2, RtpMediaKind::Aac, aconfig, 48_000),
    ]);

    for pkt in &video_stream(&out).packets {
        let c = pkt.as_contiguous();
        d.push(1, &c).unwrap();
    }
    d.flush(1).unwrap();
    for pkt in &audio_stream(&out).packets {
        let c = pkt.as_contiguous();
        d.push(2, &c).unwrap();
    }
    d.flush(2).unwrap();

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
    use transmux::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
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
        1,
        RtpMediaKind::H264,
        config,
        90_000,
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
        recovered.extend(d.push(1, &wrap_vpkt(*seq, ts, &nals[i])).unwrap());
    }
    recovered.extend(d.flush(1).unwrap());

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
        RtpStreamTrack::new(1, RtpMediaKind::H264, config, 90_000).with_reorder_depth(DEPTH),
    ]);

    // Establish `expected` with one in-order packet (seq 0).
    d.push(1, &wrap_vpkt(0, 1000, &[0x65, 0xAA])).unwrap();

    for k in 1..=FLOOD_LEN {
        let seq = 1000u16.wrapping_add(k.wrapping_mul(50));
        d.push(1, &wrap_vpkt(seq, 1000, &[0x41, 0xBB])).unwrap();
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
        1,
        RtpMediaKind::H264,
        tiny_avc_config(),
        90_000,
    )]);
    let mut out = Vec::new();
    for (i, &(seq, ts)) in pairs.iter().enumerate() {
        out.extend(d.push(1, &seq_pkt(seq, ts, i as u8)).unwrap());
    }
    out.extend(d.flush(1).unwrap());
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
    // Exactly: the pre-seek run's first three AUs (bytes 0..2), then the new
    // run's first packet discarded arming `bad_seq` (byte 4, i.e. run index
    // 0) and every following packet of it delivered. The pre-seek AU open at
    // the seam (byte 3) is dropped.
    assert_eq!(
        recovered.len(),
        3 + (RUN as usize - 1),
        "the whole new run must be delivered, got {}",
        recovered.len()
    );
    // Spot-check identity at the wrap: the run crosses 65535 -> 0.
    let first_new = 3;
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
        [0x00, 0x00, 0x00, 0x02, 0x41, 2u8],
        "the last pre-seek AU delivered is the third packet's"
    );
    // The pre-seek AU open at the seam (index 3) is dropped: the delivered
    // prefix is the first three packets only. (A `contains` check on its byte
    // would not mean anything here — the identifying byte is the packet index
    // modulo 256, so 3 recurs later in a 70 000-packet run.)
    assert_eq!(
        &recovered[..first_new],
        &[
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 0],
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 1],
            vec![0x00u8, 0x00, 0x00, 0x02, 0x41, 2],
        ][..],
        "only the pre-seek packets with known durations are delivered"
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
    // Exactly: run 1's packets whose duration is known (indices 0..4), then
    // seek 1's arming packet dropped (index 6) and the rest delivered
    // (7..24), then seek 2's arming packet dropped (index 25) and the rest
    // delivered (27..45). The AUs open at each seam (index 5, and between 24
    // and 25) are dropped by the resync.
    let expected: Vec<Vec<u8>> = (0..5u8)
        .chain(7..25u8)
        .chain(27..46u8)
        .map(|b| vec![0x00, 0x00, 0x00, 0x02, 0x41, b])
        .collect();
    assert_eq!(
        recovered, expected,
        "both post-seek runs must be delivered, with only the arming packets \
         and the seam AUs dropped"
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
    // Exactly: the pre-jump run's AUs whose durations were known (bytes 0..4),
    // the first new-run packet discarded while §A.1's `bad_seq` armed (byte 6),
    // and then every following packet of the restarted run (bytes 7..17). The
    // pre-jump AU still open at the seam (byte 5) is dropped, as a resync
    // always drops the access unit straddling it.
    let expected: Vec<Vec<u8>> = (0..5u8)
        .chain(7..18u8)
        .map(|b| vec![0x00, 0x00, 0x00, 0x02, 0x41, b])
        .collect();
    assert_eq!(
        recovered, expected,
        "the restarted run must be delivered with only the arming packet and          the seam-straddling AU dropped"
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
