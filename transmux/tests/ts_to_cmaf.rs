//! End-to-end H.264+AAC MPEG-TS → CMAF stream test.
//!
//! Drives the transmux samples-in pipeline from a real MPEG-2 TS fixture:
//! demux → synthesize codec config (avcC from SPS/PPS, esds from AAC ADTS) →
//! build_init_segment + build_media_segment → re-parse and verify every assertion.
//!
//! EXIT CRITERIA (all bite):
//! 1. avcC config fidelity: SPS/PPS bytes + profile/compat/level match the stream.
//! 2. esds config fidelity: OTI=0x40 + stream_type=0x05 + ASC bytes match.
//! 3. Structure: init has 2 traks + mvex/2×trex; media segment has 2 traf;
//!    video trun sample_count == 75; audio trun sample_count == computed ADTS count.
//! 4. Sample round-trip: first video sample re-parsed from mdat matches
//!    annexb_to_length_prefixed(first_video_au).

use std::vec::Vec;

use broadcast_common::{Parse, Serialize};
use mpeg_pes::PesAssembler;
use mpeg_ts::OwnedTsPacket;
use transmux::StblChild;
use transmux::annexb::{annexb_to_length_prefixed, iter_annexb_nals};
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::init_segment::{MovieBox, SampleEntryVariant};
use transmux::movie_fragment::MovieFragmentBox;
use transmux::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};

// ---- Helpers -----------------------------------------------------------------

/// Find a top-level box by type and return its full bytes (header + body).
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        if size < 8 {
            break;
        }
        let ty = &data[offset + 4..offset + 8];
        if ty == fourcc {
            return &data[offset..offset + size];
        }
        offset += size;
    }
    panic!("box {:?} not found", core::str::from_utf8(fourcc).unwrap());
}

/// Convert an SFI raw value to Hz.
fn sfi_to_hz(sfi: u8) -> u32 {
    match sfi {
        0 => 96000,
        1 => 88200,
        2 => 64000,
        3 => 48000,
        4 => 44100,
        5 => 32000,
        6 => 24000,
        7 => 22050,
        8 => 16000,
        9 => 12000,
        10 => 11025,
        11 => 8000,
        12 => 7350,
        _ => 0,
    }
}

/// Split a concatenated ADTS payload into individual frames (7-byte header + raw data).
fn split_adts_frames(payload: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0;
    while off + 7 <= payload.len() {
        if payload[off] != 0xFF || (payload[off + 1] & 0xF0) != 0xF0 {
            break;
        }
        let b3 = payload[off + 3];
        let b4 = payload[off + 4];
        let b5 = payload[off + 5];
        let frame_len = (((b3 & 3) as u16) << 11) | ((b4 as u16) << 3) | ((b5 >> 5) as u16);
        if frame_len < 7 || off + frame_len as usize > payload.len() {
            break;
        }
        frames.push(&payload[off..off + frame_len as usize]);
        off += frame_len as usize;
    }
    frames
}

// ---- Main test --------------------------------------------------------------

#[test]
fn ts_to_cmaf_end_to_end() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    let ts_data = std::fs::read(path).expect("fixture file must exist");
    assert_eq!(
        ts_data.len() % 188,
        0,
        "TS file must be a multiple of 188 bytes"
    );

    // ---- Phase 1: Demux TS → PES -------------------------------------------
    type PesBuf = Vec<u8>;

    let mut vid_assembler = PesAssembler::new();
    let mut aud_assembler = PesAssembler::new();
    let mut vid_pes_bufs: Vec<(PesBuf, u64, Option<u64>)> = Vec::new(); // (buf, pts, dts)
    let mut aud_pes_bufs: Vec<PesBuf> = Vec::new();

    for chunk in ts_data.chunks_exact(188) {
        let raw: [u8; 188] = chunk.try_into().unwrap();
        let pkt = OwnedTsPacket::parse(raw).expect("valid TS packet");
        let payload = match pkt.payload() {
            Some(p) => p,
            None => continue,
        };

        match pkt.pid {
            0x0100 => {
                // Video PID
                if let Some(completed) = vid_assembler.feed(pkt.pusi, payload) {
                    // Parse PTS/DTS from the PES header
                    if let Ok(pes) = mpeg_pes::PesPacket::parse(&completed) {
                        let pts = pes.header.as_ref().and_then(|h| h.pts.map(|p| p.0));
                        let dts = pes.header.as_ref().and_then(|h| h.dts.map(|d| d.0));
                        vid_pes_bufs.push((pes.payload.to_vec(), pts.unwrap_or(0), dts));
                    } else {
                        // If PES parse fails, still push the payload
                        let body = if completed.len() > 6 {
                            &completed[6..]
                        } else {
                            &completed
                        };
                        vid_pes_bufs.push((body.to_vec(), 0, None));
                    }
                }
            }
            0x0101 => {
                // Audio PID
                if let Some(completed) = aud_assembler.feed(pkt.pusi, payload) {
                    if let Ok(pes) = mpeg_pes::PesPacket::parse(&completed) {
                        aud_pes_bufs.push(pes.payload.to_vec());
                    } else {
                        let body = if completed.len() > 6 {
                            &completed[6..]
                        } else {
                            &completed
                        };
                        aud_pes_bufs.push(body.to_vec());
                    }
                }
            }
            _ => {}
        }
    }

    // Flush remaining PES packets
    if let Some(completed) = vid_assembler.flush() {
        if let Ok(pes) = mpeg_pes::PesPacket::parse(&completed) {
            let pts = pes.header.as_ref().and_then(|h| h.pts.map(|p| p.0));
            let dts = pes.header.as_ref().and_then(|h| h.dts.map(|d| d.0));
            vid_pes_bufs.push((pes.payload.to_vec(), pts.unwrap_or(0), dts));
        }
    }
    if let Some(completed) = aud_assembler.flush() {
        if let Ok(pes) = mpeg_pes::PesPacket::parse(&completed) {
            aud_pes_bufs.push(pes.payload.to_vec());
        }
    }

    // Remove empty trailing PES payloads
    vid_pes_bufs.retain(|(b, _, _)| !b.is_empty());
    aud_pes_bufs.retain(|b| !b.is_empty());

    assert_eq!(vid_pes_bufs.len(), 75, "expected 75 video PES/access units");

    // ---- Phase 2: Synthesize avcC config ------------------------------------
    let mut first_sps: Option<Vec<u8>> = None;
    let mut first_pps: Option<Vec<u8>> = None;
    let mut is_idr_flags: Vec<bool> = Vec::with_capacity(vid_pes_bufs.len());

    for (payload, _, _) in &vid_pes_bufs {
        let mut has_idr = false;
        for nal in iter_annexb_nals(payload) {
            let nal_type = nal[0] & 0x1F;
            if nal_type == 7 && first_sps.is_none() {
                first_sps = Some(nal.to_vec());
            }
            if nal_type == 8 && first_pps.is_none() {
                first_pps = Some(nal.to_vec());
            }
            if nal_type == 5 {
                has_idr = true;
            }
        }
        is_idr_flags.push(has_idr);
    }

    // The first payload that contains SPS+PPS is the config AU.
    // But we also captured it directly; ensure we have it.
    let sps = first_sps.expect("no SPS NAL found in video stream");
    let pps = first_pps.expect("no PPS NAL found in video stream");

    assert_eq!(sps[0] & 0x1F, 7, "first byte of SPS must be type 7");
    assert_eq!(pps[0] & 0x1F, 8, "first byte of PPS must be type 8");

    let avc_config = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: sps[1],
        profile_compatibility: sps[2],
        level_indication: sps[3],
        length_size_minus_one: 3,
        sps: vec![transmux::nalu_types::AvcSps(sps.clone())],
        pps: vec![transmux::nalu_types::AvcPps(pps.clone())],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    let config = AVCConfigurationBox::new(avc_config);

    // ---- Phase 3: Synthesize esds config from first audio ADTS header --------
    let first_aud_payload = &aud_pes_bufs[0];
    let adts_header = transmux::aac_asc::parse_adts_header(first_aud_payload)
        .expect("first audio payload should start with ADTS header");
    let asc = transmux::aac_asc::AudioSpecificConfig::from_adts_header(&adts_header);
    let asc_bytes = asc.to_bytes();

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
                data: asc_bytes.clone(),
            }),
        }),
        sl_config: Some(SLConfigDescriptor { body: vec![0x02] }),
    });

    let audio_channel_count = adts_header.channel_configuration as u16;
    let audio_sample_rate = sfi_to_hz(adts_header.sampling_frequency_index);
    let sample_size: u16 = 16;

    // ---- Phase 4: Build video samples ---------------------------------------
    let vid_duration = 3000u32; // 3000 ticks @ 90 kHz ≈ 33 ms
    let video_samples: Vec<Sample> = vid_pes_bufs
        .iter()
        .enumerate()
        .map(|(i, (payload, pts, dts))| {
            let is_sync = is_idr_flags[i];
            let pts_val = *pts;
            let dts_val = dts.unwrap_or(pts_val);
            let composition_offset = (pts_val as i64 - dts_val as i64) as i32;
            Sample::from_annexb(payload, vid_duration, is_sync, composition_offset)
        })
        .collect();

    // ---- Phase 5: Build audio samples ---------------------------------------
    // Split each PES payload into individual ADTS frames, strip the header.
    let mut audio_samples_vec: Vec<Sample> = Vec::new();
    let mut total_adts_frames = 0usize;

    for payload in &aud_pes_bufs {
        let frames = split_adts_frames(payload);
        for frame in frames {
            // Strip the 7-byte ADTS header → raw AAC frame
            if frame.len() > 7 {
                audio_samples_vec.push(Sample {
                    data: frame[7..].to_vec(),
                    duration: 1024,
                    is_sync: true,
                    composition_offset: 0,
                    source_timing: None,
                });
                total_adts_frames += 1;
            }
        }
    }

    assert!(
        total_adts_frames > 14,
        "expected more ADTS frames than PES packets (> 14): got {total_adts_frames}"
    );

    // ---- Phase 6: Build CMAF segments ---------------------------------------
    let video_track = TrackSpec {
        track_id: 1,
        timescale: 90000,
        config: CodecConfig::Avc {
            config: config.clone(),
            width: 0,
            height: 0,
        },
    };
    let audio_track = TrackSpec {
        track_id: 2,
        timescale: audio_sample_rate,
        config: CodecConfig::Aac {
            esds: esds.clone(),
            channel_count: audio_channel_count,
            sample_rate: audio_sample_rate,
            sample_size,
        },
    };
    let tracks = [video_track, audio_track];

    let init_data = build_init_segment(&tracks, 1000).expect("build_init_segment must succeed");

    let fragment_tracks = [
        FragmentTrackData {
            track_id: 1,
            base_media_decode_time: 0,
            samples: &video_samples,
        },
        FragmentTrackData {
            track_id: 2,
            base_media_decode_time: 0,
            samples: &audio_samples_vec,
        },
    ];
    let media_data =
        build_media_segment(1, &fragment_tracks).expect("build_media_segment must succeed");

    // ---- Phase 7: Re-parse and verify init segment (avcC fidelity) -----------
    let init_moov = find_top_box(&init_data, b"moov");
    let moov = MovieBox::parse(init_moov).expect("must parse moov from init segment");

    // Structure: 2 traks + mvex
    assert_eq!(moov.tracks.len(), 2, "init moov must have 2 tracks");
    let mvex = moov.mvex.as_ref().expect("fragmented moov must have mvex");
    assert_eq!(mvex.trex.len(), 2, "mvex must have 2 trex");

    // Find avc1 sample entry in video track
    let vid_trak = &moov.tracks[0];
    let vid_mdia = vid_trak.mdia.as_ref().expect("video trak has mdia");
    let vid_minf = vid_mdia.minf.as_ref().expect("video mdia has minf");
    let vid_stbl = vid_minf.stbl.as_ref().expect("video minf has stbl");
    let vid_stsd = vid_stbl
        .children
        .iter()
        .find_map(|c| {
            if let StblChild::Stsd(s) = c {
                Some(s)
            } else {
                None
            }
        })
        .expect("stbl must contain stsd");

    let vid_entry = vid_stsd
        .entries
        .first()
        .expect("stsd must have at least one entry");
    let (vid_avcc, vid_prof, vid_compat, vid_level) = match vid_entry {
        SampleEntryVariant::Avc1(avc1) => (
            &avc1.config.config,
            avc1.config.config.profile_indication,
            avc1.config.config.profile_compatibility,
            avc1.config.config.level_indication,
        ),
        _ => panic!("video sample entry must be avc1"),
    };

    // avcC config fidelity checks
    assert_eq!(
        vid_avcc.sps[0].0.as_slice(),
        sps.as_slice(),
        "SPS must be byte-identical to the stream SPS"
    );
    assert_eq!(
        vid_avcc.pps[0].0.as_slice(),
        pps.as_slice(),
        "PPS must be byte-identical to the stream PPS"
    );
    assert_eq!(vid_prof, sps[1], "profile_indication must equal sps[1]");
    assert_eq!(
        vid_compat, sps[2],
        "profile_compatibility must equal sps[2]"
    );
    assert_eq!(vid_level, sps[3], "level_indication must equal sps[3]");

    // Find mp4a sample entry in audio track
    let aud_trak = &moov.tracks[1];
    let aud_mdia = aud_trak.mdia.as_ref().expect("audio trak has mdia");
    let aud_minf = aud_mdia.minf.as_ref().expect("audio mdia has minf");
    let aud_stbl = aud_minf.stbl.as_ref().expect("audio minf has stbl");
    let aud_stsd = aud_stbl
        .children
        .iter()
        .find_map(|c| {
            if let StblChild::Stsd(s) = c {
                Some(s)
            } else {
                None
            }
        })
        .expect("stbl must contain stsd");

    let aud_entry = aud_stsd
        .entries
        .first()
        .expect("stsd must have at least one entry");
    let (aud_esds, aud_chan, aud_rate, aud_ssize) = match aud_entry {
        SampleEntryVariant::Mp4a(mp4a) => {
            // esds is in config_boxes as an OpaqueBox('esds', ...), need to re-parse
            let esds_opaque = mp4a
                .config_boxes
                .iter()
                .find(|b| &b.box_type == b"esds")
                .expect("mp4a must contain esds box");
            let esds_full = {
                let size = 8 + esds_opaque.data.len();
                let mut full = vec![0u8; size];
                full[..4].copy_from_slice(&(size as u32).to_be_bytes());
                full[4..8].copy_from_slice(b"esds");
                full[8..].copy_from_slice(&esds_opaque.data);
                full
            };
            let parsed = EsdsBox::parse_box(&esds_full).expect("must parse esds from init segment");
            (parsed, mp4a.channelcount, mp4a.samplerate, mp4a.samplesize)
        }
        _ => panic!("audio sample entry must be mp4a"),
    };

    // esds config fidelity checks
    assert_eq!(aud_esds.es_descriptor.es_id, 1, "esds ES_ID must be 1");
    let dc = aud_esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .expect("esds must have DecoderConfigDescriptor");
    assert_eq!(
        dc.object_type_indication,
        ObjectTypeIndication(0x40),
        "OTI must be 0x40 (MPEG-4 Audio)"
    );
    assert_eq!(
        dc.stream_type,
        StreamType(0x05),
        "stream_type must be 0x05 (AudioStream)"
    );
    let dsi = dc
        .decoder_specific_info
        .as_ref()
        .expect("DecoderConfigDescriptor must have DecoderSpecificInfo");
    assert_eq!(
        dsi.data.as_slice(),
        asc_bytes.as_slice(),
        "DecoderSpecificInfo (ASC) must be byte-identical to the ADTS-derived ASC"
    );

    assert_eq!(
        aud_chan, audio_channel_count,
        "mp4a channelcount matches ADTS"
    );
    assert_eq!(
        aud_rate,
        audio_sample_rate << 16,
        "mp4a samplerate (16.16) matches ADTS"
    );
    assert_eq!(aud_ssize, sample_size, "mp4a samplesize matches expected");

    // ---- Phase 8: Re-parse and verify media segment -------------------------
    let media_moof = find_top_box(&media_data, b"moof");
    let moof =
        MovieFragmentBox::parse_body(&media_moof[8..]).expect("must parse moof from media segment");
    assert_eq!(moof.traf.len(), 2, "media segment moof must have 2 traf");

    // Video trun: sample_count == 75
    let vid_traf = &moof.traf[0];
    assert_eq!(vid_traf.trun.len(), 1, "video traf must have 1 trun");
    assert_eq!(
        vid_traf.trun[0].samples.len(),
        75,
        "video trun must have 75 samples"
    );
    assert_eq!(vid_traf.tfhd.track_id, 1, "video traf tfhd track_id = 1");

    // Audio trun: sample_count == total ADTS frames
    let aud_traf = &moof.traf[1];
    assert_eq!(aud_traf.trun.len(), 1, "audio traf must have 1 trun");
    assert_eq!(
        aud_traf.trun[0].samples.len(),
        total_adts_frames,
        "audio trun sample count must equal total ADTS frame count"
    );
    assert_eq!(aud_traf.tfhd.track_id, 2, "audio traf tfhd track_id = 2");

    // ---- Phase 9: First video sample round-trip -----------------------------
    // Find mdat
    let mdat_box = find_top_box(&media_data, b"mdat");
    let _mdat_data = &mdat_box[8..]; // skip box header

    // Compute mdat offsets: same order as build_media_segment — video (track 1) first, then audio
    let _vid_moof_sz =
        (u32::from_be_bytes([media_moof[0], media_moof[1], media_moof[2], media_moof[3]])) as usize;
    // In media_data, layout is styp + moof + mdat. We found moof and mdat by type.
    let mdat_body = &mdat_box[8..];

    // First video sample data is at the start of mdat body
    let first_vid_sample_size = vid_traf.trun[0].samples[0]
        .sample_size
        .expect("first video sample must have size");
    let first_vid_sample = &mdat_body[..first_vid_sample_size as usize];

    let expected_first_video = annexb_to_length_prefixed(&vid_pes_bufs[0].0);
    assert_eq!(
        first_vid_sample,
        expected_first_video.as_slice(),
        "first video sample from mdat must match annexb_to_length_prefixed(first_video_au)"
    );
}
