//! Gate for the public NAL-type / IDR-IRAP keyframe helper (issue #517).
//!
//! Real fixtures only for the positive cases:
//! - `fixtures/ts/h264_aac.ts` (AVC) — demuxed via [`TsDemux`]; the helper's
//!   verdict on each length-prefixed sample must agree with the demuxer's own
//!   `is_sync` flag AND with the committed ffprobe keyframe oracle.
//! - `fixtures/ts/hevc/main.ts` (HEVC) — access units reassembled by PES
//!   boundary; the first AU is an IRAP keyframe, the rest are not.
//! - `fixtures/mp4/frag/vvc.frag.mp4` (VVC) — real vvenc bitstream; the
//!   length-prefixed NALs in the `mdat` carry a real VVC NAL header we classify.

use std::path::PathBuf;

use transmux::nal::{access_unit_is_keyframe, is_keyframe_nal, nal_unit_type, NalCodec};
use transmux::pipeline::CodecConfig;
use transmux::TsDemux;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

// ── Test 1: AVC — helper agrees with demuxer is_sync and the ffprobe oracle ──

/// Count keyframes in the ffprobe oracle CSV's `video` rows (7th column == 1).
fn oracle_video_keyframe_count() -> usize {
    let text = std::fs::read_to_string(fixtures_dir().join("ts/demux-oracle/h264_aac.packets.csv"))
        .expect("packets.csv oracle must exist");
    let mut count = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f[0] == "video" && f[6] == "1" {
            count += 1;
        }
    }
    count
}

#[test]
fn avc_keyframes_agree_with_demuxer_and_oracle() {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts")).expect("h264_aac.ts fixture");
    let media = TsDemux::new().demux(&ts).expect("demux h264_aac.ts");

    // Find the AVC video track.
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present");

    assert!(!video.samples.is_empty(), "video track must have samples");

    // The demuxer stores length-prefixed sample data. The helper's verdict on
    // each sample must match the demuxer's own is_sync flag — the whole point of
    // the refactor (single source of truth).
    let mut helper_kf = 0usize;
    for s in &video.samples {
        let helper = access_unit_is_keyframe(NalCodec::Avc, &s.data, true);
        assert_eq!(
            helper, s.is_sync,
            "helper keyframe verdict must equal demuxer is_sync for every sample"
        );
        if helper {
            helper_kf += 1;
        }
    }

    // ...and both must equal the external ffprobe oracle's keyframe count.
    let oracle_kf = oracle_video_keyframe_count();
    assert!(oracle_kf > 0, "oracle must report at least one keyframe");
    assert_eq!(
        helper_kf, oracle_kf,
        "helper keyframe count must equal the ffprobe oracle"
    );

    // Bite check: not every AU is a keyframe (otherwise the test is trivial).
    assert!(
        helper_kf < video.samples.len(),
        "stream must contain non-keyframe AUs too"
    );
}

// ── Test 2: HEVC — first AU is IRAP, a later AU is not ───────────────────────

/// Reassemble the video elementary stream's access units from a TS by PES
/// boundary (PUSI marks a new PES packet = one AU here) for `video_pid`.
/// Returns each AU as its **Annex B** ES payload (PES header stripped).
fn hevc_access_units(ts: &[u8], video_pid: u16) -> Vec<Vec<u8>> {
    const TS_PACKET: usize = 188;
    let mut aus: Vec<Vec<u8>> = Vec::new();
    let mut cur: Option<Vec<u8>> = None;
    for pkt in ts.chunks_exact(TS_PACKET) {
        if pkt[0] != 0x47 {
            continue;
        }
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if pid != video_pid {
            continue;
        }
        let pusi = pkt[1] & 0x40 != 0;
        let afc = (pkt[3] >> 4) & 0x3;
        let mut off = 4usize;
        if afc & 0x2 != 0 {
            off += 1 + pkt[4] as usize; // adaptation_field_length + the field
        }
        if afc & 0x1 == 0 || off >= TS_PACKET {
            continue;
        }
        let payload = &pkt[off..];
        if pusi {
            if let Some(prev) = cur.take() {
                aus.push(prev);
            }
            cur = Some(payload.to_vec());
        } else if let Some(c) = cur.as_mut() {
            c.extend_from_slice(payload);
        }
    }
    if let Some(c) = cur {
        aus.push(c);
    }
    aus
}

/// Strip a PES packet header, returning the elementary-stream Annex B payload.
fn pes_es_payload(pes: &[u8]) -> &[u8] {
    assert_eq!(&pes[0..3], &[0x00, 0x00, 0x01], "PES start-code prefix");
    // Optional PES header: byte 8 is PES_header_data_length.
    let header_data_len = pes[8] as usize;
    &pes[9 + header_data_len..]
}

#[test]
fn hevc_first_au_is_irap_rest_are_not() {
    let ts = std::fs::read(fixtures_dir().join("ts/hevc/main.ts")).expect("hevc/main.ts fixture");
    // Video elementary PID for this capture (HEVC stream_type 0x24).
    const HEVC_VIDEO_PID: u16 = 0x0100;
    let aus = hevc_access_units(&ts, HEVC_VIDEO_PID);
    assert!(
        aus.len() > 1,
        "need multiple AUs to bite (got {})",
        aus.len()
    );

    // First AU carries VPS/SPS/PPS + an IDR (IRAP): keyframe.
    let first = pes_es_payload(&aus[0]);
    assert!(
        access_unit_is_keyframe(NalCodec::Hevc, first, false),
        "first HEVC AU must be an IRAP keyframe"
    );

    // Every subsequent AU is a TRAIL picture (nal_unit_type 0/1): not IRAP.
    let mut non_irap_seen = 0usize;
    for au in &aus[1..] {
        let es = pes_es_payload(au);
        assert!(
            !access_unit_is_keyframe(NalCodec::Hevc, es, false),
            "non-first HEVC AU must not be an IRAP keyframe"
        );
        non_irap_seen += 1;
    }
    assert!(non_irap_seen > 0, "must observe at least one non-IRAP AU");

    // Cross-check NAL-type extraction on the first AU: it must contain an SPS
    // (type 33) and an IRAP VCL NAL (16..=23) — real values, not hand bytes.
    let mut saw_sps = false;
    let mut saw_irap = false;
    for nal in transmux::annexb::iter_annexb_nals(first) {
        match nal_unit_type(NalCodec::Hevc, nal) {
            Some(33) => saw_sps = true,
            Some(t) if (16..=23).contains(&t) => saw_irap = true,
            _ => {}
        }
    }
    assert!(saw_sps, "first HEVC AU must contain an SPS (type 33)");
    assert!(saw_irap, "first HEVC AU must contain an IRAP VCL NAL");
}

// ── Test 3: VVC — classify real vvenc NALs from the mdat ─────────────────────

/// Walk boxes at `data`, recursing container boxes, to find the first `four_cc`.
fn find_box_body<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> Option<&'a [u8]> {
    const CONTAINERS: &[&[u8; 4]] = &[
        b"moov", b"trak", b"mdia", b"minf", b"stbl", b"moof", b"traf",
    ];
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > data.len() {
            break;
        }
        let ty: &[u8; 4] = data[off + 4..off + 8].try_into().unwrap();
        if ty == four_cc {
            return Some(&data[off + 8..off + size]);
        }
        if CONTAINERS.contains(&ty) {
            if let Some(found) = find_box_body(&data[off + 8..off + size], four_cc) {
                return Some(found);
            }
        }
        off += size;
    }
    None
}

#[test]
fn vvc_classifies_real_mdat_nals() {
    let mp4 = std::fs::read(fixtures_dir().join("mp4/frag/vvc.frag.mp4")).expect("vvc.frag.mp4");
    // The `mdat` holds the coded VVC access unit(s) as 4-byte length-prefixed NALs.
    let mdat = find_box_body(&mp4, b"mdat").expect("mdat box present");

    let nals = transmux::annexb::iter_length_prefixed_nals(mdat).expect("length-prefixed mdat");
    assert!(!nals.is_empty(), "mdat must carry NAL units");

    // Every NAL classifies (2-byte header present) and we must see a real IRAP
    // VCL keyframe NAL in this intra-coded fixture.
    let mut saw_keyframe = false;
    for nal in &nals {
        let t = nal_unit_type(NalCodec::Vvc, nal).expect("VVC NAL header present");
        // VVC VCL/keyframe types are single-byte-representable; sanity bound.
        assert!(t < 32, "VVC nal_unit_type is 5 bits");
        if is_keyframe_nal(NalCodec::Vvc, nal) {
            saw_keyframe = true;
        }
    }
    assert!(
        saw_keyframe,
        "the intra VVC fixture must contain an IRAP keyframe NAL (IDR/CRA)"
    );

    // The whole mdat as a length-prefixed access unit is a keyframe.
    assert!(access_unit_is_keyframe(NalCodec::Vvc, mdat, true));
}

// ── Test 4: Annex B and length-prefixed forms of a REAL AU agree ─────────────

#[test]
fn annexb_and_length_prefixed_agree_on_real_hevc_au() {
    let ts = std::fs::read(fixtures_dir().join("ts/hevc/main.ts")).expect("hevc/main.ts fixture");
    const HEVC_VIDEO_PID: u16 = 0x0100;
    let aus = hevc_access_units(&ts, HEVC_VIDEO_PID);
    assert!(aus.len() > 1);

    for au in [&aus[0], &aus[1]] {
        let annexb = pes_es_payload(au);
        let lp = transmux::annexb::annexb_to_length_prefixed(annexb);
        let via_annexb = access_unit_is_keyframe(NalCodec::Hevc, annexb, false);
        let via_lp = access_unit_is_keyframe(NalCodec::Hevc, &lp, true);
        assert_eq!(
            via_annexb, via_lp,
            "Annex B and length-prefixed verdicts must match for the same real AU"
        );
    }
    // And they must actually differ between the two AUs (bite): AU0 keyframe, AU1 not.
    assert!(access_unit_is_keyframe(
        NalCodec::Hevc,
        pes_es_payload(&aus[0]),
        false
    ));
    assert!(!access_unit_is_keyframe(
        NalCodec::Hevc,
        pes_es_payload(&aus[1]),
        false
    ));
}
