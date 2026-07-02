//! RTP spoke gate — packetize/depacketize the demuxed `fixtures/ts/h264_aac.ts`
//! IR (75 video + 131 audio samples) and verify RFC 3550/6184/3640/4566 fidelity
//! against the real demuxed NALs / config (issue #469).
//!
//! Every test bites against the demuxed oracle, never hardcoded values.

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::rtp::{base64_decode, hex_decode};
use transmux::{
    Media, RtpDepacketizer, RtpInput, RtpInputStream, RtpMediaKind, RtpPacketizer, NAL_TYPE_IDR,
    VIDEO_CLOCK_RATE,
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

fn packetize(media: &Media) -> transmux::RtpOutput {
    let mut p = RtpPacketizer {
        mtu: MTU,
        ssrc: SSRC,
        ..RtpPacketizer::default()
    };
    p.package(media).expect("packetize IR → RTP")
}

fn parse_hdr(pkt: &[u8]) -> (u8, u8, bool, u16, u32, u32) {
    let version = pkt[0] >> 6;
    let marker = pkt[1] & 0x80 != 0;
    let pt = pkt[1] & 0x7F;
    let seq = u16::from_be_bytes([pkt[2], pkt[3]]);
    let ts = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
    let ssrc = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
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
    let out = packetize(&media);

    for stream in &out.streams {
        assert!(!stream.packets.is_empty(), "stream has packets");
        // Every packet: V=2, correct PT, fixed SSRC; strictly monotonic seq (+1).
        let mut expected_seq: Option<u16> = None;
        for pkt in &stream.packets {
            assert!(pkt.len() >= RTP_HEADER_LEN);
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
    let mut aus: Vec<Vec<&Vec<u8>>> = Vec::new();
    let mut cur: Vec<&Vec<u8>> = Vec::new();
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

    let mut prev_ts: Option<u32> = None;
    for au in &aus {
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
        // Timestamps advance by the 90 kHz per-frame delta.
        if let Some(p) = prev_ts {
            assert_eq!(ts0 - p, 3600, "video TS advances by 3600 (90kHz/25fps)");
        }
        prev_ts = Some(ts0);
    }

    // Audio: one packet per AU, marker set, timestamps advance by 1024 ticks.
    let as_ = audio_stream(&out);
    assert_eq!(as_.packets.len(), 131, "131 audio packets (one AU each)");
    let mut prev_a: Option<u32> = None;
    for pkt in &as_.packets {
        let (_v, _pt, marker, _seq, ts, _ssrc) = parse_hdr(pkt);
        assert!(marker, "audio marker set per packet");
        if let Some(p) = prev_a {
            assert_eq!(ts - p, 1024, "audio TS advances by the AAC frame length");
        }
        prev_a = Some(ts);
    }
}

// ── Test 2: FU-A fragmentation actually happens ──────────────────────────────

#[test]
fn fu_a_fragmentation_happens() {
    let media = demux_fixture();
    let out = packetize(&media);
    let vs = video_stream(&out);

    // Find FU-A packets (payload byte 0 low-5-bits == 28).
    let mut fu_packets = 0usize;
    let mut fu_starts = 0usize;
    let mut fu_ends = 0usize;
    let mut reconstructed_types = Vec::new();
    for pkt in &vs.packets {
        let payload = &pkt[RTP_HEADER_LEN..];
        let nal_type = payload[0] & 0x1F;
        if nal_type == 28 {
            fu_packets += 1;
            let fu_header = payload[1];
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
    let out = packetize(&media);
    let vs = video_stream(&out);

    let mut depack = RtpDepacketizer::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![RtpInputStream {
                kind: RtpMediaKind::H264,
                packets: vs.packets.clone(),
            }],
        })
        .expect("depacketize video");

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

    // The first depacketized AU carries the STAP-A parameter sets (SPS+PPS)
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
    let out = packetize(&media);
    let as_ = audio_stream(&out);

    let mut depack = RtpDepacketizer::new();
    let ir = depack
        .unpackage(RtpInput {
            streams: vec![RtpInputStream {
                kind: RtpMediaKind::Aac,
                packets: as_.packets.clone(),
            }],
        })
        .expect("depacketize audio");

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
    let mut broken = as_.packets[0].clone();
    // AU-header sits at payload offset [2..4]; corrupt the AU-size (top 13 bits).
    broken[RTP_HEADER_LEN + 2] ^= 0x08; // flips a bit in the AU-size field
    let mut d2 = RtpDepacketizer::new();
    let bad = d2.unpackage(RtpInput {
        streams: vec![RtpInputStream {
            kind: RtpMediaKind::Aac,
            packets: vec![broken],
        }],
    });
    // Either it errors (declared size overran) or the reassembled AU differs.
    if let Ok(m) = bad {
        assert_ne!(
            m.tracks[0].samples[0].data, audio_track.samples[0].data,
            "corrupt AU-size must not reproduce the original AU"
        );
    }
}

// ── Test 5: SDP correctness against the demuxed config ───────────────────────

#[test]
fn sdp_matches_demuxed_config() {
    let media = demux_fixture();
    let out = packetize(&media);
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
