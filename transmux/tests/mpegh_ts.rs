//! MPEG-H 3D Audio Transport-Stream spoke gate — TS→IR demux + IR→TS mux
//! (issue #579).
//!
//! Fixture: `private/fixtures/ts/mpegh-cicp01-baseline.ts` — a real
//! Fraunhofer-IIS `mpegh-test-content` capture (CICP_01, baseline profile,
//! 32 kbps, single-stream), `stream_type 0x2D`, ES_info carrying the
//! `MPEG-H_3dAudio_descriptor` (`0x3F` extension descriptor). CC BY-NC-ND —
//! lives in the private `private/` submodule, not the public tree; this
//! test **skips cleanly** (mirrors `scte35-splice/tests/downloaded_scte35.rs`)
//! when the submodule isn't checked out.
//!
//! Gate 3 below is an **independent oracle**: it re-implements the MHAS
//! packet walk from scratch (its own three-tier "escaped value" bit reader,
//! not `transmux::mpegh`'s — which is `pub(crate)` and unreachable from an
//! external test anyway) to find the `PACTYP_MPEGH3DACFG` packet's byte
//! offset/length in the raw PES payload, and checks it against the bytes the
//! demuxer actually recovered — so it bites a demuxer that finds the wrong
//! packet or off-by-one boundary, not just "some non-empty config".

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::media::{CmafMux, Media};
use transmux::pipeline::CodecConfig;
use transmux::{Fmp4Demux, MHADecoderConfigurationRecord, TsDemux, TsMux};

const TS: usize = 188;

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("private")
        .join("fixtures")
        .join("ts")
        .join("mpegh-cicp01-baseline.ts")
}

/// Load the fixture, or return `None` (with a SKIP notice) if the private
/// submodule isn't checked out.
fn try_load_fixture() -> Option<Vec<u8>> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "mpegh_ts: SKIPPED — {} not present. Run \
             `git submodule update --init private` to enable (maintainers only; \
             CC BY-NC-ND, not publicly redistributable).",
            path.display()
        );
        return None;
    }
    Some(std::fs::read(&path).expect("read mpegh-cicp01-baseline.ts fixture"))
}

/// Run `body` with the fixture bytes, or skip the test cleanly.
fn with_fixture(body: impl FnOnce(&[u8])) {
    if let Some(data) = try_load_fixture() {
        body(&data);
    }
}

fn demux_fixture(data: &[u8]) -> Media {
    TsDemux::new()
        .unpackage(data)
        .expect("demux mpegh-cicp01-baseline.ts")
}

fn mpegh_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::MpegH { .. }))
        .expect("an MPEG-H track in the demuxed media")
}

// ---------------------------------------------------------------------------
// Minimal TS/PSI/PES walking (byte-level, independent of crate internals) —
// mirrors tests/dts_ts.rs / tests/dolby.rs.
// ---------------------------------------------------------------------------

fn pid_of(pkt: &[u8]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

fn pusi_of(pkt: &[u8]) -> bool {
    pkt[1] & 0x40 != 0
}

fn payload_offset(pkt: &[u8]) -> usize {
    let afc = (pkt[3] >> 4) & 0x3;
    let has_af = afc & 0b10 != 0;
    let has_payload = afc & 0b01 != 0;
    if !has_payload {
        return TS;
    }
    if has_af { 4 + 1 + pkt[4] as usize } else { 4 }
}

/// Reassemble the first complete single-packet PSI section on `pid`.
fn first_section(ts: &[u8], pid: u16) -> Option<Vec<u8>> {
    for pkt in ts.chunks_exact(TS) {
        if pid_of(pkt) != pid || !pusi_of(pkt) {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        let payload = &pkt[off..];
        let ptr = payload[0] as usize;
        let sec_start = 1 + ptr;
        if sec_start + 3 > payload.len() {
            continue;
        }
        let sec = &payload[sec_start..];
        let section_length = (((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize;
        let total = 3 + section_length;
        if total > sec.len() {
            continue;
        }
        return Some(sec[..total].to_vec());
    }
    None
}

fn parse_pat(sec: &[u8]) -> Vec<(u16, u16)> {
    let body = &sec[8..sec.len() - 4];
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= body.len() {
        let prog = u16::from_be_bytes([body[i], body[i + 1]]);
        let pmt_pid = (((body[i + 2] & 0x1F) as u16) << 8) | body[i + 3] as u16;
        out.push((prog, pmt_pid));
        i += 4;
    }
    out
}

struct PmtEntry {
    stream_type: u8,
    pid: u16,
    descriptors: Vec<u8>,
}

/// `(stream_type, elementary_PID, ES_info descriptor bytes)` from a PMT section.
fn parse_pmt(sec: &[u8]) -> Vec<PmtEntry> {
    let body = &sec[8..sec.len() - 4];
    let program_info_length = (((body[2] & 0x0F) as usize) << 8) | body[3] as usize;
    let mut i = 4 + program_info_length;
    let mut out = Vec::new();
    while i + 5 <= body.len() {
        let stream_type = body[i];
        let es_pid = (((body[i + 1] & 0x1F) as u16) << 8) | body[i + 2] as u16;
        let es_info_len = (((body[i + 3] & 0x0F) as usize) << 8) | body[i + 4] as usize;
        let desc_start = i + 5;
        let desc_end = (desc_start + es_info_len).min(body.len());
        out.push(PmtEntry {
            stream_type,
            pid: es_pid,
            descriptors: body[desc_start..desc_end].to_vec(),
        });
        i += 5 + es_info_len;
    }
    out
}

/// Find the MPEG-H elementary PID (stream_type 0x2D) via PAT → PMT.
fn find_mpegh_es(ts: &[u8]) -> PmtEntry {
    let pat = first_section(ts, 0x0000).expect("PAT must be present");
    let programs = parse_pat(&pat);
    let pmt_pid = programs
        .iter()
        .find(|(prog, _)| *prog != 0)
        .map(|(_, pid)| *pid)
        .expect("PAT must list a program");
    let pmt = first_section(ts, pmt_pid).expect("PMT must resolve from PAT");
    parse_pmt(&pmt)
        .into_iter()
        .find(|e| e.stream_type == 0x2D)
        .expect("PMT must carry an MPEG-H (0x2D) elementary stream")
}

/// Reassemble every PES access unit's payload on `target_pid` (adaptation
/// fields / PUSI pointer fields stripped by hand — no crate internals used).
fn reassemble_pes_payloads(ts: &[u8], target_pid: u16) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for pkt in ts.chunks_exact(TS) {
        if pkt[0] != 0x47 || pid_of(pkt) != target_pid {
            continue;
        }
        let off = payload_offset(pkt);
        if off >= TS {
            continue;
        }
        let mut payload = &pkt[off..];
        if pusi_of(pkt) {
            assert_eq!(&payload[0..3], &[0x00, 0x00, 0x01], "PES start code");
            let hdr_len = payload[8] as usize;
            out.push(Vec::new());
            payload = &payload[9 + hdr_len..];
        }
        if let Some(last) = out.last_mut() {
            last.extend_from_slice(payload);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Independent MHAS packet walk (Gate 3's oracle) — deliberately re-derived
// from scratch, not calling `transmux::mpegh`.
// ---------------------------------------------------------------------------

/// Read `n` bits (MSB-first) starting at bit offset `pos`; returns the value
/// and the advanced bit position.
fn read_bits(data: &[u8], pos: usize, n: usize) -> (u64, usize) {
    let mut v = 0u64;
    let mut p = pos;
    for _ in 0..n {
        let byte = data[p / 8];
        let bit = (byte >> (7 - (p % 8))) & 1;
        v = (v << 1) | bit as u64;
        p += 1;
    }
    (v, p)
}

fn escaped_value(data: &[u8], pos: usize, n1: usize, n2: usize, n3: usize) -> (u64, usize) {
    let (mut value, mut p) = read_bits(data, pos, n1);
    if value == (1u64 << n1) - 1 {
        let (v2, p2) = read_bits(data, p, n2);
        value += v2;
        p = p2;
        if v2 == (1u64 << n2) - 1 {
            let (v3, p3) = read_bits(data, p, n3);
            value += v3;
            p = p3;
        }
    }
    (value, p)
}

/// Walk one MHAS access unit, returning `(packet_type, byte_start, byte_len)`
/// for every packet found.
fn independent_mhas_walk(data: &[u8]) -> Vec<(u8, usize, usize)> {
    let mut out = Vec::new();
    let mut bitpos = 0usize;
    while bitpos + 16 <= data.len() * 8 {
        let (ptype, p1) = escaped_value(data, bitpos, 3, 8, 8);
        let (_label, p2) = escaped_value(data, p1, 2, 8, 32);
        let (length, p3) = escaped_value(data, p2, 11, 24, 24);
        if p3 % 8 != 0 {
            break;
        }
        let start = p3 / 8;
        let len = length as usize;
        if start + len > data.len() {
            break;
        }
        out.push((ptype as u8, start, len));
        bitpos = (start + len) * 8;
    }
    out
}

const MHAS_PACTYP_MPEGH3DACFG: u8 = 1;

// ---------------------------------------------------------------------------
// Gate 1: demux enumerates the MPEG-H track; stream_type/descriptor
// recognised via the independent PAT/PMT walk; config recovered non-empty.
// ---------------------------------------------------------------------------

#[test]
fn demux_enumerates_mpegh_track_with_recovered_config() {
    with_fixture(|data| {
        let es = find_mpegh_es(data);
        assert_eq!(es.stream_type, 0x2D, "fixture stream_type must be 0x2D");
        assert!(
            !es.descriptors.is_empty(),
            "fixture ES_info must carry the MPEG-H_3dAudio_descriptor"
        );
        assert_eq!(
            es.descriptors[0], 0x3F,
            "ES_info descriptor must be an extension_descriptor (tag 0x3F)"
        );

        let media = demux_fixture(data);
        let mpegh_tracks: Vec<_> = media
            .tracks
            .iter()
            .filter(|t| matches!(t.config(), CodecConfig::MpegH { .. }))
            .collect();
        assert_eq!(mpegh_tracks.len(), 1, "exactly one MPEG-H track expected");

        let track = mpegh_tracks[0];
        let CodecConfig::MpegH { config, .. } = track.config() else {
            unreachable!()
        };
        assert!(
            !config.mpegh3da_config.is_empty(),
            "mpegh3daConfig must be recovered non-empty"
        );
        assert_eq!(
            config.configuration_version, 1,
            "configurationVersion must be 1 (ISO/IEC 23008-3 §20)"
        );
        assert_eq!(
            config.mpegh3da_profile_level_indication, config.mpegh3da_config[0],
            "the record's profile-level field must equal mpegh3daConfig()'s leading byte"
        );

        assert!(
            !track.samples.is_empty(),
            "expected at least one MPEG-H sample"
        );
        eprintln!(
            "mpegh_ts: {} samples, mpegh3daConfig {} bytes, profile-level 0x{:02X}",
            track.samples.len(),
            config.mpegh3da_config.len(),
            config.mpegh3da_profile_level_indication
        );
    });
}

// ---------------------------------------------------------------------------
// Gate 2: TS -> IR -> fMP4 -> IR round-trip: mhaC config + sample bytes
// byte-identical (via CmafMux -> Fmp4Demux, the existing mhm1/mhaC path).
// ---------------------------------------------------------------------------

#[test]
fn fmp4_round_trip_preserves_config_and_sample_bytes() {
    with_fixture(|data| {
        let media = demux_fixture(data);
        let track1 = mpegh_track(&media);
        let CodecConfig::MpegH { config: cfg1, .. } = track1.config() else {
            unreachable!()
        };
        let mut cfg1_bytes = vec![0u8; cfg1.serialized_len()];
        cfg1.serialize_into(&mut cfg1_bytes).unwrap();

        let mut mux = CmafMux::new(1);
        let fmp4 = mux.package(&media).expect("mux to fMP4");

        let media2 = Fmp4Demux::new()
            .unpackage(&fmp4)
            .expect("demux round-tripped fMP4");
        let track2 = mpegh_track(&media2);
        let CodecConfig::MpegH { config: cfg2, .. } = track2.config() else {
            unreachable!()
        };
        let mut cfg2_bytes = vec![0u8; cfg2.serialized_len()];
        cfg2.serialize_into(&mut cfg2_bytes).unwrap();

        assert_eq!(
            cfg1_bytes, cfg2_bytes,
            "mhaC record must round-trip byte-identical through fMP4"
        );

        assert_eq!(track2.samples.len(), track1.samples.len());
        for (i, (a, b)) in track1.samples.iter().zip(&track2.samples).enumerate() {
            assert_eq!(
                a.data, b.data,
                "sample {i}: MHAS access-unit bytes must be byte-identical after the fMP4 round-trip"
            );
        }

        // Cross-check MHADecoderConfigurationRecord::parse also agrees.
        let reparsed = MHADecoderConfigurationRecord::parse(&cfg2_bytes).unwrap();
        assert_eq!(&reparsed, cfg2);
    });
}

// ---------------------------------------------------------------------------
// Gate 3: independent MHAS-packet walk confirms the config packet offset
// the demuxer used.
// ---------------------------------------------------------------------------

#[test]
fn independent_mhas_walk_confirms_config_packet_offset() {
    with_fixture(|data| {
        let es = find_mpegh_es(data);
        let access_units = reassemble_pes_payloads(data, es.pid);
        assert!(
            access_units.len() > 1,
            "fixture must carry more than one MHAS access unit"
        );

        // Find every access unit that independently walks to a
        // PACTYP_MPEGH3DACFG packet (a RAP, per ETSI TS 101 154 §6.8.4.1).
        let mut oracle_configs: Vec<Vec<u8>> = Vec::new();
        for au in &access_units {
            for (ptype, start, len) in independent_mhas_walk(au) {
                if ptype == MHAS_PACTYP_MPEGH3DACFG {
                    oracle_configs.push(au[start..start + len].to_vec());
                }
            }
        }
        assert!(
            !oracle_configs.is_empty(),
            "the independent walk must find at least one PACTYP_MPEGH3DACFG packet"
        );
        // Every RAP in this (single-config) fixture must carry the identical
        // mpegh3daConfig() blob.
        assert!(
            oracle_configs.iter().all(|c| c == &oracle_configs[0]),
            "all RAPs in this fixture must carry the identical mpegh3daConfig()"
        );

        let media = demux_fixture(data);
        let track = mpegh_track(&media);
        let CodecConfig::MpegH { config, .. } = track.config() else {
            unreachable!()
        };

        assert_eq!(
            config.mpegh3da_config, oracle_configs[0],
            "the demuxer's recovered mpegh3daConfig must equal the independently-walked \
             PACTYP_MPEGH3DACFG packet payload byte-for-byte"
        );
    });
}

// ---------------------------------------------------------------------------
// Gate 4: TS mux round-trip — sample bytes byte-identical, PMT carries 0x2D
// + a well-formed MPEG-H_3dAudio_descriptor.
// ---------------------------------------------------------------------------

#[test]
fn ts_mux_round_trip_preserves_samples_stream_type_and_descriptor() {
    with_fixture(|data| {
        let media = demux_fixture(data);
        let track1 = mpegh_track(&media);
        let CodecConfig::MpegH { config, .. } = track1.config() else {
            unreachable!()
        };

        let ts2 = TsMux::new().package(&media).expect("mux IR back to TS");

        let es2 = find_mpegh_es(&ts2);
        assert_eq!(
            es2.stream_type, 0x2D,
            "muxed PMT must carry stream_type 0x2D"
        );
        assert_eq!(
            es2.descriptors[0], 0x3F,
            "muxed ES_info must carry an extension_descriptor (tag 0x3F)"
        );
        assert_eq!(
            es2.descriptors[2], 0x08,
            "extension_descriptor_tag must be MPEG-H (0x08)"
        );
        assert_eq!(
            es2.descriptors[3], config.mpegh3da_profile_level_indication,
            "muxed descriptor's profile-level byte must match the IR's config"
        );

        let media2 = TsDemux::new()
            .unpackage(&ts2)
            .expect("re-demux muxed output");
        let track2 = mpegh_track(&media2);

        assert_eq!(track2.samples.len(), track1.samples.len());
        for (i, (a, b)) in track1.samples.iter().zip(&track2.samples).enumerate() {
            assert_eq!(
                a.data, b.data,
                "sample {i}: MHAS access-unit bytes must be byte-identical after the TS mux round-trip"
            );
        }

        let CodecConfig::MpegH { config: cfg2, .. } = track2.config() else {
            unreachable!()
        };
        assert_eq!(
            cfg2.mpegh3da_config, config.mpegh3da_config,
            "mpegh3daConfig must survive the TS mux round-trip"
        );
    });
}
