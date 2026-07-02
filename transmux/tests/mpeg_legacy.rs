//! MPEG-2 video (H.262) + MPEG-1/2 audio (MP1/2/3) codec support gate.
//!
//! Exercises the two legacy-codec [`CodecConfig`] variants end to end:
//! fMP4 demux (mp4v/esds + mp4a/esds), TS demux (stream_type 0x02/0x03),
//! config recovery from the in-band `sequence_header()` / audio frame header,
//! and byte-exact sample + esds round-trips. Every assertion is derived from
//! the fixtures / the ffprobe CSV oracle — no hardcoded offsets.

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::init_segment::{MovieBox, SampleEntryVariant, StblChild};
use transmux::media::{CmafMux, Fmp4Demux};
use transmux::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer};
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::TsDemux;

fn fixture(rel: &[&str]) -> Vec<u8> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("fixtures");
    for p in rel {
        path.push(p);
    }
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

// ── Helpers: pull the raw esds box body straight out of the source mp4 ──────

/// Walk the moov and return the body bytes of the first `esds` box under the
/// first track's stsd sample entry (the box body after its 8-byte header).
fn source_esds_body(mp4: &[u8]) -> Vec<u8> {
    // Find moov, then scan for the "esds" FourCC and slice by its size prefix.
    // (The mp4 has exactly one esds; this is independent of any crate parser.)
    let pos = find_fourcc(mp4, b"esds").expect("source esds box");
    let size =
        u32::from_be_bytes([mp4[pos - 4], mp4[pos - 3], mp4[pos - 2], mp4[pos - 1]]) as usize;
    // box starts 4 bytes before the FourCC (the size field); body is after hdr.
    let box_start = pos - 4;
    mp4[box_start + 8..box_start + size].to_vec()
}

fn find_fourcc(data: &[u8], fourcc: &[u8; 4]) -> Option<usize> {
    data.windows(4).position(|w| w == fourcc)
}

// ── Test 1: fMP4 MPEG-2 video ───────────────────────────────────────────────

#[test]
fn fmp4_mpeg2_video_config_and_dims() {
    let mp4 = fixture(&["mp4", "frag", "mpeg2video.frag.mp4"]);
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux mpeg2 video mp4");

    assert_eq!(media.tracks.len(), 1, "one video track");
    let track = &media.tracks[0];

    let (esds, width, height) = match track.config() {
        CodecConfig::Mpeg2Video {
            esds,
            width,
            height,
        } => (esds, *width, *height),
        other => panic!("expected Mpeg2Video, got {other:?}"),
    };

    // The reconstructed esds serializes byte-identically to the source box body.
    let mut buf = vec![0u8; esds.serialized_len()];
    let n = esds.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[8..n],
        &source_esds_body(&mp4)[..],
        "esds body byte-identical"
    );

    // OTI 0x61 (MPEG-2 Main Visual).
    let oti = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .unwrap()
        .object_type_indication
        .0;
    assert_eq!(oti, 0x61, "esds OTI == MPEG-2 Main Visual");

    // Dimensions must equal the value parsed from the in-band sequence header.
    let first = &track.samples[0].data;
    let sh = Mpeg2SeqHeader::find(first).expect("in-band sequence_header in first sample");
    assert_eq!((sh.width, sh.height), (320, 240), "seq header is 320x240");
    assert_eq!(
        (width, height),
        (sh.width, sh.height),
        "config dims == seq header"
    );
}

// ── Test 2: fMP4 MPEG audio ─────────────────────────────────────────────────

#[test]
fn fmp4_mpeg_audio_config() {
    let mp4 = fixture(&["mp4", "frag", "mp3.frag.mp4"]);
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux mp3 mp4");

    assert_eq!(media.tracks.len(), 1, "one audio track");
    let track = &media.tracks[0];

    let (esds, layer, sample_rate, channels) = match track.config() {
        CodecConfig::MpegAudio {
            esds,
            layer,
            sample_rate,
            channel_count,
            ..
        } => (esds, *layer, *sample_rate, *channel_count),
        other => panic!("expected MpegAudio, got {other:?}"),
    };

    let oti = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .unwrap()
        .object_type_indication
        .0;
    assert_eq!(oti, 0x6B, "esds OTI == MPEG-1 Audio");

    // layer/rate/channels must match the parsed first frame header.
    let hdr = MpegAudioFrameHeader::parse(&track.samples[0].data)
        .expect("MPEG audio frame header in first sample");
    assert_eq!(layer, MpegAudioLayer::LayerIII, "MP3 = Layer III");
    assert_eq!(layer, hdr.layer);
    assert_eq!(sample_rate, hdr.sample_rate);
    assert_eq!(channels, hdr.channels);
    assert_eq!(hdr.sample_rate, 44100);
}

// ── Test 3: fMP4 sample fidelity (demux → IR → CmafMux → re-demux) ──────────

fn assert_sample_roundtrip(rel: &[&str]) {
    let mp4 = fixture(rel);
    let mut d1 = Fmp4Demux::new();
    let media = d1.unpackage(&mp4).expect("demux");

    let mut mux = CmafMux::default();
    let remuxed = mux.package(&media).expect("re-mux");

    let mut d2 = Fmp4Demux::new();
    let media2 = d2.unpackage(&remuxed).expect("re-demux");

    assert_eq!(
        media.tracks.len(),
        media2.tracks.len(),
        "track count preserved"
    );
    for (a, b) in media.tracks.iter().zip(&media2.tracks) {
        assert_eq!(a.samples.len(), b.samples.len(), "sample count preserved");
        for (sa, sb) in a.samples.iter().zip(&b.samples) {
            assert_eq!(sa.data, sb.data, "coded sample bytes byte-identical");
        }
    }
}

#[test]
fn fmp4_sample_fidelity_mpeg2_video() {
    assert_sample_roundtrip(&["mp4", "frag", "mpeg2video.frag.mp4"]);
}

#[test]
fn fmp4_sample_fidelity_mpeg_audio() {
    assert_sample_roundtrip(&["mp4", "frag", "mp3.frag.mp4"]);
}

// ── Test 4/5: TS demux — the broadcast pair + config from real ES ───────────

struct CsvRow {
    kind: String,
    pts: i128,
    dts: i128,
    duration: u32,
    keyframe: bool,
}

fn load_csv() -> Vec<CsvRow> {
    let raw = fixture(&["ts", "legacy", "mpeg2_mp2.packets.csv"]);
    let text = String::from_utf8(raw).unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            CsvRow {
                kind: f[0].to_string(),
                pts: f[2].parse().unwrap(),
                dts: f[3].parse().unwrap(),
                duration: f[4].parse().unwrap(),
                keyframe: f[6].trim() == "1",
            }
        })
        .collect()
}

#[test]
fn ts_demux_broadcast_pair() {
    let ts = fixture(&["ts", "legacy", "mpeg2_mp2.ts"]);
    let mut demux = TsDemux::new();
    let media = demux.demux(&ts).expect("demux ts");

    assert_eq!(media.tracks.len(), 2, "exactly 2 tracks");

    // Track order follows PMT order: video (0x02) then audio (0x03).
    let video = &media.tracks[0];
    let audio = &media.tracks[1];

    let vwidth = match video.config() {
        CodecConfig::Mpeg2Video { width, height, .. } => {
            assert_eq!((*width, *height), (320, 240), "TS video 320x240");
            *width
        }
        other => panic!("track 0 expected Mpeg2Video, got {other:?}"),
    };
    let _ = vwidth;

    match audio.config() {
        CodecConfig::MpegAudio {
            layer,
            sample_rate,
            channel_count,
            ..
        } => {
            assert_eq!(*layer, MpegAudioLayer::LayerII, "MP2 = Layer II");
            assert_eq!(*sample_rate, 44100);
            assert_eq!(*channel_count, 1, "mono");
        }
        other => panic!("track 1 expected MpegAudio, got {other:?}"),
    }

    let csv = load_csv();
    let csv_video: Vec<&CsvRow> = csv.iter().filter(|r| r.kind == "video").collect();
    let csv_audio: Vec<&CsvRow> = csv.iter().filter(|r| r.kind == "audio").collect();

    assert_eq!(video.samples.len(), 25, "25 video samples");
    assert_eq!(audio.samples.len(), 39, "39 audio samples");
    assert_eq!(csv_video.len(), 25);
    assert_eq!(csv_audio.len(), 39);

    // Video timing: samples are decode-ordered (ascending DTS) — the CSV video
    // rows are in the same order. Reconstruct absolute DTS from durations
    // starting at the first CSV DTS, and PTS = DTS + composition_offset.
    let dts0 = csv_video[0].dts;
    let mut dts = dts0;
    for (i, s) in video.samples.iter().enumerate() {
        let row = csv_video[i];
        assert_eq!(s.duration, row.duration, "video[{i}] duration matches CSV");
        assert_eq!(dts, row.dts, "video[{i}] DTS matches CSV");
        let pts = dts + s.composition_offset as i128;
        assert_eq!(pts, row.pts, "video[{i}] PTS matches CSV");
        assert_eq!(s.is_sync, row.keyframe, "video[{i}] keyframe matches CSV");
        dts += s.duration as i128;
    }

    // Audio is contiguous: PTS == DTS, uniform duration. The audio track uses a
    // media timescale of the sampling rate (44100), so the CSV 90 kHz duration
    // maps to sample_rate ticks: 2351 @ 90 kHz == 1152 @ 44100 (one MP2 frame).
    let audio_ts = audio.timescale();
    assert_eq!(audio_ts, 44100, "audio media timescale = sampling rate");
    let samples_per_frame = MpegAudioFrameHeader::parse(&audio.samples[0].data)
        .unwrap()
        .samples_per_frame;
    assert_eq!(samples_per_frame, 1152, "MP2 = 1152 samples/frame");
    for (i, s) in audio.samples.iter().enumerate() {
        // duration is one MP2 frame = samples_per_frame ticks at the audio
        // timescale; converting to 90 kHz matches the CSV within a rounding tick.
        assert_eq!(
            s.duration, samples_per_frame,
            "audio[{i}] duration = 1 MP2 frame"
        );
        let dur_90k = s.duration as i128 * 90_000 / audio_ts as i128;
        assert!(
            (dur_90k - csv_audio[i].duration as i128).abs() <= 1,
            "audio[{i}] duration matches CSV @90kHz within rounding"
        );
        assert_eq!(s.composition_offset, 0, "audio has no CTS offset");
    }
    // Sanity: contiguous audio PTS grid (in 90 kHz) derived from the durations
    // spans the same range as the CSV (guards against dropped/extra frames).
    let apts0 = csv_audio[0].pts;
    let total_90k: i128 = audio.samples[..audio.samples.len() - 1]
        .iter()
        .map(|s| s.duration as i128 * 90_000 / audio_ts as i128)
        .sum();
    let span_last = apts0 + total_90k;
    let csv_last = csv_audio[csv_audio.len() - 1].pts;
    assert!(
        (span_last - csv_last).abs() <= audio.samples.len() as i128,
        "audio PTS grid spans the CSV range (within accumulated rounding)"
    );
}

#[test]
fn ts_config_from_real_es() {
    let ts = fixture(&["ts", "legacy", "mpeg2_mp2.ts"]);
    let mut demux = TsDemux::new();
    let media = demux.demux(&ts).expect("demux ts");
    let video = &media.tracks[0];
    let audio = &media.tracks[1];

    // Video config geometry recovered from the in-band sequence_header().
    let sh = Mpeg2SeqHeader::find(&video.samples[0].data).expect("in-band seq header");
    assert_eq!((sh.width, sh.height), (320, 240));

    // Audio config from the first MP2 frame header.
    let hdr = MpegAudioFrameHeader::parse(&audio.samples[0].data).expect("mp2 frame header");
    assert_eq!(hdr.layer, MpegAudioLayer::LayerII);
    assert_eq!(hdr.sample_rate, 44100);
    assert_eq!(hdr.channels, 1);

    // The MPEG-2 video sample entry must round-trip through mp4v (build init +
    // re-parse) with an esds carrying OTI 0x61.
    let mut mux = CmafMux::default();
    let remuxed = mux.package(&media).expect("mux ts->cmaf");
    let moov_bytes = find_top_box(&remuxed, b"moov").expect("moov");
    let moov = MovieBox::parse(moov_bytes).expect("parse moov");
    let entry = stsd_entry(&moov, 0);
    match entry {
        SampleEntryVariant::Mp4v(m) => {
            assert!(
                m.config_boxes.iter().any(|b| &b.box_type == b"esds"),
                "mp4v carries an esds"
            );
        }
        other => panic!("expected Mp4v sample entry, got {other:?}"),
    }
}

fn stsd_entry(moov: &MovieBox, track_idx: usize) -> &SampleEntryVariant {
    let stbl = moov.tracks[track_idx]
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .unwrap()
        .stbl
        .as_ref()
        .unwrap();
    stbl.children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(&s.entries[0]),
            _ => None,
        })
        .unwrap()
}

fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let sz =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if sz < 8 || off + sz > data.len() {
            return None;
        }
        if &data[off + 4..off + 8] == fourcc {
            return Some(&data[off..off + sz]);
        }
        off += sz;
    }
    None
}
