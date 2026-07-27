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
    let orig_video_total: u64 = orig_video
        .samples
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum();

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
        recovered.extend(d.push(1, pkt).unwrap());
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
    // Total duration within one frame of the original (one-AU flush tolerance).
    let rec_total: u64 = recovered
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum();
    let frame = orig_video
        .samples
        .first()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .unwrap_or(3000);
    assert!(
        rec_total.abs_diff(orig_video_total) <= frame,
        "total duration {rec_total} vs {orig_video_total}"
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
    let packets = video_stream(&out).packets.clone();
    (config, originals, packets)
}

/// Feed `pkts` through a fresh depayloader, returning the recovered samples'
/// byte data (in emission order) and whether any loss event was raised.
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
    let had_loss = d.poll_loss_event().is_some();
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
        d.push(1, pkt).unwrap();
    }
    d.flush(1).unwrap();
    for pkt in &audio_stream(&out).packets {
        d.push(2, pkt).unwrap();
    }
    d.flush(2).unwrap();

    assert!(
        d.poll_loss_event().is_none(),
        "a clean capture must emit zero loss signals end to end"
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
