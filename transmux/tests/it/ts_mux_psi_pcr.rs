//! `TsMux` PMT `stream_type` mapping, multi-packet PSI packetisation, the
//! PAT/PMT repetition schedule and the PCR-only filler (audit 21 finding 56).
//!
//! The `stream_type` values asserted here are the literals of ISO/IEC 13818-1
//! Table 2-34 (0x02 MPEG-2 video, 0x03 MPEG-1 audio, 0x04 MPEG-2 audio,
//! 0x05 private sections, 0x06 PES private data) and ISO/IEC 23008-3 (0x2D
//! MPEG-H 3D Audio). PSI framing is ISO/IEC 13818-1 §2.4.4: a payload of 184
//! bytes, a `pointer_field` of 0 on the PUSI packet, and 0xFF stuffing after
//! the section. PCR repetition targets TR 101 290 indicator 2.3 (40 ms).
#![cfg(feature = "std")]

use std::collections::BTreeMap;
use std::time::Duration;

use broadcast_common::Parse;
use broadcast_common::{Package, Unpackage};
use dvb_si::tables::pmt::PmtSection;
use mpeg_ts::ts::SectionReassembler;
use transmux::ir::{Track, TrackId, TrackSpec};
use transmux::mpegh::MHADecoderConfigurationRecord;
use transmux::pipeline::{CodecConfig, DataCarriage};
use transmux::{
    DEFAULT_PSI_INTERVAL, Media, ObjectTypeIndication, Sample, StreamingTsMux, TsDemux, TsMux,
};

const PKT: usize = 188;
const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
/// PSI payload capacity of the PUSI packet: 188 − 4-byte header − pointer_field
/// (ISO/IEC 13818-1 §2.4.4).
const FIRST_PACKET_SECTION_BYTES: usize = PKT - 5;
/// PSI payload capacity of a continuation packet.
const NEXT_PACKET_SECTION_BYTES: usize = PKT - 4;
/// TR 101 290 indicator 2.3 bound on the PCR interval, in 90 kHz ticks.
const MAX_PCR_INTERVAL_TICKS: u64 = 90_000 * 40 / 1000;

fn ts(rel: &str) -> Vec<u8> {
    test_fixtures::read(&format!("ts/{rel}"))
}

fn demux(bytes: &[u8]) -> Media {
    TsDemux::new().unpackage(bytes).expect("demux fixture")
}

fn timescale(v: u32) -> broadcast_common::timescale::Timescale {
    broadcast_common::timescale::Timescale::try_new(v).expect("non-zero timescale")
}

fn track(id: u32, config: CodecConfig, samples: Vec<Sample>) -> Track {
    Track::new(
        TrackSpec::new(TrackId::new(id), timescale(90_000), config),
        samples,
    )
}

fn data_config(stream_type: u8, descriptors: Vec<u8>, carriage: DataCarriage) -> CodecConfig {
    CodecConfig::Data {
        stream_type,
        descriptors,
        carriage,
    }
}

fn pids(ts: &[u8]) -> impl Iterator<Item = u16> + '_ {
    ts.chunks_exact(PKT)
        .map(|p| (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]))
}

fn payload(pkt: &[u8]) -> &[u8] {
    let start = if pkt[3] & 0x20 != 0 {
        5 + usize::from(pkt[4])
    } else {
        4
    };
    &pkt[start.min(PKT)..]
}

/// The first complete PMT section of a muxed stream, reassembled from its
/// packets (so a section split over several packets still arrives whole).
fn pmt_section_bytes(ts: &[u8]) -> Vec<u8> {
    let mut r = SectionReassembler::default();
    for p in ts
        .chunks_exact(PKT)
        .filter(|p| ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) == PMT_PID)
    {
        r.feed(payload(p), p[1] & 0x40 != 0);
        if let Some(s) = r.pop_section() {
            return s.to_vec();
        }
    }
    panic!("no complete PMT section in the muxed output");
}

/// `(stream_type, elementary_PID)` of every entry in the muxed PMT.
fn pmt_entries(ts: &[u8]) -> Vec<(u8, u16)> {
    let sec = pmt_section_bytes(ts);
    let pmt = PmtSection::parse(&sec).expect("muxed PMT parses");
    pmt.streams
        .iter()
        .map(|s| (s.stream_type.to_u8(), s.elementary_pid.value()))
        .collect()
}

/// Mux one track (with a single sample) and return `(output, PMT entries)`.
fn mux_one(config: CodecConfig, sample: Sample) -> (Vec<u8>, Vec<(u8, u16)>) {
    let media = Media::new(vec![track(1, config, vec![sample])], timescale(90_000));
    let out = TsMux::new().package(&media).expect("mux");
    let entries = pmt_entries(&out);
    (out, entries)
}

fn sample_at(dts: i64, duration: u32) -> Sample {
    Sample::new(vec![0xAAu8; 8], Some(dts), Some(dts), Some(duration), true).expect("valid timing")
}

// ---------------------------------------------------------------------------
// 1. PMT stream_type per carriage (ISO/IEC 13818-1 Table 2-34).
// ---------------------------------------------------------------------------

#[test]
fn mpeg2_video_maps_to_stream_type_0x02() {
    let media = demux(&ts("legacy/mpeg2-mp2.ts"));
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track in the legacy fixture")
        .clone();
    let out = TsMux::new()
        .package(&Media::new(vec![video], media.movie_timescale))
        .expect("mux");
    assert_eq!(
        pmt_entries(&out),
        vec![(0x02, 0x0100)],
        "MPEG-2 video is stream_type 0x02 on the single ES PID"
    );
}

#[test]
fn mpeg1_and_mpeg2_audio_map_to_0x03_and_0x04() {
    let media = demux(&ts("legacy/mpeg2-mp2.ts"));
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::MpegAudio { .. }))
        .expect("MPEG audio track in the legacy fixture")
        .clone();
    let CodecConfig::MpegAudio { esds, .. } = audio.config() else {
        panic!("expected MpegAudio");
    };
    let oti = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .expect("decoder config")
        .object_type_indication
        .0;
    assert_eq!(oti, 0x6B, "the legacy fixture's audio is MPEG-1 audio");

    let out = TsMux::new()
        .package(&Media::new(vec![audio.clone()], media.movie_timescale))
        .expect("mux MPEG-1 audio");
    assert_eq!(
        pmt_entries(&out),
        vec![(0x03, 0x0100)],
        "MPEG-1 audio (OTI 0x6B) is stream_type 0x03"
    );

    let mut mpeg2 = audio;
    let CodecConfig::MpegAudio { esds, .. } = &mut mpeg2.spec.config else {
        panic!("expected MpegAudio");
    };
    esds.es_descriptor
        .decoder_config
        .as_mut()
        .expect("decoder config")
        .object_type_indication = ObjectTypeIndication(0x69);
    let out = TsMux::new()
        .package(&Media::new(vec![mpeg2], media.movie_timescale))
        .expect("mux MPEG-2 audio");
    assert_eq!(
        pmt_entries(&out),
        vec![(0x04, 0x0100)],
        "MPEG-2 audio (OTI 0x69) is stream_type 0x04"
    );
}

#[test]
fn data_pes_and_sections_keep_their_stream_type() {
    let pes = data_config(0x06, Vec::new(), DataCarriage::Pes);
    let (out, entries) = mux_one(pes, sample_at(0, 1000));
    assert_eq!(entries, vec![(0x06, 0x0100)], "PES private data is 0x06");
    assert!(
        pids(&out).any(|p| p == 0x0100),
        "the data-PES elementary PID is in the output"
    );

    let sections = data_config(0x05, Vec::new(), DataCarriage::Sections);
    let (out, entries) = mux_one(sections, sample_at(0, 1000));
    assert_eq!(entries, vec![(0x05, 0x0100)], "private sections are 0x05");
    assert!(
        pids(&out).any(|p| p == 0x0100),
        "the section-carried elementary PID is in the output"
    );
}

#[test]
fn mpeg_h_maps_to_stream_type_0x2d() {
    let config = CodecConfig::MpegH {
        config: MHADecoderConfigurationRecord::new(0x0B, 0x02, vec![0x0B, 0x00]),
        channel_count: 2,
        sample_rate: 48_000,
        sample_size: 16,
    };
    let (out, entries) = mux_one(config, sample_at(0, 1024));
    assert_eq!(entries, vec![(0x2D, 0x0100)], "MPEG-H 3D Audio is 0x2D");
    assert!(
        pids(&out).any(|p| p == 0x0100),
        "the MPEG-H elementary PID is in the output"
    );
}

// ---------------------------------------------------------------------------
// 2. A PMT longer than one packet is split across TS packets (§2.4.4).
// ---------------------------------------------------------------------------

#[test]
fn pmt_over_one_packet_is_split_with_pusi_pointer_and_stuffing() {
    // Twelve opaque PES streams, each with a 12-byte ES_info descriptor loop:
    // 5 + 12 bytes per entry past a 16-byte section prefix and 4-byte CRC is
    // 220 bytes, past the 183 a single packet can carry.
    let descriptor = vec![0x86u8, 0x0A, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let tracks: Vec<Track> = (0..12)
        .map(|i| {
            let mut t = track(
                1 + i,
                data_config(0x06, Vec::new(), DataCarriage::Pes),
                vec![],
            );
            t.spec.es_info_descriptors = descriptor.clone();
            t
        })
        .collect();
    let media = Media::new(tracks, timescale(90_000));
    let out = TsMux::new().package(&media).expect("mux");

    let pmt_packets: Vec<&[u8]> = out
        .chunks_exact(PKT)
        .filter(|p| ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) == PMT_PID)
        .collect();
    assert_eq!(
        pmt_packets.len(),
        2,
        "220-byte PMT: one PUSI packet of 183 section bytes + one continuation"
    );
    assert!(
        pmt_packets[0][1] & 0x40 != 0,
        "PUSI on the first packet only"
    );
    assert!(pmt_packets[1][1] & 0x40 == 0, "no PUSI on the continuation");
    assert_eq!(payload(pmt_packets[0])[0], 0, "pointer_field is 0");
    assert_eq!(
        pmt_packets.iter().map(|p| p[3] & 0x0F).collect::<Vec<_>>(),
        vec![0, 1],
        "continuity_counter increments across the split"
    );

    // Reassembled, the section accounts for exactly its own length.
    let sec = pmt_section_bytes(&out);
    let declared = 3 + ((((sec[1] & 0x0F) as usize) << 8) | sec[2] as usize);
    assert_eq!(
        declared,
        sec.len(),
        "section_length accounts for the whole section"
    );
    assert_eq!(
        sec.len(),
        220,
        "the twelve 17-byte ES entries plus 16 bytes"
    );

    // The bytes after the CRC in the last packet are 0xFF stuffing.
    let carried_first = FIRST_PACKET_SECTION_BYTES;
    let carried_last = sec.len() - carried_first;
    assert_eq!(carried_last, 37, "second packet carries the rest");
    assert!(carried_last <= NEXT_PACKET_SECTION_BYTES);
    let tail = &payload(pmt_packets[1])[carried_last..];
    assert!(
        !tail.is_empty() && tail.iter().all(|&b| b == 0xFF),
        "stuffing after the section is all 0xFF: {tail:?}"
    );

    let entries = pmt_entries(&out);
    assert_eq!(entries.len(), 12, "every ES survives the split");
    assert!(entries.iter().all(|&(st, _)| st == 0x06));
    let mut unique: Vec<u16> = entries.iter().map(|&(_, pid)| pid).collect();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 12, "each ES has its own PID");
}

// ---------------------------------------------------------------------------
// 3. Periodic PAT/PMT repetition (TR 101 290 indicators 1.3/1.5).
// ---------------------------------------------------------------------------

/// 100 MPEG-2 video samples, one per 100 ms of 90 kHz time.
fn one_hz_video() -> Media {
    let samples: Vec<Sample> = (0..100).map(|k| sample_at(k * 9_000, 9_000)).collect();
    let config = demux(&ts("legacy/mpeg2-mp2.ts"))
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .config()
        .clone();
    Media::new(vec![track(1, config, samples)], timescale(90_000))
}

fn pat_starts(ts: &[u8]) -> Vec<u64> {
    let mut last_pcr = 0u64;
    let mut out = Vec::new();
    for p in ts.chunks_exact(PKT) {
        if let Some(t) = pcr90(p) {
            last_pcr = t;
        }
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid == PAT_PID && p[1] & 0x40 != 0 {
            out.push(last_pcr);
        }
    }
    out
}

/// The PCR base carried by a packet's adaptation field, if any.
fn pcr90(pkt: &[u8]) -> Option<u64> {
    let af_len = if pkt[3] & 0x20 != 0 {
        usize::from(pkt[4])
    } else {
        return None;
    };
    // flags(1) + PCR(6) content bytes must be present and the PCR flag set.
    if af_len < 7 || pkt[5] & 0x10 == 0 {
        return None;
    }
    let b = &pkt[6..12];
    Some(
        (u64::from(b[0]) << 25)
            | (u64::from(b[1]) << 17)
            | (u64::from(b[2]) << 9)
            | (u64::from(b[3]) << 1)
            | (u64::from(b[4]) >> 7),
    )
}

#[test]
fn psi_interval_getters_round_trip() {
    assert_eq!(TsMux::new().psi_interval(), Some(DEFAULT_PSI_INTERVAL));
    assert_eq!(
        TsMux::new().psi_interval(),
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        TsMux::new()
            .with_psi_interval(Some(Duration::from_millis(250)))
            .psi_interval(),
        Some(Duration::from_millis(250))
    );
    assert_eq!(TsMux::new().with_psi_interval(None).psi_interval(), None);
    assert_eq!(
        StreamingTsMux::new().psi_interval(),
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        StreamingTsMux::with_psi_interval(Some(Duration::from_millis(250))).psi_interval(),
        Some(Duration::from_millis(250))
    );
    assert_eq!(StreamingTsMux::with_psi_interval(None).psi_interval(), None);
}

#[test]
fn one_repeat_per_interval_of_evenly_spaced_samples() {
    let media = one_hz_video();
    let out = TsMux::new()
        .with_psi_interval(Some(Duration::from_millis(100)))
        .package(&media)
        .expect("mux");
    // 100 samples 100 ms apart: the leading PAT plus one repeat per later
    // sample boundary.
    let pats = pat_starts(&out);
    assert_eq!(pats.len(), 100, "one PAT per 100 ms sample boundary");
    let gaps: Vec<u64> = pats.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|&g| g <= 9_000),
        "no PAT gap exceeds the 100 ms interval: {gaps:?}"
    );
    assert_eq!(gaps.iter().max(), Some(&9_000));
}

#[test]
fn a_dense_sample_grid_repeats_on_the_interval() {
    // 250 samples 40 ms apart (10 s at 25 fps): the PCR interval bound is never
    // exceeded by the samples themselves, so no PCR-only packet is inserted and
    // every repeat lands on a sample boundary. A 100 ms interval then repeats
    // every third sample: one leading PAT plus 83 repeats.
    let config = demux(&ts("legacy/mpeg2-mp2.ts"))
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .config()
        .clone();
    let samples: Vec<Sample> = (0..250).map(|k| sample_at(k * 3_600, 3_600)).collect();
    let media = Media::new(vec![track(1, config, samples)], timescale(90_000));
    let out = TsMux::new()
        .with_psi_interval(Some(Duration::from_millis(100)))
        .package(&media)
        .expect("mux");
    let pats = pat_starts(&out);
    assert_eq!(
        pats.len(),
        84,
        "one leading PAT plus a repeat every third 40 ms sample"
    );
    let gaps: Vec<u64> = pats.windows(2).map(|w| w[1] - w[0]).collect();
    assert_eq!(
        gaps.iter().max(),
        Some(&10_800),
        "a repeat lands on the first sample boundary at or after its due time"
    );
    assert!(
        gaps.iter().all(|&g| g <= 10_800),
        "no PAT gap exceeds the interval plus one sample period: {gaps:?}"
    );
    // The PAT/PMT pair is contiguous: a repeat is PAT then PMT, never a PMT
    // without its PAT.
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    for p in out.chunks_exact(PKT) {
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid == PAT_PID || pid == PMT_PID {
            *counts.entry(pid).or_default() += 1;
        }
    }
    assert_eq!(counts.get(&PAT_PID), counts.get(&PMT_PID), "PAT/PMT paired");
    assert_eq!(counts.get(&PAT_PID), Some(&84));
}

#[test]
fn streaming_schedule_carries_across_calls() {
    let media = one_hz_video();
    let samples: Vec<Sample> = media.tracks[0].samples.clone();
    let spec = media.tracks[0].spec.clone();
    let mut mux = StreamingTsMux::with_psi_interval(Some(Duration::from_millis(100)));
    let mut out = Vec::new();
    for s in samples {
        let chunk = Media::new(vec![Track::new(spec.clone(), vec![s])], timescale(90_000));
        out.extend_from_slice(&mux.package(&chunk).expect("mux chunk"));
    }
    // The schedule continues across calls: one PAT per sample boundary, not one
    // per call plus repeats inside a boundary.
    assert_eq!(
        pat_starts(&out).len(),
        100,
        "schedule survives the call boundary"
    );
}

#[test]
fn no_psi_repeat_inside_an_access_unit() {
    // One sample large enough to need several TS packets, with the repetition
    // interval reached at its own key: the repeat must precede its first
    // packet, never fall between two packets of the same PES packet.
    let config = demux(&ts("legacy/mpeg2-mp2.ts"))
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .config()
        .clone();
    let big =
        Sample::new(vec![0x55u8; 1200], Some(0), Some(0), Some(9_000), true).expect("valid timing");
    let next = sample_at(9_000, 9_000);
    let media = Media::new(vec![track(1, config, vec![big, next])], timescale(90_000));
    let out = TsMux::new()
        .with_psi_interval(Some(Duration::from_millis(100)))
        .package(&media)
        .expect("mux");
    let pkts: Vec<&[u8]> = out.chunks_exact(PKT).collect();
    for (i, p) in pkts.iter().enumerate() {
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid != PAT_PID && pid != PMT_PID {
            continue;
        }
        let Some(next_pkt) = pkts.get(i + 1) else {
            continue;
        };
        let next_pid = (u16::from(next_pkt[1] & 0x1F) << 8) | u16::from(next_pkt[2]);
        if next_pid == PAT_PID || next_pid == PMT_PID {
            continue;
        }
        // A PCR-only packet carries no payload; anything else on the ES PID
        // right after a repeat must start an access unit.
        let has_payload = next_pkt[3] & 0x10 != 0;
        assert!(
            !has_payload || next_pkt[1] & 0x40 != 0,
            "packet {i}: PSI repeat landed inside a PES packet"
        );
    }
}

/// Two video tracks take consecutive `stream_id`s of the video family
/// (`1110 xxxx`, 0xE0–0xEF — ISO/IEC 13818-1 Table 2-22).
#[test]
fn a_second_video_stream_takes_the_next_stream_id() {
    let base = demux(&ts("legacy/mpeg2-mp2.ts"));
    let first = base
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .clone();
    let mut second = first.clone();
    second.spec.track_id = TrackId::new(2);
    let media = Media::new(vec![first, second], base.movie_timescale);
    let out = TsMux::new().package(&media).expect("mux");

    let mut stream_ids: Vec<(u16, u8)> = Vec::new();
    for p in out.chunks_exact(PKT) {
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        if pid == PAT_PID || pid == PMT_PID || p[1] & 0x40 == 0 || p[3] & 0x10 == 0 {
            continue;
        }
        let pl = payload(p);
        if pl.len() >= 4 && pl[..3] == [0, 0, 1] {
            stream_ids.push((pid, pl[3]));
        }
    }
    stream_ids.sort_unstable();
    stream_ids.dedup();
    assert_eq!(
        stream_ids,
        vec![(0x0100, 0xE0), (0x0101, 0xE1)],
        "each video ES counts up the 0xE0 stream_id family"
    );
}

/// An AVC keyframe that already begins with an access unit delimiter still
/// gets its SPS/PPS inserted **after** the AUD (ISO/IEC 14496-10 §7.4.1.2.3:
/// the AUD is the first NAL of the access unit).
#[test]
fn avc_parameter_sets_go_after_a_leading_access_unit_delimiter() {
    let media = demux(&ts("h264-aac.ts"));
    let avc = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("AVC track")
        .clone();
    let CodecConfig::Avc { config, .. } = avc.config() else {
        panic!("expected Avc");
    };
    let sps = config.config.sps[0].0.clone();
    let pps = config.config.pps[0].0.clone();

    // A keyframe whose access unit is an AUD followed by one slice, with no
    // parameter sets in-band (the avc1 sample-entry case): length-prefixed
    // [AUD, slice].
    let keyframe = Sample::new(
        vec![
            0x00, 0x00, 0x00, 0x02, 0x09, 0xF0, // AUD (primary_pic_type 7)
            0x00, 0x00, 0x00, 0x03, 0x41, 0x9A, 0x00, // non-IDR slice
        ],
        Some(0),
        Some(0),
        Some(3_000),
        true,
    )
    .expect("valid timing");
    let media = Media::new(
        vec![track(1, avc.config().clone(), vec![keyframe])],
        timescale(90_000),
    );
    let out = TsMux::new()
        .with_psi_interval(None)
        .package(&media)
        .expect("mux");

    // Concatenate the video PID's payloads and strip the PES header.
    let mut pes = Vec::new();
    for p in out.chunks_exact(PKT) {
        if ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) != 0x0100 || p[3] & 0x10 == 0 {
            continue;
        }
        pes.extend_from_slice(payload(p));
    }
    assert!(pes.starts_with(&[0x00, 0x00, 0x01]));
    let header_len = 9 + usize::from(pes[8]);
    let es = &pes[header_len..];

    // The AU order is AUD, SPS, PPS, slice.
    let mut nals: Vec<(usize, u8)> = Vec::new();
    let mut i = 0;
    while i + 4 <= es.len() {
        if es[i..i + 4] == [0, 0, 0, 1] {
            nals.push((i, es[i + 4] & 0x1F));
            i += 4;
        } else {
            i += 1;
        }
    }
    assert_eq!(
        nals.iter().map(|&(_, t)| t).collect::<Vec<_>>(),
        vec![9, 7, 8, 1],
        "AUD first, then the parameter sets, then the slice: {nals:?}"
    );
    let start = nals[1].0 + 4;
    assert_eq!(&es[start..start + sps.len()], &sps[..], "the SPS follows");
    let start = nals[2].0 + 4;
    assert_eq!(&es[start..start + pps.len()], &pps[..], "the PPS follows");
}

// ---------------------------------------------------------------------------
// 4. PCR-only filler on the PCR PID (TR 101 290 indicator 2.3).
// ---------------------------------------------------------------------------

#[test]
fn pcr_only_packets_fill_a_one_second_gap_at_40ms() {
    let config = demux(&ts("legacy/mpeg2-mp2.ts"))
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .config()
        .clone();
    let samples = vec![sample_at(0, 9_000), sample_at(90_000, 9_000)];
    let media = Media::new(vec![track(1, config, samples)], timescale(90_000));
    let out = TsMux::new()
        .with_psi_interval(None)
        .package(&media)
        .expect("mux");
    let fillers: Vec<u64> = out
        .chunks_exact(PKT)
        .filter(|p| {
            let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
            pid == 0x0100 && p[3] & 0x10 == 0
        })
        .filter_map(pcr90)
        .collect();
    assert_eq!(
        fillers.len(),
        24,
        "a 90 000-tick gap needs one filler every 3 600 ticks, exclusive of both ends"
    );
    let gaps: Vec<u64> = fillers.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|&g| g == MAX_PCR_INTERVAL_TICKS),
        "fillers are exactly one interval apart: {gaps:?}"
    );
}

#[test]
fn a_gap_of_exactly_the_maximum_inserts_nothing() {
    let config = demux(&ts("legacy/mpeg2-mp2.ts"))
        .tracks
        .into_iter()
        .find(|t| matches!(t.config(), CodecConfig::Mpeg2Video { .. }))
        .expect("MPEG-2 video track")
        .config()
        .clone();
    let samples = vec![
        sample_at(0, 3_600),
        sample_at(MAX_PCR_INTERVAL_TICKS as i64, 3_600),
    ];
    let media = Media::new(vec![track(1, config, samples)], timescale(90_000));
    let out = TsMux::new()
        .with_psi_interval(None)
        .package(&media)
        .expect("mux");
    let fillers = out
        .chunks_exact(PKT)
        .filter(|p| p[3] & 0x10 == 0 && ((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2])) == 0x0100)
        .count();
    assert_eq!(
        fillers, 0,
        "a gap of exactly one interval is within the bound"
    );
}
