//! Gate tests for the stateful CMAF [`Segmenter`].
//!
//! Two layers:
//!  1. A **synthetic** test with fully controlled sample durations + keyframe
//!     positions pins the exact segmentation arithmetic — segment count, per-track
//!     `tfdt` contiguity, keyframe-aligned segment starts, sample preservation, and
//!     byte-identity of a single-segment run against [`build_media_segment`]. These
//!     are properties a broken cut/accounting cannot satisfy.
//!  2. A **real-fixture** test remuxes `fixtures/ts/h264_aac.ts` through the
//!     segmenter and checks every emitted segment parses and preserves the stream.

use broadcast_common::Serialize;
use mpeg_pes::PesAssembler;
use mpeg_ts::OwnedTsPacket;
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, CodecConfig, DecoderConfigDescriptor,
    DecoderSpecificInfo, ESDescriptor, EsdsBox, FragmentTrackData, MovieFragmentBox,
    ObjectTypeIndication, SLConfigDescriptor, Sample, Segmenter, StreamType, TrackSpec,
    build_media_segment,
};

/// Wire value of a sync (random-access) `sample_flags`, per ISO/IEC 14496-12
/// §8.8.3.1 (`sample_is_non_sync_sample == 0`, `sample_depends_on == 2`).
const WIRE_SYNC_FLAGS: u32 = 0x0200_0000;

/// A minimal but structurally-real `avcC` so `build_init_segment` succeeds.
fn dummy_avc_config() -> AVCConfigurationBox {
    AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    })
}

fn dummy_esds() -> EsdsBox {
    EsdsBox::new(ESDescriptor {
        es_id: 1,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(0x40),
            stream_type: StreamType(0x05),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo {
                data: vec![0x12, 0x10],
            }),
        }),
        sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
    })
}

fn video_track() -> TrackSpec {
    TrackSpec {
        track_id: 1,
        timescale: 90_000,
        config: CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    }
}

fn audio_track() -> TrackSpec {
    TrackSpec {
        track_id: 2,
        timescale: 48_000,
        config: CodecConfig::Aac {
            esds: dummy_esds(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    }
}

/// Find the first top-level box of `fourcc` in a serialized segment and return
/// its **body** (bytes after the 8-byte box header).
fn find_box_body<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size = u32::from_be_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        let ty = &data[off + 4..off + 8];
        if size < 8 || off + size > data.len() {
            return None;
        }
        if ty == fourcc {
            return Some(&data[off + 8..off + size]);
        }
        off += size;
    }
    None
}

/// Parse the `moof` of a media segment and return, per traf, the tuple
/// `(track_id, base_media_decode_time, sample_count, first_sample_flags)`.
/// `first_sample_flags` resolves `first_sample_flags` when present, else the
/// first per-sample `sample_flags`.
fn moof_summary(segment: &[u8]) -> Vec<(u32, u64, usize, u32)> {
    let moof = find_box_body(segment, b"moof").expect("segment has moof");
    // find_box_body gave us the moof *body*; MovieFragmentBox::parse_body wants that.
    let mf = MovieFragmentBox::parse_body(moof).expect("moof parses");
    mf.traf
        .iter()
        .map(|traf| {
            let track_id = traf.tfhd.track_id;
            let base = traf
                .tfdt
                .as_ref()
                .map(|b| b.base_media_decode_time())
                .unwrap_or(0);
            let count: usize = traf.trun.iter().map(|r| r.samples.len()).sum();
            let first_flags = traf
                .trun
                .first()
                .map(|r| {
                    r.first_sample_flags
                        .or_else(|| r.samples.first().and_then(|s| s.sample_flags))
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            (track_id, base, count, first_flags)
        })
        .collect()
}

/// Interleave video (90 kHz) and audio (48 kHz) samples in decode order and push
/// them through a segmenter with a 0.3 s target. Video keyframes every 10 AUs.
#[test]
fn synthetic_cuts_are_exact_and_lossless() {
    let mut seg = Segmenter::new(vec![video_track(), audio_track()], 1000, 0.3).unwrap();

    // 30 video AUs @ 3000 ticks (=1/30 s), IDR every 10th → keyframes at 0,10,20.
    const N_VID: usize = 30;
    const VID_DUR: u32 = 3000; // 90 kHz
    const N_AUD: usize = 48;
    const AUD_DUR: u32 = 1024; // 48 kHz

    // Build a merged (decode-time, is_video, index) order.
    let mut items: Vec<(f64, bool, usize)> = Vec::new();
    for i in 0..N_VID {
        items.push((i as f64 * VID_DUR as f64 / 90_000.0, true, i));
    }
    for j in 0..N_AUD {
        items.push((j as f64 * AUD_DUR as f64 / 48_000.0, false, j));
    }
    // Stable sort by time; ties keep video before audio (insertion order above).
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    for (_, is_video, idx) in &items {
        if *is_video {
            let is_sync = idx % 10 == 0;
            seg.push(
                1,
                Sample {
                    data: vec![0xAA; 16],
                    duration: VID_DUR,
                    is_sync,
                    composition_offset: 0,
                },
            )
            .unwrap();
        } else {
            seg.push(2, Sample::from_raw(vec![0xBB; 8], AUD_DUR))
                .unwrap();
        }
    }
    seg.flush().unwrap();
    let segments = seg.take_ready();

    // --- exact segment count: keyframes at 0/10/20, target 0.3s (=27000 ticks).
    // cut fires at AU10 (buffered 30000≥27000) and AU20 → 2 mid cuts + 1 flush = 3.
    assert_eq!(segments.len(), 3, "expected exactly 3 segments");

    // --- per-track invariants across the segments.
    let mut vid_seen = 0usize;
    let mut aud_seen = 0usize;
    let mut vid_next_tfdt = 0u64;
    let mut aud_next_tfdt = 0u64;
    for (si, s) in segments.iter().enumerate() {
        // Every segment must carry an styp + a parseable moof + an mdat.
        assert!(find_box_body(s, b"styp").is_some(), "seg {si} has styp");
        assert!(find_box_body(s, b"mdat").is_some(), "seg {si} has mdat");
        for (track_id, base, count, first_flags) in moof_summary(s) {
            match track_id {
                1 => {
                    // Video segment must START on a keyframe (sync flags on sample 0).
                    assert_eq!(
                        first_flags, WIRE_SYNC_FLAGS,
                        "seg {si} video must start on a sync sample"
                    );
                    // tfdt is contiguous: equals total video duration emitted so far.
                    assert_eq!(base, vid_next_tfdt, "seg {si} video tfdt contiguity");
                    vid_next_tfdt += count as u64 * VID_DUR as u64;
                    vid_seen += count;
                }
                2 => {
                    assert_eq!(base, aud_next_tfdt, "seg {si} audio tfdt contiguity");
                    aud_next_tfdt += count as u64 * AUD_DUR as u64;
                    aud_seen += count;
                }
                other => panic!("unexpected track_id {other}"),
            }
        }
    }
    // No sample dropped or duplicated.
    assert_eq!(vid_seen, N_VID, "all video samples preserved");
    assert_eq!(aud_seen, N_AUD, "all audio samples preserved");
    // Each 0.3s segment holds ~10 video AUs (keyframe cadence) → 10/10/10.
    assert_eq!(vid_next_tfdt, N_VID as u64 * VID_DUR as u64);
}

/// A segmenter with a target larger than the whole stream never cuts mid-stream,
/// so `flush` emits exactly one segment — and that segment must be byte-identical
/// to a direct `build_media_segment(1, [video_all, audio_all])`. Any reordering,
/// duplication, or mangling in the segmenter breaks this equality.
#[test]
fn single_segment_is_byte_identical_to_batch_builder() {
    let vid: Vec<Sample> = (0..12)
        .map(|i| Sample {
            data: vec![i as u8; 10],
            duration: 3000,
            is_sync: i % 6 == 0,
            composition_offset: 0,
        })
        .collect();
    let aud: Vec<Sample> = (0..20)
        .map(|_| Sample::from_raw(vec![0x5A; 6], 1024))
        .collect();

    // Batch reference: one media segment, video traf first then audio.
    let reference = build_media_segment(
        1,
        &[
            FragmentTrackData {
                track_id: 1,
                base_media_decode_time: 0,
                samples: &vid,
            },
            FragmentTrackData {
                track_id: 2,
                base_media_decode_time: 0,
                samples: &aud,
            },
        ],
    )
    .unwrap();

    // Segmenter with a 1000 s target → no mid cut. Push interleaved; flush once.
    let mut seg = Segmenter::new(vec![video_track(), audio_track()], 1000, 1000.0).unwrap();
    for v in &vid {
        seg.push(1, v.clone()).unwrap();
    }
    for a in &aud {
        seg.push(2, a.clone()).unwrap();
    }
    seg.flush().unwrap();
    let segments = seg.take_ready();
    assert_eq!(segments.len(), 1, "huge target → single segment");
    assert_eq!(
        segments[0], reference,
        "single-segment output must equal the batch builder byte-for-byte"
    );
}

#[test]
fn rejects_bad_construction() {
    assert!(Segmenter::new(vec![], 1000, 2.0).is_err(), "empty tracks");
    assert!(
        Segmenter::new(vec![video_track()], 1000, 0.0).is_err(),
        "zero duration"
    );
    assert!(
        Segmenter::new(vec![video_track()], 1000, f64::NAN).is_err(),
        "NaN duration"
    );
    assert!(
        Segmenter::new(vec![video_track(), video_track()], 1000, 2.0).is_err(),
        "duplicate track_id"
    );
    let mut seg = Segmenter::new(vec![video_track()], 1000, 2.0).unwrap();
    assert!(
        seg.push(99, Sample::from_raw(vec![0], 1)).is_err(),
        "unknown track_id"
    );
}

// ---------------------------------------------------------------------------
// Real-fixture remux: h264_aac.ts → segmenter → structural validity.
// ---------------------------------------------------------------------------

fn sfi_to_hz(sfi: u8) -> u32 {
    const TABLE: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    TABLE.get(sfi as usize).copied().unwrap_or(48000)
}

/// Split concatenated ADTS frames in a PES payload into per-frame slices.
fn split_adts_frames(payload: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 7 <= payload.len() {
        if payload[off] != 0xFF || (payload[off + 1] & 0xF0) != 0xF0 {
            break;
        }
        let len = (((payload[off + 3] as usize & 0x03) << 11)
            | ((payload[off + 4] as usize) << 3)
            | ((payload[off + 5] as usize & 0xE0) >> 5))
            & 0x1FFF;
        if len < 7 || off + len > payload.len() {
            break;
        }
        out.push(&payload[off..off + len]);
        off += len;
    }
    out
}

#[test]
fn real_fixture_remux_is_lossless_and_parseable() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let ts = std::fs::read(path).expect("fixture exists");

    let mut vid_asm = PesAssembler::new();
    let mut aud_asm = PesAssembler::new();
    let mut vid: Vec<(Vec<u8>, u64, Option<u64>)> = Vec::new();
    let mut aud: Vec<Vec<u8>> = Vec::new();

    let ingest_vid = |completed: &[u8], vid: &mut Vec<(Vec<u8>, u64, Option<u64>)>| {
        if let Ok(pes) = mpeg_pes::PesPacket::parse(completed) {
            let pts = pes.header.as_ref().and_then(|h| h.pts.map(|p| p.0));
            let dts = pes.header.as_ref().and_then(|h| h.dts.map(|d| d.0));
            if !pes.payload.is_empty() {
                vid.push((pes.payload.to_vec(), pts.unwrap_or(0), dts));
            }
        }
    };

    for chunk in ts.chunks_exact(188) {
        let pkt = OwnedTsPacket::parse(chunk.try_into().unwrap()).expect("valid TS packet");
        let Some(payload) = pkt.payload() else {
            continue;
        };
        match pkt.pid {
            0x0100 => {
                if let Some(c) = vid_asm.feed(pkt.pusi, payload) {
                    ingest_vid(&c, &mut vid);
                }
            }
            0x0101 => {
                if let Some(c) = aud_asm.feed(pkt.pusi, payload) {
                    if let Ok(pes) = mpeg_pes::PesPacket::parse(&c) {
                        if !pes.payload.is_empty() {
                            aud.push(pes.payload.to_vec());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(c) = vid_asm.flush() {
        ingest_vid(&c, &mut vid);
    }
    if let Some(c) = aud_asm.flush() {
        if let Ok(pes) = mpeg_pes::PesPacket::parse(&c) {
            if !pes.payload.is_empty() {
                aud.push(pes.payload.to_vec());
            }
        }
    }
    assert!(!vid.is_empty() && !aud.is_empty(), "demux produced samples");

    // Synthesize avcC from the first SPS/PPS + IDR flags.
    let mut sps = None;
    let mut pps = None;
    let mut idr = Vec::with_capacity(vid.len());
    for (payload, _, _) in &vid {
        let mut has_idr = false;
        for nal in transmux::iter_annexb_nals(payload) {
            match nal[0] & 0x1F {
                7 if sps.is_none() => sps = Some(nal.to_vec()),
                8 if pps.is_none() => pps = Some(nal.to_vec()),
                5 => has_idr = true,
                _ => {}
            }
        }
        idr.push(has_idr);
    }
    let sps = sps.expect("SPS");
    let pps = pps.expect("PPS");
    let config = AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: sps[1],
        profile_compatibility: sps[2],
        level_indication: sps[3],
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(sps.clone())],
        pps: vec![transmux::AvcPps(pps.clone())],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    });

    let adts = transmux::parse_adts_header(&aud[0]).expect("ADTS header");
    let asc = transmux::AudioSpecificConfig::from_adts_header(&adts);
    let esds = EsdsBox::new(ESDescriptor {
        es_id: 1,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(0x40),
            stream_type: StreamType(0x05),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo {
                data: asc.to_bytes(),
            }),
        }),
        sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
    });
    let audio_rate = sfi_to_hz(adts.sampling_frequency_index);

    let vtrack = TrackSpec {
        track_id: 1,
        timescale: 90_000,
        config: CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        },
    };
    let atrack = TrackSpec {
        track_id: 2,
        timescale: audio_rate,
        config: CodecConfig::Aac {
            esds,
            channel_count: adts.channel_configuration as u16,
            sample_rate: audio_rate,
            sample_size: 16,
        },
    };

    // Count expected samples for the loss check.
    let expect_vid = vid.len();
    let mut expect_aud = 0usize;
    for p in &aud {
        for f in split_adts_frames(p) {
            if f.len() > 7 {
                expect_aud += 1;
            }
        }
    }

    let mut seg = Segmenter::new(vec![vtrack, atrack], 1000, 0.5).unwrap();
    let init = seg.init_segment().expect("init");
    assert!(find_box_body(&init, b"ftyp").is_some() && find_box_body(&init, b"moov").is_some());

    for (i, (payload, pts, dts)) in vid.iter().enumerate() {
        let cts = (*pts as i64 - (*dts).unwrap_or(*pts) as i64) as i32;
        seg.push(1, Sample::from_annexb(payload, 3000, idr[i], cts))
            .unwrap();
    }
    for p in &aud {
        for f in split_adts_frames(p) {
            if f.len() > 7 {
                seg.push(2, Sample::from_raw(f[7..].to_vec(), 1024))
                    .unwrap();
            }
        }
    }
    seg.flush().unwrap();
    let segments = seg.take_ready();
    assert!(!segments.is_empty(), "produced segments");

    let mut vid_seen = 0usize;
    let mut aud_seen = 0usize;
    let mut vid_tfdt = 0u64;
    for s in &segments {
        assert!(find_box_body(s, b"styp").is_some());
        assert!(find_box_body(s, b"mdat").is_some());
        for (tid, base, count, first_flags) in moof_summary(s) {
            match tid {
                1 => {
                    assert_eq!(first_flags, WIRE_SYNC_FLAGS, "video seg starts on keyframe");
                    assert_eq!(base, vid_tfdt, "video tfdt contiguity");
                    vid_tfdt += count as u64 * 3000;
                    vid_seen += count;
                }
                2 => aud_seen += count,
                other => panic!("track {other}"),
            }
        }
    }
    assert_eq!(vid_seen, expect_vid, "no video sample lost");
    assert_eq!(aud_seen, expect_aud, "no audio sample lost");
}
