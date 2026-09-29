//! RTP spoke gate — packetise/depacketise the demuxed `fixtures/ts/h264_aac.ts`
//! IR (75 video + 131 audio samples) and verify RFC 3550/6184/3640/4566 fidelity
//! against the real demuxed NALs / config (issue #469).
//!
//! Every test bites against the demuxed oracle, never hardcoded values.

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::rtp::{base64_decode, hex_decode};
use transmux::{
    Media, NAL_TYPE_IDR, RtpDepacketiser, RtpInput, RtpInputStream, RtpMediaKind, RtpPacket,
    RtpPacketiser, VIDEO_CLOCK_RATE,
};

const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;
const RTP_HEADER_LEN: usize = 12;

// ── Fixture demux ────────────────────────────────────────────────────────────

fn demux_fixture() -> Media {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let data = std::fs::read(path).expect("h264_aac.ts fixture must exist");
    let mut demux = transmux::TsDemux::new();
    demux.unpackage(&data[..]).expect("demux TS → IR")
}

fn packetise(media: &Media) -> transmux::RtpOutput {
    let mut p = RtpPacketiser {
        mtu: MTU,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    p.package(media).expect("packetise IR → RTP")
}

fn parse_hdr(pkt: &RtpPacket) -> (u8, u8, bool, u16, u32, u32) {
    let h = &pkt.header;
    let version = h[0] >> 6;
    let marker = h[1] & 0x80 != 0;
    let pt = h[1] & 0x7F;
    let seq = u16::from_be_bytes([h[2], h[3]]);
    let ts = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
    let ssrc = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
    (version, pt, marker, seq, ts, ssrc)
}

/// Original demuxed NAL payloads of every video AU (length prefixes stripped).
fn original_video_nals(media: &Media) -> Vec<Vec<Vec<u8>>> {
    let vt = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    vt.samples
        .iter()
        .map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect()
        })
        .collect()
}

fn video_stream(out: &transmux::RtpOutput) -> &transmux::RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap()
}

fn audio_stream(out: &transmux::RtpOutput) -> &transmux::RtpStream {
    out.streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .unwrap()
}

// ── Test 1: valid RTP headers, monotonic seq, per-AU shared TS + marker ──────

#[test]
fn valid_rtp_headers_and_marker_semantics() {
    let media = demux_fixture();
    let out = packetise(&media);

    for stream in &out.streams {
        assert!(!stream.packets.is_empty(), "stream has packets");
        // Every packet: V=2, correct PT, fixed SSRC; strictly monotonic seq (+1).
        let mut expected_seq: Option<u16> = None;
        for pkt in &stream.packets {
            assert!(pkt.header.len() >= RTP_HEADER_LEN);
            let (v, pt, _m, seq, _ts, ssrc) = parse_hdr(pkt);
            assert_eq!(v, 2, "RTP version must be 2");
            assert_eq!(pt, stream.pt, "payload type matches the stream PT");
            assert_eq!(ssrc, SSRC, "fixed SSRC");
            if let Some(prev) = expected_seq {
                assert_eq!(seq, prev, "sequence numbers strictly +1");
            }
            expected_seq = Some(seq.wrapping_add(1));
        }
    }

    // Video: group packets by AU using the marker bit; every packet within an AU
    // shares a timestamp; the marker is set on exactly the last packet of the AU;
    // the AU timestamps advance by the per-frame 90 kHz delta (3600).
    let vs = video_stream(&out);
    // Skip the leading STAP-A parameter-set packet (marker=0, its own TS group).
    let mut aus: Vec<Vec<&RtpPacket>> = Vec::new();
    let mut cur: Vec<&RtpPacket> = Vec::new();
    // The STAP-A is the first packet and has no marker; treat everything up to
    // and including each marker as one AU (STAP-A then rides with the first AU's
    // timestamp group, but it is emitted before frame 0 with timestamp 0 too).
    for pkt in &vs.packets {
        let (_v, _pt, marker, _seq, _ts, _ssrc) = parse_hdr(pkt);
        cur.push(pkt);
        if marker {
            aus.push(std::mem::take(&mut cur));
        }
    }
    assert!(cur.is_empty(), "every AU ends with a marker packet");
    assert_eq!(aus.len(), 75, "75 video access units delimited by markers");

    // The fixture's own access-unit timestamps, in presentation order: the
    // demuxed samples' `pts`, rescaled to the RTP 90 kHz clock and relative to
    // the first. `h264_aac.ts` is a B-frame encode, so this series is NOT a
    // uniform +3600 step — asserting one would be asserting that the
    // packetiser stamped decode times (audit r04-W29).
    let video_track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let first_pts = video_track.samples[0].pts.expect("timed sample");
    let ts_of = |tick: i64| -> u32 {
        let rel = u64::try_from(tick - first_pts).expect("presentation order");
        u32::try_from(rel * u64::from(VIDEO_CLOCK_RATE) / u64::from(video_track.spec.timescale))
            .expect("fits 32 bits")
    };
    let expected: Vec<u32> = video_track
        .samples
        .iter()
        .map(|s| ts_of(s.pts.expect("timed")))
        .collect();
    assert_eq!(expected.len(), aus.len(), "one AU per demuxed sample");

    for (i, au) in aus.iter().enumerate() {
        // All packets in the AU share one timestamp.
        let ts0 = parse_hdr(au[0]).4;
        for pkt in au {
            assert_eq!(parse_hdr(pkt).4, ts0, "AU packets share a timestamp");
        }
        // Marker set on exactly the last packet.
        for (i, pkt) in au.iter().enumerate() {
            let marker = parse_hdr(pkt).2;
            assert_eq!(
                marker,
                i == au.len() - 1,
                "marker set on exactly the last packet of the AU"
            );
        }
        // Timestamps are the presentation times (RFC 6184 §5.1), one per AU.
        // The leading STAP-A parameter-set packet shares AU 0's timestamp, so
        // it is inside `au[0]`'s group and does not shift the series.
        assert_eq!(
            ts0, expected[i],
            "AU {i} must carry its presentation timestamp"
        );
    }
    // Sanity: the series really is non-uniform (a B-frame reorder), so the
    // equality above is not vacuous.
    assert!(
        expected.windows(2).any(|w| w[1] < w[0]),
        "fixture must be reordered for this assertion to mean anything"
    );

    // Audio: one packet per AU, marker set, timestamps advance by 1024 ticks.
    let as_ = audio_stream(&out);
    assert_eq!(as_.packets.len(), 131, "131 audio packets (one AU each)");
    let mut prev_a: Option<u32> = None;
    for pkt in &as_.packets {
        let (_v, _pt, marker, _seq, ts, _ssrc) = parse_hdr(pkt);
        assert!(marker, "audio marker set per packet");
        if let Some(p) = prev_a {
            // The RTP timestamp carries the real recovered decode time (media
            // plane step 2c), in the track's own timescale (44.1 kHz for this
            // AAC track) — the same unit an AAC frame's intrinsic duration
            // (1024 samples) is exact in. Issue B5 (media plane step-2 fix
            // wave 1): the demuxer used to re-derive each access unit's dts
            // from the lossy 90 kHz PES clock (1024 * 90000 / 44100 =
            // 2089.79... ticks, not an integer), injecting a spurious ±1 tick
            // at every PES boundary; it now anchors once and advances by the
            // intrinsic per-frame duration, so every delta is exactly 1024 —
            // fixing the demuxer, not relaxing this assertion, is the fix.
            let d = ts - p;
            assert_eq!(
                d, 1024,
                "audio TS must advance by exactly the AAC frame length (1024 \
                 samples) — a demuxer re-deriving dts from the lossy 90 kHz \
                 PES clock per access unit would drift by ±1 tick here"
            );
        }
        prev_a = Some(ts);
    }
}

// ── Test 2: FU-A fragmentation actually happens ──────────────────────────────

#[test]
fn fu_a_fragmentation_happens() {
    let media = demux_fixture();
    let out = packetise(&media);
    let vs = video_stream(&out);

    // Find FU-A packets (FU indicator is in the header at offset RTP_HEADER_LEN;
    // its low 5 bits carry the FU-A type = 28).
    let mut fu_packets = 0usize;
    let mut fu_starts = 0usize;
    let mut fu_ends = 0usize;
    let mut reconstructed_types = Vec::new();
    for pkt in &vs.packets {
        let hdr = &pkt.header;
        if hdr.len() <= RTP_HEADER_LEN {
            continue; // single-NAL packet — no FU indicator
        }
        let fu_indicator = hdr[RTP_HEADER_LEN];
        let nal_type = fu_indicator & 0x1F;
        if nal_type == 28 {
            fu_packets += 1;
            let fu_header = hdr[RTP_HEADER_LEN + 1];
            let s = fu_header & 0x80 != 0;
            let e = fu_header & 0x40 != 0;
            if s {
                fu_starts += 1;
                reconstructed_types.push(fu_header & 0x1F);
            }
            if e {
                fu_ends += 1;
            }
            // A fragment cannot be both S and E in a real multi-fragment NAL.
            if s {
                assert!(!e, "start fragment is not also the end (>=2 fragments)");
            }
        }
    }
    assert!(fu_packets >= 2, "at least 2 FU-A packets emitted");
    assert!(fu_starts >= 1, "at least one FU-A start (S) fragment");
    assert_eq!(
        fu_starts, fu_ends,
        "each fragmented NAL has one S and one E"
    );
    // The demuxed IDR slices (type 5) are the large NALs that fragment.
    assert!(
        reconstructed_types.contains(&NAL_TYPE_IDR),
        "a fragmented NAL reconstructs to an IDR slice (type {NAL_TYPE_IDR})"
    );

    // Cross-check against the oracle: the count of AUs that contain a NAL larger
    // than the MTU budget must equal the number of FU-A start fragments.
    let originals = original_video_nals(&media);
    let big_nals = originals
        .iter()
        .flat_map(|au| au.iter())
        .filter(|n| n.len() + RTP_HEADER_LEN > MTU)
        .count();
    assert_eq!(
        fu_starts, big_nals,
        "one FU-A start per over-MTU NAL in the demuxed IR"
    );
}

// ── Test 3: video round-trip byte-identical ──────────────────────────────────

#[test]
fn video_round_trip_byte_identical() {
    let media = demux_fixture();
    let out = packetise(&media);
    let vs = video_stream(&out);

    let mut depack = RtpDepacketiser::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(
                RtpMediaKind::H264,
                vs.packets
                    .iter()
                    .map(|p| p.as_contiguous().to_vec())
                    .collect(),
            )],
        })
        .expect("depacketise video");

    // The reassembled access units' NAL payloads must be byte-identical to the
    // original demuxed video sample NALs, sample-for-sample.
    let originals = original_video_nals(&media);
    let rebuilt: Vec<Vec<Vec<u8>>> = ir.tracks[0]
        .samples
        .iter()
        .map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect()
        })
        .collect();

    // The first depacketised AU carries the STAP-A parameter sets (SPS+PPS)
    // prepended to frame 0's NALs; compare the tail (per-frame VCL NALs) against
    // the originals, and verify the parameter sets survived in the first AU.
    assert_eq!(
        rebuilt.len(),
        originals.len(),
        "75 reassembled access units"
    );
    let sps = match &media.tracks[0].spec.config {
        CodecConfig::Avc { config, .. } => config.config.sps[0].0.clone(),
        _ => unreachable!(),
    };
    let pps = match &media.tracks[0].spec.config {
        CodecConfig::Avc { config, .. } => config.config.pps[0].0.clone(),
        _ => unreachable!(),
    };
    // Frame 0's rebuilt NALs = [SPS, PPS, <original frame-0 NALs...>].
    assert_eq!(rebuilt[0][0], sps, "SPS reassembled first");
    assert_eq!(rebuilt[0][1], pps, "PPS reassembled second");
    assert_eq!(
        &rebuilt[0][2..],
        &originals[0][..],
        "frame 0 VCL NALs byte-identical"
    );
    for i in 1..originals.len() {
        assert_eq!(rebuilt[i], originals[i], "AU {i} NALs byte-identical");
    }
}

// ── Test 4: audio round-trip byte-identical ──────────────────────────────────

#[test]
fn audio_round_trip_byte_identical() {
    let media = demux_fixture();
    let out = packetise(&media);
    let as_ = audio_stream(&out);
    let audio_config = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .spec
        .config
        .clone();

    let mut depack = RtpDepacketiser::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(
                    RtpMediaKind::Aac,
                    as_.packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_config(audio_config),
            ],
        })
        .expect("depacketise audio");

    let audio_track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap();

    assert_eq!(ir.tracks[0].samples.len(), 131, "131 reassembled AUs");
    for (i, (rebuilt, orig)) in ir.tracks[0]
        .samples
        .iter()
        .zip(audio_track.samples.iter())
        .enumerate()
    {
        assert_eq!(rebuilt.data, orig.data, "audio AU {i} byte-identical");
    }

    // The AU-headers-length / AU-size math must be exact: mutating a size byte
    // in the header breaks reassembly (proves the size field is honoured).
    let pkt0 = &as_.packets[0];
    // Build a contiguous copy to mutate — the AAC-hbr header bytes are at
    // the tail of `pkt0.header`, and the AU-header is at `header[RTP_HEADER_LEN + 2]`.
    let broken = pkt0.as_contiguous();
    let broken_vec = {
        let mut v = broken.to_vec();
        // AU-header sits at payload offset [2..4]; corrupt the AU-size (top 13 bits).
        v[RTP_HEADER_LEN + 2] ^= 0x08;
        v
    };
    let mut d2 = RtpDepacketiser::new();
    let bad = d2.unpackage(RtpInput {
        streams: vec![
            RtpInputStream::new(RtpMediaKind::Aac, vec![broken_vec]).with_config(
                match &media.tracks[1].spec.config {
                    CodecConfig::Aac { .. } => media.tracks[1].spec.config.clone(),
                    _ => unreachable!(),
                },
            ),
        ],
    });
    // Either it errors (declared size overran) or the reassembled AU differs.
    if let Ok(m) = bad {
        assert_ne!(
            m.tracks[0].samples[0].data, audio_track.samples[0].data,
            "corrupt AU-size must not reproduce the original AU"
        );
    }
}

// ── Test 4b: the depacketised IR describes each stream's own kind/clock ──────

/// The batch depacketiser's output must describe each stream as what it is:
/// its own payload format and its own RTP clock rate (audit r04-W30).
///
/// An RTP timestamp is a count in the *stream's* clock (RFC 3550 §5.1), so a
/// track declared at the 90 kHz video clock while carrying audio sample-rate
/// timestamps states every duration ~1.9x wrong (48000 -> 90000), and a track
/// declared as AVC while carrying AAC makes a consumer write a video sample
/// entry for it. Both are asserted against the demuxed oracle's own config.
#[test]
fn depacketised_tracks_carry_their_own_kind_clock_and_config() {
    let media = demux_fixture();
    let out = packetise(&media);
    let audio_config = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .spec
        .config
        .clone();
    let (rate, channels) = match &audio_config {
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => (*sample_rate, *channel_count),
        _ => unreachable!(),
    };

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(
                    RtpMediaKind::H264,
                    video_stream(&out)
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_clock_rate(VIDEO_CLOCK_RATE),
                RtpInputStream::new(
                    RtpMediaKind::Aac,
                    audio_stream(&out)
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_clock_rate(rate)
                .with_config(audio_config),
            ],
        })
        .expect("depacketise both streams");

    // Video: the 90 kHz clock, and an AVC config.
    assert_eq!(
        ir.tracks[0].spec.timescale, VIDEO_CLOCK_RATE,
        "the video track's timescale is the RTP video clock"
    );
    assert!(
        matches!(ir.tracks[0].spec.config, CodecConfig::Avc { .. }),
        "the video track carries an AVC config"
    );

    // Audio: its OWN sample-rate clock, and an AAC config — not AVC at 90 kHz
    // with the sample rate lost, which is what the placeholder wrote before.
    assert_eq!(
        ir.tracks[1].spec.timescale, rate,
        "the audio track's timescale is the stream's audio clock, not 90 kHz"
    );
    match &ir.tracks[1].spec.config {
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => {
            assert_eq!(*sample_rate, rate, "the audio config's rate survives");
            assert_eq!(*channel_count, channels, "and its channel count");
        }
        other => panic!("audio track must carry an AAC config, got {other:?}"),
    }

    // Durations are therefore in *audio* ticks, not 90 kHz ones: the 131 AUs
    // of the fixture's AAC stream at 1024 samples/frame sum to ~130 * 1024 by
    // the one-behind duration rule. At 90 kHz the same byte stream would have
    // summed to ~130 * 1920.
    let total: u64 = ir.tracks[1]
        .samples
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
        .sum();
    const AAC_SAMPLES_PER_FRAME: u64 = 1024;
    const AUDIO_AUS: u64 = 131;
    let in_audio_ticks = AUDIO_AUS * AAC_SAMPLES_PER_FRAME;
    // 48000 -> 90000 would have scaled this by 1.875; assert we are within one
    // frame of the sample-rate count and nowhere near the rescaled one.
    assert!(
        total.abs_diff(in_audio_ticks) <= AAC_SAMPLES_PER_FRAME,
        "audio durations must be counted in audio ticks (expected ~{in_audio_ticks}, got {total})"
    );
}

/// An AAC stream with no codec config must be rejected, not described with an
/// invented one — RTP carries no config, so it has to come from the SDP
/// (RFC 3640 §4.1). The old placeholder wrote an AVC config with a 90 kHz
/// clock instead, i.e. a track claiming to be video.
#[test]
fn aac_stream_without_config_is_rejected() {
    let media = demux_fixture();
    let out = packetise(&media);
    let err = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(
                RtpMediaKind::Aac,
                audio_stream(&out)
                    .packets
                    .iter()
                    .map(|p| p.as_contiguous().to_vec())
                    .collect(),
            )],
        })
        .expect_err("an AAC stream with no AudioSpecificConfig cannot be described");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "must be a structured input error, got {err:?}"
    );
}

/// A zero clock rate is rejected: an RTP timestamp has no unit without it.
#[test]
fn zero_clock_rate_is_rejected() {
    let media = demux_fixture();
    let out = packetise(&media);
    let err = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(
                    RtpMediaKind::H264,
                    video_stream(&out)
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_clock_rate(0),
            ],
        })
        .expect_err("a zero clock rate cannot time a track");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "must be a structured input error, got {err:?}"
    );
}

// ── Test 4c: a real ffmpeg RTSP SDP becomes a byte-exact avcC ───────────────

/// The High profile `avcC` built from ffmpeg's own SDP for
/// `fixtures/ts/h264/high.ts` must be byte-identical to the `avcC` ffmpeg
/// itself writes when remuxing that stream (audit r04-W33).
///
/// Both sides come from ffmpeg 8.1 over the same input: the SDP from
/// `-c copy -f rtp -sdp_file` (what an RTSP camera's DESCRIBE returns) and the
/// avcC from `-c copy out.mp4`. Pre-fix the record omitted ISO/IEC
/// 14496-15 §5.3.3.1.2's High profile chroma/bit-depth trailer, so it was
/// four bytes short of ffmpeg's own and a strict reader would mis-parse.
#[test]
fn high_profile_sdp_becomes_ffmpeg_byte_exact_avcc() {
    const SDP: &str = include_str!("fixtures/rtp/high-ffmpeg.sdp");
    const AVCC_HEX: &str = include_str!("fixtures/rtp/high-ffmpeg.avcc.txt");
    let oracle = hex_decode(AVCC_HEX.trim()).expect("ffmpeg avcC hex");
    // Strip the 8-byte box header (`size` + `avcC`), leaving the record.
    let record = &oracle[8..];

    let fmtp = SDP
        .lines()
        .find(|l| l.starts_with("a=fmtp:"))
        .expect("SDP has an fmtp line");
    let config = transmux::rtp_sdp::avc_config_from_fmtp(fmtp).expect("real ffmpeg High SDP");

    let mut ours = vec![0u8; config.config.serialized_len()];
    config
        .config
        .serialize_into(&mut ours)
        .expect("serialize avcC");
    assert_eq!(
        ours, record,
        "the avcC from the SDP must match ffmpeg's own for the same stream"
    );
    assert_eq!(
        config.config.chroma_format,
        Some(1),
        "the trailer's chroma_format is the SPS's own 4:2:0"
    );
    assert_eq!(config.config.bit_depth_luma_minus8, Some(0));
    assert_eq!(config.config.bit_depth_chroma_minus8, Some(0));
}

/// The same oracle against a Baseline stream: no trailer on either side
/// (ISO/IEC 14496-15 §5.3.3.1.2 conditions it on the profile).
#[test]
fn baseline_profile_sdp_writes_no_trailer() {
    const SDP: &str = include_str!("fixtures/rtp/baseline-ffmpeg.sdp");
    let fmtp = SDP
        .lines()
        .find(|l| l.starts_with("a=fmtp:"))
        .expect("SDP has an fmtp line");
    let config = transmux::rtp_sdp::avc_config_from_fmtp(fmtp).expect("real ffmpeg Baseline SDP");
    assert_eq!(config.config.profile_indication, 0x42, "Baseline");
    assert_eq!(
        config.config.chroma_format, None,
        "§5.3.3.1.2's trailer is absent for a non-High profile"
    );
    assert_eq!(config.config.bit_depth_luma_minus8, None);
    assert_eq!(config.config.bit_depth_chroma_minus8, None);
}

// ── Test 4d: the RTP timestamp is the presentation time (RFC 6184 §5.1) ─────

/// A B-frame stream must be packetised with each access unit's **PTS**, not
/// its DTS (audit r04-W29).
///
/// RFC 6184 §5.1: "The RTP timestamp is set to the sampling timestamp of the
/// content", and receivers "SHOULD use the RTP timestamp for synchronizing the
/// display process" — a presentation time. `fixtures/ts/h264/high.ts` is a real
/// B-frame encode (ffprobe: `has_b_frames=2`, 12 of its 15 samples carry a
/// composition offset), so stamping the decode time gives a receiver a
/// timestamp series that does not match presentation order at all. The oracle
/// here is the demuxed track's own `pts` series, rescaled into the 90 kHz RTP
/// clock — never a hardcoded value.
#[test]
fn packetised_timestamps_are_presentation_times() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/h264/high.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux high.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track");
    let timescale = track.spec.timescale;
    let pts_series: Vec<u64> = track
        .samples
        .iter()
        .map(|s| u64::try_from(s.pts.expect("timed sample")).expect("pts >= 0"))
        .collect();
    let dts_series: Vec<u64> = track
        .samples
        .iter()
        .map(|s| u64::try_from(s.dts.expect("timed sample")).expect("dts >= 0"))
        .collect();
    assert_ne!(
        pts_series, dts_series,
        "the fixture must actually be reordered for this test to mean anything"
    );

    let out = packetise(&media);
    let vs = video_stream(&out);
    // The STAP-A parameter-set packet leads with the same (first) timestamp;
    // one packet per AU here (the fixture is small enough not to fragment),
    // so the distinct timestamps in wire order are the AU series.
    let mut au_ts: Vec<u32> = Vec::new();
    for pkt in &vs.packets {
        let (_v, _pt, _m, _seq, ts, _ssrc) = parse_hdr(pkt);
        if au_ts.last() != Some(&ts) {
            au_ts.push(ts);
        }
    }

    let first_pts = pts_series[0];
    let rescale = |ticks: u64| -> u32 {
        u32::try_from((ticks - first_pts) * u64::from(VIDEO_CLOCK_RATE) / u64::from(timescale))
            .expect("fits 32 bits")
    };
    let expected_pts: Vec<u32> = pts_series.iter().map(|&p| rescale(p)).collect();

    assert_eq!(
        au_ts[..expected_pts.len()],
        expected_pts[..],
        "the RTP timestamp series must be the PTS series"
    );

    // The series it must NOT be: the same samples in decode order, which is
    // what stamping `dts` produced.
    let expected_dts: Vec<u32> = dts_series
        .iter()
        .map(|&d| rescale(d + (pts_series[0] - dts_series[0])))
        .collect();
    assert_ne!(
        au_ts[..expected_pts.len()],
        expected_dts[..],
        "and it must not be the decode-time series on a reordered stream"
    );
}

/// The batch depacketiser must *report* a stream whose presentation
/// timestamps step backward, not absorb it silently (audit r04-W29): RTP
/// carries only the sampling timestamp (RFC 6184 §5.1), so `dts == pts` is an
/// assumption — and the stream it is wrong for is a real one, the reordered
/// `fixtures/ts/h264/high.ts`. `Unpackage` can only return `Media`, so the
/// signal surfaces through `poll_timing_warning`.
#[test]
fn batch_depacketiser_reports_reordered_timestamps() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/h264/high.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux high.ts");
    let out = packetise(&media);
    let mut depack = RtpDepacketiser::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(
                RtpMediaKind::H264,
                video_stream(&out)
                    .packets
                    .iter()
                    .map(|p| p.as_contiguous().to_vec())
                    .collect(),
            )],
        })
        .expect("reassembly itself succeeds — the AUs are intact");

    let mut warnings = Vec::new();
    while let Some(w) = depack.poll_timing_warning() {
        warnings.push(w);
    }
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            transmux::RtpTimingWarning::ReorderedPresentationTimestamps {
                stream_index: 0,
                ..
            }
        )),
        "the real B-frame fixture must be reported as reordered, got {warnings:?}"
    );

    // And the samples are still the wire's own presentation timestamps.
    let pts: Vec<i64> = ir.tracks[0]
        .samples
        .iter()
        .map(|s| s.pts.unwrap())
        .collect();
    assert!(
        pts.windows(2).any(|w| w[1] < w[0]),
        "the fixture's presentation timestamps really are out of order"
    );
}

/// The same reordered stream through the streaming depacketiser is *reported*,
/// not silently flattened: it raises `NonMonotonicTimestamp` once per step
/// backward, and the samples it does emit keep their presentation timestamps.
#[test]
fn streaming_depacketiser_reports_reordered_timestamps() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/h264/high.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux high.ts");
    let out = packetise(&media);
    let config = media.tracks[0].spec.config.clone();

    // The batch path emits these in PTS order (so its own input is what a
    // PTS-ordered receiver would see); feed the raw wire packets instead and
    // count how many presentation steps go backward.
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let mut d = transmux::RtpStreamDepacketiser::new(vec![transmux::RtpStreamTrack::new(
        1,
        RtpMediaKind::H264,
        config,
        VIDEO_CLOCK_RATE,
    )]);
    let mut samples = Vec::new();
    for pkt in &packets {
        samples.extend(d.push(1, pkt).unwrap());
    }
    samples.extend(d.flush(1).unwrap());

    let mut reorders = 0usize;
    while let Some(e) = d.poll_loss_event() {
        if matches!(e, transmux::RtpLossEvent::NonMonotonicTimestamp { .. }) {
            reorders += 1;
        }
    }
    assert!(
        reorders > 0,
        "the real B-frame fixture must be reported as reordered"
    );

    // The samples carry the wire timestamps (presentation), not a fabricated
    // decode timeline: the pts series is exactly the wire series.
    let pts: Vec<i64> = samples.iter().map(|s| s.pts.expect("timed")).collect();
    let expected: Vec<i64> = {
        let mut seen: Vec<i64> = Vec::new();
        for pkt in &packets {
            let ts = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
            let v = i64::from(ts);
            if seen.last() != Some(&v) {
                seen.push(v);
            }
        }
        seen
    };
    assert_eq!(
        pts, expected,
        "emitted pts must be the wire sampling timestamps, in wire order"
    );
    // The streaming path keeps the documented low-delay model (`dts == pts`)
    // and reports the reorder instead of guessing a decode timeline — it
    // cannot know the frame period from a live stream the way the batch path
    // can (which derives it from the whole AU set). A consumer that needs a
    // decode-ordered timeline must therefore check for
    // `NonMonotonicTimestamp`; that is precisely why it is reported.
    for s in &samples {
        assert_eq!(
            s.pts, s.dts,
            "the streaming path's documented model: dts == pts, with reorder              reported rather than guessed"
        );
    }
}

// ── Test 4e: loss and RFC 3640 AU fragmentation (audit r04-W31) ─────────────

/// A run of FU-A continuation fragments with no preceding start fragment — a
/// mid-stream capture, or one lost start packet — must skip that NAL, not fail
/// the whole input.
///
/// Built from the real fixture's own packets: the first packets of a real
/// fragmented access unit are dropped, so what remains is a genuine
/// continuation run at a real timestamp, and the packets after it are real
/// ones whose recovery is asserted.
#[test]
fn fua_continuation_without_start_does_not_fail_the_stream() {
    let media = demux_fixture();
    let out = packetise(&media);
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();

    // Start the capture part-way through a real fragmented access unit: keep
    // every packet from the *second* fragment of the first FU-A run onward, so
    // the truncated input genuinely begins with a continuation fragment.
    const NAL_TYPE_FU_A: u8 = 28;
    const FU_START_MASK: u8 = 0x80;
    let start = packets
        .iter()
        .position(|p| {
            (p[RTP_HEADER_LEN] & 0x1F) == NAL_TYPE_FU_A
                && (p[RTP_HEADER_LEN + 1] & FU_START_MASK) != 0
        })
        .expect("the fixture must contain a fragmented NAL");
    let truncated = &packets[start + 1..];
    assert_eq!(
        truncated[0][RTP_HEADER_LEN] & 0x1F,
        NAL_TYPE_FU_A,
        "the truncated input must begin mid-FU-A"
    );
    assert_eq!(
        truncated[0][RTP_HEADER_LEN + 1] & FU_START_MASK,
        0,
        "and it must be a continuation fragment, not a new start"
    );

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, truncated.to_vec())],
        })
        .expect("a mid-stream capture must depacketise, not fail");

    // The stream after the damaged NAL is recovered: the tail of the original
    // access-unit series is present, byte for byte.
    let originals = original_video_nals(&media);
    let rebuilt: Vec<Vec<Vec<u8>>> = ir.tracks[0]
        .samples
        .iter()
        .map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect::<Vec<Vec<u8>>>()
        })
        .collect();
    assert!(
        rebuilt.len() >= originals.len() - 2,
        "recovered {} AUs of {} — the packets after the damaged NAL must survive",
        rebuilt.len(),
        originals.len()
    );
    assert_eq!(
        rebuilt.last(),
        originals.last(),
        "the final access unit must be recovered intact"
    );
}

/// An AAC access unit larger than the payload budget must be fragmented on
/// packetise (RFC 3640 §3.2.3.1) and reassembled on depacketise, byte for
/// byte.
///
/// Uses the real 5.1 640 kb/s fixture (`fixtures/ts/aac-5_1-640k.ts`), whose
/// frames are ~785 bytes, and a 400-byte MTU so every access unit must split
/// over several packets. The oracle is the demuxed track's own access-unit
/// bytes.
#[test]
fn fragmented_aac_access_units_round_trip() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/aac-5_1-640k.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux aac-5_1-640k.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track");
    let max_au = track
        .samples
        .iter()
        .map(|s| s.data.len())
        .max()
        .expect("samples");
    const SMALL_MTU: usize = 400;
    assert!(
        max_au > SMALL_MTU,
        "the fixture's largest AU ({max_au} B) must exceed the MTU ({SMALL_MTU} B)"
    );
    let config = track.spec.config.clone();
    let clock = match &config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        _ => unreachable!(),
    };
    let original: Vec<bytes::Bytes> = track.samples.iter().map(|s| s.data.clone()).collect();

    let mut p = RtpPacketiser {
        mtu: SMALL_MTU,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise with fragmentation");
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .expect("audio stream");
    assert!(
        stream.packets.len() > original.len(),
        "fragmentation must produce more packets ({}) than AUs ({})",
        stream.packets.len(),
        original.len()
    );
    // Every fragment carries one RTP timestamp per AU, and only the last
    // fragment of an AU has the marker (RFC 3640 §3.1).
    let mut frag_timestamps = 0usize;
    let mut prev_ts: Option<u32> = None;
    for pkt in &stream.packets {
        let ts = u32::from_be_bytes([pkt.header[4], pkt.header[5], pkt.header[6], pkt.header[7]]);
        if prev_ts != Some(ts) {
            frag_timestamps += 1;
        }
        prev_ts = Some(ts);
    }
    assert_eq!(
        frag_timestamps,
        original.len(),
        "one RTP timestamp group per access unit"
    );

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(
                    RtpMediaKind::Aac,
                    stream
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_clock_rate(clock)
                .with_config(config),
            ],
        })
        .expect("depacketise fragmented AAC");

    assert_eq!(
        ir.tracks[0].samples.len(),
        original.len(),
        "every access unit must be reassembled"
    );
    for (i, (got, want)) in ir.tracks[0].samples.iter().zip(original.iter()).enumerate() {
        assert_eq!(got.data, *want, "AU {i} must be byte-identical");
    }
}

/// A fragment run that never completes (its remaining packets lost) must not
/// emit a truncated access unit — the receiver cannot know what the missing
/// tail was, and handing a partial AAC frame to a decoder is worse than
/// dropping it.
#[test]
fn incomplete_aac_fragments_are_not_emitted_truncated() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/aac-5_1-640k.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux aac-5_1-640k.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track");
    let config = track.spec.config.clone();
    let clock = match &config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        _ => unreachable!(),
    };
    let first_au = track.samples[0].data.clone();

    let mut p = RtpPacketiser {
        mtu: 200,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise with fragmentation");
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .expect("audio stream");

    // Keep only the first fragment of the first access unit (its second
    // packet is "lost").
    let first_ts = u32::from_be_bytes([
        stream.packets[0].header[4],
        stream.packets[0].header[5],
        stream.packets[0].header[6],
        stream.packets[0].header[7],
    ]);
    let kept: Vec<Vec<u8>> = stream
        .packets
        .iter()
        .filter(|p| {
            u32::from_be_bytes([p.header[4], p.header[5], p.header[6], p.header[7]]) == first_ts
        })
        .take(1)
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    assert_eq!(kept.len(), 1, "one surviving fragment");

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::Aac, kept)
                    .with_clock_rate(clock)
                    .with_config(config),
            ],
        })
        .expect("an incomplete run is not an error");
    assert!(
        ir.tracks[0].samples.is_empty(),
        "a truncated AU must be dropped, not emitted (got {})",
        ir.tracks[0].samples.len()
    );
    assert_ne!(
        ir.tracks[0].samples.first().map(|s| s.data.clone()),
        Some(first_au),
        "and it must not be presented as the complete AU"
    );
}

// ── Test 4f: SDP payload types and the c= line (audit r04-W34) ──────────────

/// A session with more than one video (or audio) track must give every stream
/// its own dynamic payload type, and none may collide with the fixed type this
/// crate binds to KLV.
///
/// A payload type is a session-wide binding (RFC 3551 §6 reserves 96-127 "for
/// dynamic assignment"), so two streams sharing one describes both as the same
/// encoding. Before this, only "has a video track been seen" was tracked, so a
/// third video track reused `video_pt + 2` (colliding with the second) and
/// `96 + 2 = 98` collided with `DEFAULT_KLV_PT`.
#[test]
fn every_stream_gets_a_unique_dynamic_payload_type() {
    let media = demux_fixture();
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
        .clone();
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .clone();

    // Three video + two audio tracks: the shape that used to collide.
    let mut ir = Media::new(
        vec![video.clone(), video.clone(), video, audio.clone(), audio],
        media.movie_timescale,
    );
    ir.pcr = media.pcr.clone();
    let out = packetise(&ir);

    let pts: Vec<u8> = out.streams.iter().map(|s| s.pt).collect();
    let mut unique = pts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        pts.len(),
        "every stream needs its own payload type, got {pts:?}"
    );
    assert!(
        !pts.contains(&transmux::DEFAULT_KLV_PT),
        "no RTP stream may take {} — it is bound to KLV (RFC 6597), got {pts:?}",
        transmux::DEFAULT_KLV_PT
    );
    for pt in &pts {
        assert!(
            (96..=127).contains(pt),
            "payload type {pt} is outside RFC 3551 §6's dynamic range"
        );
    }

    // The first video/audio tracks keep the documented defaults, so the common
    // single-video/single-audio session's SDP is unchanged.
    assert_eq!(pts[0], transmux::DEFAULT_VIDEO_PT);
    assert_eq!(pts[3], transmux::DEFAULT_AUDIO_PT);

    // And the generated SDP binds each of those types exactly once.
    for pt in &pts {
        let needle = format!("a=rtpmap:{pt} ");
        assert_eq!(
            out.sdp.matches(&needle).count(),
            1,
            "SDP must bind {pt} exactly once:
{}",
            out.sdp
        );
    }
}

/// The generated SDP must be accepted by an independent parser, including
/// RFC 4566 §5.7's `c=` requirement: "A session description MUST contain
/// either at least one `c=` field in each media description or a single `c=`
/// field at the session level."
///
/// The oracle is `sdp-types`, a third-party crate already in the workspace
/// (rtsp-runtime depends on it) — so this catches a misreading of RFC 4566 that
/// the renderer and its own string assertions would share.
#[test]
fn generated_sdp_parses_with_an_independent_parser() {
    let media = demux_fixture();
    let out = packetise(&media);
    assert!(
        out.sdp.contains("c=IN IP4 "),
        "the SDP must carry a c= line:
{}",
        out.sdp
    );

    let parsed: sdp_types::Session =
        sdp_types::Session::parse(out.sdp.as_bytes()).expect("sdp-types must accept the SDP");
    assert_eq!(
        parsed.medias.len(),
        out.streams.len(),
        "every stream must appear as one m= section"
    );
    // A parser that follows §5.7 can resolve a connection for every media
    // section: either its own c= or the session-level one.
    let session_connection = parsed.connection.as_ref();
    for m in &parsed.medias {
        assert!(
            !m.connections.is_empty() || session_connection.is_some(),
            "media {:?} has no reachable c= (RFC 4566 §5.7)",
            m.media
        );
    }
    // Each stream's payload type is bound in exactly its own m= section, and
    // that section is the stream's own media kind. The `rtpmap` value is
    // `<pt> <encoding>/<clock>` (RFC 4566 §5.14).
    for stream in &out.streams {
        let bound: Vec<&str> = parsed
            .medias
            .iter()
            .filter(|m| {
                m.attributes.iter().any(|a| {
                    a.attribute == "rtpmap"
                        && a.value.as_deref().is_some_and(|v| {
                            v.split_whitespace()
                                .next()
                                .and_then(|pt| pt.parse::<u8>().ok())
                                == Some(stream.pt)
                        })
                })
            })
            .map(|m| m.media.as_str())
            .collect();
        assert_eq!(
            bound.len(),
            1,
            "payload type {} must be bound in exactly one m= section, found {bound:?}",
            stream.pt
        );
        assert_eq!(
            bound[0],
            stream.kind.name(),
            "payload type {} must be bound in the {} section",
            stream.pt,
            stream.kind.name()
        );
    }
}

// ── Test 4g: the depacketised decode timeline (audit r04-W29) ───────────────

/// A **variable-frame-rate** stream must come back with `dts == pts`: without
/// reordering there is nothing to reconstruct, and laying a uniform grid over
/// irregular steps would misplace every sample after the first one.
///
/// Drives hand-built packets through the real batch depacketiser: two frames
/// 3000 ticks apart followed by one 6000 ticks later, i.e. the shape a dropped
/// frame or a VFR encoder produces.
#[test]
fn vfr_stream_keeps_dts_equal_to_pts() {
    // Presentation instants 0, 3000, 9000 (a 3000-tick frame then a 6000-tick
    // one — a genuine VFR step).
    let packets: Vec<Vec<u8>> = [0u32, 3000, 9000]
        .iter()
        .enumerate()
        .map(|(i, ts)| nalu_pkt(i as u16, *ts, &[0x41, 0xA0 + i as u8]))
        .collect();
    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, packets)],
        })
        .expect("depacketise VFR stream");
    let samples = &ir.tracks[0].samples;
    assert_eq!(samples.len(), 3);
    let dts: Vec<i64> = samples.iter().map(|s| s.dts.unwrap()).collect();
    let pts: Vec<i64> = samples.iter().map(|s| s.pts.unwrap()).collect();
    assert_eq!(
        dts,
        vec![0, 3000, 9000],
        "dts must be the VFR series itself"
    );
    assert_eq!(pts, vec![0, 3000, 9000], "and so must pts");
    let durations: Vec<u32> = samples.iter().map(|s| s.duration.unwrap()).collect();
    assert_eq!(
        durations,
        vec![3000, 6000, 6000],
        "durations must be the real steps (the last reuses the previous)"
    );
}

/// A **B-frame reordered** stream must come back with an exact, standard decode
/// order: `dts` non-decreasing, `dts <= pts` at every sample, and the two
/// series differing by a positive composition offset.
///
/// Uses the real reordered pattern of `fixtures/ts/h264/high.ts` (pts
/// 0, 14400, 7200, 3600, 10800 … at 3600 ticks/frame), packetised by this
/// crate's own packetiser and depacketised by the batch path, so the wire bytes
/// are real.
#[test]
fn reordered_stream_gets_an_exact_decode_order() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/h264/high.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux high.ts");
    let out = packetise(&media);
    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(
                RtpMediaKind::H264,
                video_stream(&out)
                    .packets
                    .iter()
                    .map(|p| p.as_contiguous().to_vec())
                    .collect(),
            )],
        })
        .expect("depacketise the reordered fixture");

    let samples = &ir.tracks[0].samples;
    let dts: Vec<i64> = samples.iter().map(|s| s.dts.unwrap()).collect();
    let pts: Vec<i64> = samples.iter().map(|s| s.pts.unwrap()).collect();

    // The period is the fixture's own frame interval (3600 ticks at 90 kHz),
    // and the decode timeline is that period laid out in wire order.
    assert_eq!(
        dts,
        (0..samples.len() as i64)
            .map(|i| i * 3600)
            .collect::<Vec<i64>>(),
        "the decode timeline must be the frame period in wire (decode) order"
    );
    assert!(
        dts.windows(2).all(|w| w[1] >= w[0]),
        "dts must be non-decreasing"
    );
    assert!(
        dts.iter().zip(&pts).all(|(d, p)| d <= p),
        "no frame may be presented before it is decoded"
    );
    // The composition offsets are the presentation order minus the decode
    // order, i.e. exactly the latch of the reorder pattern the fixture has.
    let offsets: Vec<i64> = pts.iter().zip(&dts).map(|(p, d)| p - d).collect();
    assert!(
        offsets.iter().all(|o| *o >= 0),
        "every composition offset must be non-negative: {offsets:?}"
    );
    assert!(
        offsets.iter().any(|o| *o > 0),
        "a reordered stream must carry a positive offset somewhere"
    );
    // Durations are the grid step, never zero for a non-final frame.
    let durations: Vec<u32> = samples.iter().map(|s| s.duration.unwrap()).collect();
    assert!(
        durations.iter().all(|d| *d > 0),
        "no non-final frame may have a zero duration: {durations:?}"
    );
    assert_eq!(durations[0], 3600, "the frame period is the fixture's 3600");
}

/// One single-NAL, marker-set video packet (the shape `packetise_video` emits
/// for a small NAL), so a test can state an exact wire timestamp series.
fn nalu_pkt(seq: u16, ts: u32, nal: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, 0x80 | 96];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&SSRC.to_be_bytes());
    p.extend_from_slice(nal);
    p
}

// ── Test 4h: the stream clock follows the codec config (audit r04-W30) ──────

/// An AAC stream's RTP clock rate **is** its sampling rate (RFC 3640 §3.1), so
/// it must come from the `AudioSpecificConfig` rather than from the 48000 Hz
/// placeholder [`RtpInputStream::new`] starts with: a 44.1 kHz stream declared
/// at 48 kHz is timed at the wrong rate by every duration.
#[test]
fn aac_clock_rate_follows_the_config_sample_rate() {
    // A real 44.1 kHz AAC-LC ASC (samplingFrequencyIndex 4 = 44100,
    // channelConfiguration 2 = stereo): 0x12 0x10.
    let config = transmux::rtp_sdp::aac_config_from_asc_hex("1210").expect("real 44.1 kHz ASC");
    let sample_rate = match &config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        other => panic!("expected AAC, got {other:?}"),
    };
    assert_eq!(sample_rate, 44_100, "sfi 4 = 44100 Hz");

    let stream =
        RtpInputStream::new(RtpMediaKind::Aac, vec![aac_pkt(0, 0, &[0xAA; 4])]).with_config(config);
    assert_eq!(
        stream.clock_rate, 44_100,
        "the stream's clock must be taken from the config's own sample rate"
    );

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![stream],
        })
        .expect("depacketise a 44.1 kHz AAC stream");
    assert_eq!(
        ir.tracks[0].spec.timescale, 44_100,
        "and the IR track's timescale must be that rate, not 48000"
    );
}

/// A declared clock rate that disagrees with the AAC config's sample rate is
/// rejected: the two cannot both be the stream's clock, and picking either
/// silently times the media wrongly.
#[test]
fn aac_clock_rate_disagreeing_with_the_config_is_rejected() {
    let config = transmux::rtp_sdp::aac_config_from_asc_hex("1210").expect("44.1 kHz ASC");
    let err = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::Aac, vec![aac_pkt(0, 0, &[0xAA; 4])])
                    .with_clock_rate(48_000)
                    .with_config(config),
            ],
        })
        .expect_err("48 kHz is not the config's sample rate");
    assert!(
        matches!(err, transmux::Error::InvalidValue { .. }),
        "must be a structured value error, got {err:?}"
    );
}

/// A stream whose `kind` disagrees with its config's variant is rejected: the
/// payload format is what the packets are parsed as, so an AAC config on an
/// H264 stream (the pre-fix placeholder shape) cannot be honoured.
#[test]
fn kind_disagreeing_with_the_config_is_rejected() {
    let media = demux_fixture();
    let aac = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .spec
        .config
        .clone();
    let err = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::H264, vec![nalu_pkt(0, 0, &[0x65, 0xAA])])
                    .with_config(aac),
            ],
        })
        .expect_err("an AAC config on an H264 stream is a contradiction");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "must be a structured input error, got {err:?}"
    );
}

/// A non-90 kHz clock on an AVC stream is rejected: RFC 6184 §8.1 fixes it.
#[test]
fn non_90khz_avc_clock_is_rejected() {
    let media = demux_fixture();
    let avc = media.tracks[0].spec.config.clone();
    let err = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::H264, vec![nalu_pkt(0, 0, &[0x65, 0xAA])])
                    .with_clock_rate(48_000)
                    .with_config(avc),
            ],
        })
        .expect_err("H.264 is carried at 90 kHz (RFC 6184 §8.1)");
    assert!(
        matches!(err, transmux::Error::InvalidValue { .. }),
        "must be a structured value error, got {err:?}"
    );
}

/// A single-packet AAC-hbr sample carrying `au` as one access unit.
fn aac_pkt(seq: u16, ts: u32, au: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, 0x80 | 97];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&SSRC.to_be_bytes());
    // AU-headers-length (16 bits) then the AU-header (size << 3).
    p.extend_from_slice(&16u16.to_be_bytes());
    p.extend_from_slice(&((au.len() as u16) << 3).to_be_bytes());
    p.extend_from_slice(au);
    p
}

// ── Test 4i: the c= address is typed, so it cannot inject SDP (r04-W34) ─────

/// The `c=` line's `<addrtype>` follows the address's own family, and a typed
/// address cannot carry SDP syntax into the description.
///
/// RFC 8866 §5.7: "This memo only defines `IP4` and `IP6`". A `&str` parameter
/// made this a way to inject arbitrary fields — an address of
/// `"1.2.3.4\r\nm=video 0 RTP/AVP 96"` produced a whole extra media section —
/// which a `core::net::IpAddr` makes impossible.
#[test]
fn connection_address_is_typed_and_emits_the_right_addrtype() {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    let media = demux_fixture();
    let out = packetise(&media);

    // IPv4: `c=IN IP4 <v4>`, alongside real media blocks.
    let media_blocks: String = out
        .sdp
        .lines()
        .filter(|l| l.starts_with("m=") || l.starts_with("a="))
        .map(|l| {
            format!(
                "{l}
"
            )
        })
        .collect();
    let v4 = transmux::build_sdp_with_connection(
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
        &media_blocks,
    );
    assert!(
        v4.contains("c=IN IP4 203.0.113.7\r\n"),
        "IPv4 must emit addrtype IP4:\n{v4}"
    );

    // IPv6: `c=IN IP6 <v6>`, with no brackets and no TTL (RFC 8866 §5.7).
    let v6 = transmux::build_sdp_with_connection(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        "",
    );
    assert!(
        v6.contains("c=IN IP6 2001:db8::1\r\n"),
        "IPv6 must emit addrtype IP6 and the canonical form:\n{v6}"
    );
    assert!(
        !v6.contains('/'),
        "an IPv6 connection address must carry no TTL suffix (RFC 8866 §5.7)"
    );

    // Exactly one `c=` line at session level, and no media-level ones.
    assert_eq!(
        v4.matches("c=").count(),
        1,
        "one session-level c= line:\n{v4}"
    );
    // The address is parsed from a string that *would* have been an injection
    // under the old `&str` API: this is the closest a caller can get to it now,
    // and it yields a plain address (or a parse error), never extra fields.
    if let Ok(injected) = "1.2.3.4\r\nm=video 0 RTP/AVP 96".parse::<IpAddr>() {
        let sdp = transmux::build_sdp_with_connection(injected, "");
        assert_eq!(
            sdp.matches("m=").count(),
            0,
            "no media section can be injected through the connection address"
        );
    }
    assert!(
        transmux::LOCAL_CONNECTION_ADDRESS.is_ipv4(),
        "the default connection address is the loopback the o= line names"
    );
    // A description with the typed address still parses, and every media
    // section resolves a connection from the session level (RFC 8866 §5.7).
    let parsed = sdp_types::Session::parse(v4.as_bytes()).expect("sdp-types accepts the SDP");
    assert!(parsed.connection.is_some(), "session c= must be present");
    for m in &parsed.medias {
        assert!(
            !m.connections.is_empty() || parsed.connection.is_some(),
            "media {:?} must resolve a connection",
            m.media
        );
    }
}

// ── Test 4j: sequence-number loss detection (audit r04-W31) ─────────────────

/// A **lost middle FU-A fragment** followed by the end fragment must not emit a
/// truncated NAL: the reassembled access unit would be missing the bytes
/// between them, and the length prefix it is written with would describe a NAL
/// that does not exist.
///
/// Built from the real fixture's own fragmented access unit: one middle packet
/// is removed, so the surviving fragments are contiguous *except* at the hole —
/// exactly a packet lost in transit.
#[test]
fn lost_middle_fu_a_fragment_never_emits_a_truncated_nal() {
    let media = demux_fixture();
    let out = packetise(&media);
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();

    // Locate a real FU-A run and drop a fragment from its middle.
    const NAL_TYPE_FU_A: u8 = 28;
    let (start, len) = {
        let mut best = (0usize, 0usize);
        let mut i = 0usize;
        while i < packets.len() {
            if packets[i][RTP_HEADER_LEN] & 0x1F != NAL_TYPE_FU_A {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < packets.len() && packets[j][RTP_HEADER_LEN] & 0x1F == NAL_TYPE_FU_A {
                j += 1;
            }
            if j - i > best.1 {
                best = (i, j - i);
            }
            i = j;
        }
        best
    };
    assert!(
        len >= 3,
        "the fixture must have an FU-A run of >=3 fragments"
    );
    let dropped = start + len / 2;
    let mut holed = packets.clone();
    holed.remove(dropped);

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, holed)],
        })
        .expect("the stream still depacketises");

    // The damaged NAL must be absent, not present-but-short: compare every
    // recovered NAL against the original fixture's NALs.
    let originals = original_video_nals(&media);
    let original_flat: Vec<Vec<u8>> = originals.iter().flatten().cloned().collect();
    let recovered: Vec<Vec<u8>> = ir.tracks[0]
        .samples
        .iter()
        .flat_map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect::<Vec<Vec<u8>>>()
        })
        .collect();

    // Every recovered NAL that is not a parameter set must be one of the
    // original NALs, byte for byte — a truncated reassembly cannot be.
    for nal in &recovered {
        if nal.is_empty() {
            continue;
        }
        let nal_type = nal[0] & 0x1F;
        if nal_type == 7 || nal_type == 8 {
            continue; // SPS/PPS from the leading STAP-A
        }
        assert!(
            original_flat.contains(nal),
            "recovered NAL type {nal_type} of {} bytes is a truncated \
             reassembly: it matches no NAL in the original stream",
            nal.len()
        );
    }
    // And the damaged NAL really is gone: one fewer VCL NAL comes back than
    // the original stream has.
    let original_vcl: Vec<&Vec<u8>> = original_flat
        .iter()
        .filter(|n| !n.is_empty() && (n[0] & 0x1F) != 7 && (n[0] & 0x1F) != 8)
        .collect();
    let recovered_vcl: Vec<&Vec<u8>> = recovered
        .iter()
        .filter(|n| !n.is_empty() && (n[0] & 0x1F) != 7 && (n[0] & 0x1F) != 8)
        .collect();
    assert!(
        recovered_vcl.len() < original_vcl.len(),
        "the damaged NAL must be dropped, not emitted short ({} vs {})",
        recovered_vcl.len(),
        original_vcl.len()
    );
}

/// A **duplicated** audio fragment must not push the accumulated bytes past the
/// AU-header's declared `AU-size` and still be emitted: the header states the
/// AU's full size (RFC 3640 §3.2.3.2), so a run that overruns it is not the AU
/// it claims to be.
#[test]
fn duplicated_audio_fragment_is_not_emitted_as_an_oversized_au() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/aac-5_1-640k.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux aac-5_1-640k.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track");
    let config = track.spec.config.clone();
    let clock = match &config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        _ => unreachable!(),
    };
    let original: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.to_vec()).collect();

    // Fragmented, since only then can a duplicate land inside a run.
    let mut p = RtpPacketiser {
        mtu: 200,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise with fragmentation");
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .expect("audio stream");
    let mut packets: Vec<Vec<u8>> = stream
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();

    // Duplicate the second *fragment* of the run (not the first packet of an
    // AU), i.e. the packet after the first AU's start.
    let dup_idx = 2.min(packets.len() - 1);
    packets.insert(dup_idx, packets[dup_idx].clone());

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::Aac, packets)
                    .with_clock_rate(clock)
                    .with_config(config),
            ],
        })
        .expect("depacketise with a duplicate fragment");

    // No emitted AU may be longer than the original it corresponds to: an
    // over-long one is the duplicated bytes having been appended.
    let longest_original = original.iter().map(|a| a.len()).max().unwrap();
    for s in &ir.tracks[0].samples {
        assert!(
            s.data.len() <= longest_original,
            "an emitted AU of {} bytes exceeds the fixture's largest AU \
             ({longest_original}): a duplicate fragment was concatenated",
            s.data.len()
        );
        assert!(
            original.contains(&s.data.to_vec()),
            "every emitted AU must be one of the originals, byte for byte"
        );
    }
}

/// A **missing** audio fragment must drop the whole run rather than emit the
/// bytes that did arrive. The AU-header declares the full `AU-size`, so a short
/// run is known to be incomplete.
#[test]
fn missing_audio_fragment_drops_the_whole_au() {
    const REF: &[u8] = include_bytes!("../../fixtures/ts/aac-5_1-640k.ts");
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(REF).expect("demux aac-5_1-640k.ts");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track");
    let config = track.spec.config.clone();
    let clock = match &config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        _ => unreachable!(),
    };
    let original: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.to_vec()).collect();

    let mut p = RtpPacketiser {
        mtu: 200,
        ssrc: SSRC,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise with fragmentation");
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::Aac)
        .expect("audio stream");
    let packets: Vec<Vec<u8>> = stream
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();

    // Drop the packet carrying the *middle* of the first access unit's run.
    let first_ts = u32::from_be_bytes([packets[0][4], packets[0][5], packets[0][6], packets[0][7]]);
    let run: Vec<usize> = packets
        .iter()
        .enumerate()
        .filter(|(_, p)| u32::from_be_bytes([p[4], p[5], p[6], p[7]]) == first_ts)
        .map(|(i, _)| i)
        .collect();
    assert!(
        run.len() >= 3,
        "the first AU must fragment into >=3 packets"
    );
    let dropped = run[run.len() / 2];
    let kept: Vec<Vec<u8>> = packets
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != dropped)
        .map(|(_, p)| p.clone())
        .collect();

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(RtpMediaKind::Aac, kept)
                    .with_clock_rate(clock)
                    .with_config(config),
            ],
        })
        .expect("depacketise with a missing fragment");

    // The damaged AU is absent; every AU that *is* present is an original.
    assert!(
        !ir.tracks[0].samples.iter().any(|s| s.data == original[0]),
        "the AU whose fragment was lost must be dropped, not emitted short"
    );
    for s in &ir.tracks[0].samples {
        assert!(
            original.contains(&s.data.to_vec()),
            "an emitted AU of {} bytes is not one of the originals",
            s.data.len()
        );
    }
    assert!(
        ir.tracks[0].samples.len() >= original.len() - 2,
        "the AUs after the damaged one must still be delivered"
    );
}

// ── Test 4k: one epoch across the tracks of a Media (r04-W29) ───────────────

/// Every track in one `Media` must be on the same epoch. A reordered video
/// track's decode timeline is the presentation instants re-laid in wire order,
/// shifted by a reorder latch, while an audio track's is simply its own wire
/// timestamps — so translating only the video track would put the two on
/// different clocks and silently desync a mux that pairs them.
///
/// Uses the real reordered video fixture plus the real AAC fixture, packetised
/// and depacketised together.
#[test]
fn reordered_video_and_audio_share_one_epoch() {
    // Video: a real B-frame encode.
    const VIDEO_TS: &[u8] = include_bytes!("../../fixtures/ts/h264/high.ts");
    let mut vdemux = transmux::TsDemux::new();
    let video_media = vdemux.unpackage(VIDEO_TS).expect("demux high.ts");
    let vout = packetise(&video_media);

    // Audio: a real AAC stream, packetised at a small MTU too.
    const AUDIO_TS: &[u8] = include_bytes!("../../fixtures/ts/aac-5_1-640k.ts");
    let mut ademux = transmux::TsDemux::new();
    let audio_media = ademux.unpackage(AUDIO_TS).expect("demux aac-5_1-640k.ts");
    let aout = {
        let mut p = RtpPacketiser {
            mtu: MTU,
            ssrc: SSRC,
            ..RtpPacketiser::default()
        };
        p.package(&audio_media).expect("packetise the audio")
    };
    let audio_config = audio_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
        .spec
        .config
        .clone();
    let audio_clock = match &audio_config {
        CodecConfig::Aac { sample_rate, .. } => *sample_rate,
        _ => unreachable!(),
    };

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![
                RtpInputStream::new(
                    RtpMediaKind::H264,
                    video_stream(&vout)
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                ),
                RtpInputStream::new(
                    RtpMediaKind::Aac,
                    aout.streams
                        .iter()
                        .find(|s| s.kind == RtpMediaKind::Aac)
                        .expect("audio stream")
                        .packets
                        .iter()
                        .map(|p| p.as_contiguous().to_vec())
                        .collect(),
                )
                .with_clock_rate(audio_clock)
                .with_config(audio_config),
            ],
        })
        .expect("depacketise both streams");
    assert_eq!(ir.tracks.len(), 2, "one track per stream");
    let video_first = ir.tracks[0].samples[0].dts.expect("timed");
    let audio_first = ir.tracks[1].samples[0].dts.expect("timed");

    // Every track is translated by the *same* constant, so the two epochs keep
    // the relationship the wire gave them. The two fixtures are independent
    // sources with unrelated random RTP timestamp origins (RFC 3550 §5.1), so
    // their absolute values differ — what must not differ is the treatment:
    // each track's anchor is its own first sample's dts, and the shift is one
    // constant for the whole `Media` (which is what makes `rebase_to_zero`,
    // which shifts every track together, safe to apply).
    assert_eq!(
        video_first, ir.tracks[0].start_decode_time as i64,
        "the video anchor must be its first sample's dts"
    );
    assert_eq!(
        audio_first, ir.tracks[1].start_decode_time as i64,
        "the audio anchor must be its first sample's dts"
    );
    // The video track is the reordered one: its decode timeline is shifted
    // relative to its own presentation series by the reorder latch. The audio
    // track is not reordered, so its latch is zero — and *both* are on their
    // own wire epoch, neither zero-based independently of the other.
    let video_latch = ir.tracks[0].samples[0]
        .pts
        .expect("timed")
        .saturating_sub(video_first);
    assert!(
        video_latch > 0,
        "the reordered video track must carry a positive latch, got {video_latch}"
    );
    let audio_latch = ir.tracks[1].samples[0]
        .pts
        .expect("timed")
        .saturating_sub(audio_first);
    assert_eq!(
        audio_latch, 0,
        "a non-reordered track's presentation and decode times coincide"
    );
    // A track that starts at the Media's lowest decode instant is at 0; any
    // other keeps its own absolute value rather than being zero-based
    // separately.
    assert_eq!(
        video_first.min(audio_first),
        0,
        "exactly the earliest track sits at the shared origin"
    );
    assert_ne!(
        video_first, audio_first,
        "the two fixtures' RTP origins are unrelated, so only the earlier is at 0"
    );
    // No track's timeline goes negative, which is what forces the shared
    // origin to be a real translation rather than a per-track zero-basing.
    for (i, t) in ir.tracks.iter().enumerate() {
        assert!(
            t.samples.iter().all(|s| s.dts.unwrap_or(0) >= 0),
            "track {i} must have no negative dts"
        );
        assert!(
            t.samples
                .iter()
                .zip(t.samples.iter().skip(1))
                .all(|(a, b)| b.dts.unwrap_or(0) >= a.dts.unwrap_or(0)),
            "track {i} dts must be non-decreasing"
        );
    }
}

// ── Test 4l: the timing-warning queue is bounded (r04-W29) ──────────────────

/// A stream whose timestamps reorder on *every* access unit must not make the
/// depacketiser's warning queue grow without limit: RTP is untrusted remote
/// input, and one warning per access unit over a long session is an
/// unbounded-allocation vector. The queue is capped, the oldest entries are
/// dropped, and the number dropped is reported rather than lost silently.
#[test]
fn timing_warning_queue_is_bounded() {
    // Twice the cap in reordering events, so the queue must drop some:
    // alternate two timestamps so every second AU steps backward.
    let count = 2 * transmux::MAX_TIMING_WARNINGS + 512;
    let packets: Vec<Vec<u8>> = (0..count)
        .map(|i| {
            // 0, 100, 0, 100, ... : every packet after the first steps backward
            // or forward by a full frame, so each backward one raises a warning.
            let ts = if i % 2 == 0 { 0 } else { 100 };
            nalu_pkt(i as u16, ts, &[0x41, 0xAA])
        })
        .collect();
    let mut depack = RtpDepacketiser::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, packets)],
        })
        .expect("depacketise the alternating stream");
    assert_eq!(ir.tracks[0].samples.len(), count);

    let mut seen = 0usize;
    while depack.poll_timing_warning().is_some() {
        seen += 1;
    }
    let dropped = depack.dropped_timing_warnings();
    // The alternating series steps backward once every two packets, and the
    // first packet can never step backward from anything — so the number of
    // reordering events is `count / 2 - 1`. The queue holds the last
    // `MAX_TIMING_WARNINGS` of them and counts the rest as dropped.
    let events = count as u64 / 2 - 1;
    assert!(
        events > transmux::MAX_TIMING_WARNINGS as u64,
        "the test must raise more events ({events}) than the cap          ({})",
        transmux::MAX_TIMING_WARNINGS
    );
    assert_eq!(
        seen,
        transmux::MAX_TIMING_WARNINGS,
        "the queue must hold exactly the cap"
    );
    assert_eq!(
        seen as u64 + dropped,
        events,
        "every reordering event must be either reported or counted as dropped"
    );
    assert!(
        dropped > 0,
        "events beyond the cap must be counted, not silently lost"
    );
}

// ── Test 4m: one lost packet costs exactly one AU (r04-W31) ─────────────────

/// Dropping the **last** packet of access unit N must lose AU N and nothing
/// else: AU N is emitted missing its final NAL was the old behaviour, and the
/// gap was then attributed to AU N+1, whose first packet got discarded with it
/// — one lost packet costing two AUs.
///
/// Built from the real fixture: the marker bit identifies the last packet of
/// each AU, so the test drops exactly that one.
#[test]
fn lost_last_packet_of_an_au_costs_only_that_au() {
    let media = demux_fixture();
    let out = packetise(&media);
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let originals = original_video_nals(&media);
    let original_flat: Vec<Vec<u8>> = originals.iter().flatten().cloned().collect();

    // Find a middle AU whose last packet is an FU-A **fragment**: dropping it
    // leaves the run unfinished, so the AU is genuinely short of data rather
    // than simply missing one whole NAL.
    const NAL_TYPE_FU_A: u8 = 28;
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    for (i, p) in packets.iter().enumerate() {
        if p[1] & 0x80 != 0 {
            groups.push((start, i));
            start = i + 1;
        }
    }
    assert!(groups.len() >= 6, "the fixture must have several AUs");
    // The first AU (after the parameter-set one) whose final packet is an FU-A
    // fragment and which still has a successor AU to check.
    let pick = groups
        .iter()
        .enumerate()
        .find(|(k, (gs, ge))| {
            *k >= 2
                && *ge > *gs
                && *k + 2 < groups.len()
                && packets[*ge][RTP_HEADER_LEN] & 0x1F == NAL_TYPE_FU_A
        })
        .map(|(k, g)| (k, *g))
        .expect("the fixture must have an FU-A-terminated AU with a successor");
    let (group_index, (gstart, gend)) = pick;
    assert!(
        gend > gstart,
        "the chosen AU must have more than one packet"
    );
    let mut damaged = packets.clone();
    damaged.remove(gend);

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, damaged)],
        })
        .expect("the stream still depacketises");

    // Every NAL that comes back must be one of the original stream's, byte for
    // byte — a truncated AU cannot be.
    let recovered: Vec<Vec<u8>> = ir.tracks[0]
        .samples
        .iter()
        .flat_map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect::<Vec<Vec<u8>>>()
        })
        .collect();
    for nal in &recovered {
        if nal.is_empty() || matches!(nal[0] & 0x1F, 7 | 8) {
            continue; // parameter sets from the leading STAP-A
        }
        assert!(
            original_flat.contains(nal),
            "recovered NAL type {} of {} bytes is a truncated reassembly",
            nal[0] & 0x1F,
            nal.len()
        );
    }
    // Exactly one AU's worth of NALs is missing — the damaged AU's, not two.
    let vcl = |v: &[Vec<u8>]| -> usize {
        v.iter()
            .filter(|n| !n.is_empty() && !matches!(n[0] & 0x1F, 7 | 8))
            .count()
    };
    let missing = vcl(&original_flat) - vcl(&recovered);
    // Group index == sample index (one AU per sample, in order), and the AU the
    // drop damaged contributes one VCL NAL fewer than the original had — plus,
    // when the dropped fragment *carried* the AU's only VCL NAL, that NAL too.
    let damaged_vcl = originals[group_index]
        .iter()
        .filter(|n| !matches!(n[0] & 0x1F, 7 | 8))
        .count();
    assert_eq!(
        missing, damaged_vcl,
        "exactly the damaged AU's NALs must be lost — emitting a *shorter* AU          (missing {missing} of the damaged AU's {damaged_vcl}) is the truncated          reassembly this guards against, and losing more would mean the next          AU was dropped with it"
    );
    // And the AU after the damaged one is delivered intact.
    assert!(
        ir.tracks[0].samples.iter().any(|s| s.data
            == originals[group_index + 1]
                .iter()
                .fold(Vec::new(), |mut acc, n| {
                    acc.extend_from_slice(&(n.len() as u32).to_be_bytes());
                    acc.extend_from_slice(n);
                    acc
                })),
        "the AU after the damaged one must be delivered intact"
    );
}

/// A gap inside an FU-A run — a fragment lost from the **middle** — must stop
/// that run: the fragments after the hole belong to a NAL whose middle is
/// missing, so they must not be concatenated onto it (which would emit a NAL
/// silently short of its middle) nor joined to a later run.
///
/// (The sibling test below covers a gap immediately *before* an FU-A start.)
#[test]
fn gap_inside_an_fu_a_run_stops_that_run() {
    let media = demux_fixture();
    let out = packetise(&media);
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let originals = original_video_nals(&media);
    let original_flat: Vec<Vec<u8>> = originals.iter().flatten().cloned().collect();

    // Locate the longest FU-A run and drop a fragment from its middle.
    const NAL_TYPE_FU_A: u8 = 28;
    let (start, len) = {
        let mut best = (0usize, 0usize);
        let mut i = 0usize;
        while i < packets.len() {
            if packets[i][RTP_HEADER_LEN] & 0x1F != NAL_TYPE_FU_A {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < packets.len() && packets[j][RTP_HEADER_LEN] & 0x1F == NAL_TYPE_FU_A {
                j += 1;
            }
            if j - i > best.1 {
                best = (i, j - i);
            }
            i = j;
        }
        best
    };
    assert!(
        len >= 3,
        "the fixture must have an FU-A run of >=3 fragments"
    );
    let mut damaged = packets.clone();
    damaged.remove(start + len / 2);

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, damaged)],
        })
        .expect("the stream still depacketises");
    let recovered: Vec<Vec<u8>> = ir.tracks[0]
        .samples
        .iter()
        .flat_map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect::<Vec<Vec<u8>>>()
        })
        .collect();
    // No NAL may have been assembled across the hole: every recovered NAL is
    // one of the originals, byte for byte.
    for nal in &recovered {
        if nal.is_empty() || matches!(nal[0] & 0x1F, 7 | 8) {
            continue;
        }
        assert!(
            original_flat.contains(nal),
            "a NAL of {} bytes was assembled across the hole: {:02X?}",
            nal.len(),
            &nal[..nal.len().min(8)]
        );
    }
    // The damaged NAL is gone, and the stream continues to the end.
    assert!(
        recovered.len() < original_flat.len(),
        "the damaged NAL must be dropped"
    );
    assert_eq!(
        ir.tracks[0].samples.last().map(|s| s.data.clone()),
        Some(
            originals
                .last()
                .expect("originals")
                .iter()
                .fold(Vec::new(), |mut acc, n| {
                    acc.extend_from_slice(&(n.len() as u32).to_be_bytes());
                    acc.extend_from_slice(n);
                    acc
                })
                .into()
        ),
        "the final access unit must be intact"
    );
}

/// A gap immediately **before** an FU-A start means the run's first fragment
/// was lost (or the previous AU lost its last packet). The run cannot be
/// reassembled, and the packets after it must still be delivered.
#[test]
fn gap_before_an_fu_a_start_loses_only_the_runs_au() {
    let media = demux_fixture();
    let out = packetise(&media);
    let packets: Vec<Vec<u8>> = video_stream(&out)
        .packets
        .iter()
        .map(|p| p.as_contiguous().to_vec())
        .collect();
    let originals = original_video_nals(&media);
    let original_flat: Vec<Vec<u8>> = originals.iter().flatten().cloned().collect();

    // Locate an FU-A start fragment and drop the packet *before* it, so the
    // gap falls immediately before the start.
    const NAL_TYPE_FU_A: u8 = 28;
    const FU_START_MASK: u8 = 0x80;
    let start = packets
        .iter()
        .position(|p| {
            (p[RTP_HEADER_LEN] & 0x1F) == NAL_TYPE_FU_A
                && (p[RTP_HEADER_LEN + 1] & FU_START_MASK) != 0
        })
        .expect("the fixture must contain an FU-A start");
    assert!(start > 0, "there must be a packet before the FU-A start");
    let mut damaged = packets.clone();
    damaged.remove(start - 1);

    let ir = RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: vec![RtpInputStream::new(RtpMediaKind::H264, damaged)],
        })
        .expect("the stream still depacketises");

    let recovered: Vec<Vec<u8>> = ir.tracks[0]
        .samples
        .iter()
        .flat_map(|s| {
            transmux::annexb::iter_length_prefixed_nals(&s.data)
                .unwrap()
                .into_iter()
                .map(|n| n.to_vec())
                .collect::<Vec<Vec<u8>>>()
        })
        .collect();
    for nal in &recovered {
        if nal.is_empty() || matches!(nal[0] & 0x1F, 7 | 8) {
            continue;
        }
        assert!(
            original_flat.contains(nal),
            "recovered NAL type {} of {} bytes is a truncated reassembly",
            nal[0] & 0x1F,
            nal.len()
        );
    }
    // The stream continues: the AUs after the damaged run are delivered.
    let vcl = |v: &[Vec<u8>]| -> usize {
        v.iter()
            .filter(|n| !n.is_empty() && !matches!(n[0] & 0x1F, 7 | 8))
            .count()
    };
    let original_vcl = vcl(&original_flat);
    let recovered_vcl = vcl(&recovered);
    assert!(
        recovered_vcl < original_vcl,
        "the damaged run's NAL must be dropped ({recovered_vcl} of {original_vcl}          recovered)"
    );
    // Only the damaged AU's NALs are lost: the gap falls before the FU-A start,
    // so the AU whose *previous* packet went missing is the damaged one, and
    // the run that starts on that gap is short one fragment — at most a couple
    // of NALs, never the tail of the stream.
    assert!(
        recovered_vcl + 4 >= original_vcl,
        "the loss must be local to the damaged AU, not the tail of the stream          ({recovered_vcl} of {original_vcl})"
    );
    // The fragments *after* the gap belong to an FU-A run that lost its start
    // (or whose AU lost its last packet): they must not be made into a NAL of
    // their own, nor joined to a later run. Their payload bytes start with the
    // original NAL's body from the second octet on, so a NAL built from them
    // would begin with the fragment's own continuation byte — checked by
    // requiring every recovered NAL to be one of the originals.
    for nal in &recovered {
        if nal.is_empty() || matches!(nal[0] & 0x1F, 7 | 8) {
            continue;
        }
        assert!(
            original_flat.contains(nal),
            "a NAL was assembled from fragments across the hole: {:02X?}",
            &nal[..nal.len().min(8)]
        );
    }
    // The very last AU of the fixture is delivered byte-identically.
    assert_eq!(
        ir.tracks[0].samples.last().map(|s| s.data.clone()),
        Some(
            originals
                .last()
                .expect("originals")
                .iter()
                .fold(Vec::new(), |mut acc, n| {
                    acc.extend_from_slice(&(n.len() as u32).to_be_bytes());
                    acc.extend_from_slice(n);
                    acc
                })
                .into()
        ),
        "the final access unit must be intact"
    );
}

// ── Test 5: SDP correctness against the demuxed config ───────────────────────

#[test]
fn sdp_matches_demuxed_config() {
    let media = demux_fixture();
    let out = packetise(&media);
    let sdp = &out.sdp;

    assert!(sdp.contains("m=video"), "SDP has m=video");
    assert!(sdp.contains("m=audio"), "SDP has m=audio");
    assert!(
        sdp.contains(&format!("H264/{VIDEO_CLOCK_RATE}")),
        "video rtpmap uses the 90 kHz clock"
    );

    // Audio rtpmap uses the demuxed sample rate + channels.
    let (rate, channels, asc) = match &media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .unwrap()
        .spec
        .config
    {
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => {
            let asc = esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap()
                .data
                .clone();
            (*sample_rate, *channel_count, asc)
        }
        _ => unreachable!(),
    };
    assert!(
        sdp.contains(&format!("mpeg4-generic/{rate}/{channels}")),
        "audio rtpmap uses the demuxed rate/{{channels}}"
    );

    // sprop-parameter-sets base64-decodes to the demuxed SPS + PPS.
    let sps = match &media.tracks[0].spec.config {
        CodecConfig::Avc { config, .. } => config.config.sps[0].0.clone(),
        _ => unreachable!(),
    };
    let pps = match &media.tracks[0].spec.config {
        CodecConfig::Avc { config, .. } => config.config.pps[0].0.clone(),
        _ => unreachable!(),
    };
    let sprop = extract_param(sdp, "sprop-parameter-sets=");
    let parts: Vec<&str> = sprop.split(',').collect();
    assert_eq!(parts.len(), 2, "sprop has SPS,PPS");
    assert_eq!(
        base64_decode(parts[0]).unwrap(),
        sps,
        "sprop[0] == demuxed SPS"
    );
    assert_eq!(
        base64_decode(parts[1]).unwrap(),
        pps,
        "sprop[1] == demuxed PPS"
    );

    // config= hex-decodes to the demuxed ASC.
    let cfg = extract_param(sdp, "config=");
    assert_eq!(hex_decode(&cfg).unwrap(), asc, "config == demuxed ASC");
}

/// Extract a `key=value` fmtp parameter value (up to `;`, whitespace, or EOL).
fn extract_param(sdp: &str, key: &str) -> String {
    let start = sdp.find(key).unwrap_or_else(|| panic!("SDP has {key}")) + key.len();
    let tail = &sdp[start..];
    let end = tail.find([';', '\r', '\n', ' ']).unwrap_or(tail.len());
    tail[..end].to_string()
}
