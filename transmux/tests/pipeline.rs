//! Real-data gate for the TS→CMAF remux pipeline.
//!
//! Drives [`transmux::build_init_segment`] / [`transmux::build_media_segment`]
//! with the **real** codec configs (avcC/esds) and **real** coded samples taken
//! from `h264_aac_frag.mp4`, then re-parses the produced segments with the
//! crate's own parsers and asserts structure, `trun.data_offset`, `tfdt`, and
//! byte-exact sample survival. A raw-passthrough or wrong-offset implementation
//! cannot pass this.

use broadcast_common::Parse;
use transmux::{
    build_init_segment, build_media_segment, CodecConfig, EsdsBox, FragmentTrackData, MediaDataBox,
    MovieBox, MovieFragmentBox, Sample, SampleEntryVariant, StblChild, TrackSpec,
};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    std::fs::read(path).expect("fixture file must exist")
}

/// Find a top-level box, returning (absolute offset, full bytes).
fn find_top_box(data: &[u8], fourcc: &[u8; 4]) -> (usize, Vec<u8>) {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        if &data[off + 4..off + 8] == fourcc {
            return (off, data[off..off + size].to_vec());
        }
        off += size;
    }
    panic!("box {:?} not found", std::str::from_utf8(fourcc).unwrap());
}

/// Pull the stsd sample entry for a track (video track index 0, audio 1).
fn track_stsd_entry(moov: &MovieBox, track_idx: usize) -> SampleEntryVariant {
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
    let stsd = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(s),
            _ => None,
        })
        .unwrap();
    stsd.entries[0].clone()
}

fn track_timescale(moov: &MovieBox, track_idx: usize) -> u32 {
    moov.tracks[track_idx]
        .mdia
        .as_ref()
        .unwrap()
        .mdhd
        .as_ref()
        .unwrap()
        .timescale
}

/// Build the two [`TrackSpec`]s from the fixture's real avcC + esds.
fn track_specs(moov: &MovieBox) -> (Vec<TrackSpec>, EsdsBox) {
    // Video (track 0): avc1 → avcC.
    let avc = match track_stsd_entry(moov, 0) {
        SampleEntryVariant::Avc1(a) => a,
        other => panic!("expected avc1, got {:?}", std::mem::discriminant(&other)),
    };
    // Audio (track 1): mp4a → esds (reconstruct full box from the opaque body).
    let mp4a = match track_stsd_entry(moov, 1) {
        SampleEntryVariant::Mp4a(m) => m,
        other => panic!("expected mp4a, got {:?}", std::mem::discriminant(&other)),
    };
    let esds_ob = mp4a
        .config_boxes
        .iter()
        .find(|b| &b.box_type == b"esds")
        .expect("mp4a must carry esds");
    // Reassemble the full esds box (8-byte header + body) for parse_box.
    let mut esds_full = Vec::with_capacity(8 + esds_ob.data.len());
    esds_full.extend_from_slice(&((8 + esds_ob.data.len()) as u32).to_be_bytes());
    esds_full.extend_from_slice(b"esds");
    esds_full.extend_from_slice(&esds_ob.data);
    let esds = EsdsBox::parse_box(&esds_full).expect("esds must parse");

    let specs = vec![
        TrackSpec {
            track_id: 1,
            timescale: track_timescale(moov, 0),
            config: CodecConfig::Avc {
                config: avc.config.clone(),
                width: avc.visual.width,
                height: avc.visual.height,
            },
        },
        TrackSpec {
            track_id: 2,
            timescale: track_timescale(moov, 1),
            config: CodecConfig::Aac {
                esds: esds.clone(),
                channel_count: mp4a.channelcount,
                // sample entry stores 16.16; recover the integer Hz.
                sample_rate: mp4a.samplerate >> 16,
                sample_size: mp4a.samplesize,
            },
        },
    ];
    (specs, esds)
}

/// Extract one track's real coded samples from the first moof+mdat.
/// Returns (track_id, samples). Uses default-base-is-moof: the trun data_offset
/// is measured from the moof start.
fn extract_samples(
    data: &[u8],
    moof_off: usize,
    moof: &MovieFragmentBox,
    traf_idx: usize,
) -> (u32, Vec<Sample>) {
    let traf = &moof.traf[traf_idx];
    let track_id = traf.tfhd.track_id;
    let trun = &traf.trun[0];
    let base = moof_off + trun.data_offset.expect("data_offset present") as usize;
    let mut samples = Vec::new();
    let mut cursor = base;
    for (i, ts) in trun.samples.iter().enumerate() {
        let size = ts.sample_size.expect("sample_size present") as usize;
        let data_bytes = data[cursor..cursor + size].to_vec();
        cursor += size;
        samples.push(Sample {
            data: data_bytes,
            duration: ts.sample_duration.unwrap_or(3000),
            is_sync: i == 0,
            composition_offset: ts.sample_composition_time_offset.unwrap_or(0),
        });
    }
    (track_id, samples)
}

#[test]
fn init_segment_from_real_config_reparses() {
    let data = fixture();
    let (_, moov_bytes) = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(&moov_bytes).unwrap();
    let (specs, _) = track_specs(&moov);

    let init = build_init_segment(&specs, 1000).expect("build init");

    // ftyp first, then moov.
    let (_, ftyp) = find_top_box(&init, b"ftyp");
    assert_eq!(&ftyp[4..8], b"ftyp");
    let (_, out_moov_bytes) = find_top_box(&init, b"moov");
    let out_moov = MovieBox::parse(&out_moov_bytes).unwrap();

    assert_eq!(out_moov.tracks.len(), 2, "two tracks");
    let mvex = out_moov
        .mvex
        .as_ref()
        .expect("fragmented init must have mvex");
    assert_eq!(mvex.trex.len(), 2, "one trex per track");
    assert_eq!(mvex.trex[0].track_id, 1);
    assert_eq!(mvex.trex[1].track_id, 2);

    // Video stsd survived with a real avcC (SPS present).
    match track_stsd_entry(&out_moov, 0) {
        SampleEntryVariant::Avc1(a) => {
            assert!(!a.config.config.sps.is_empty(), "avcC SPS survived");
            assert!(a.visual.width > 0 && a.visual.height > 0, "non-zero dims");
        }
        _ => panic!("video stsd not avc1"),
    }
    // Audio stsd survived with an esds.
    match track_stsd_entry(&out_moov, 1) {
        SampleEntryVariant::Mp4a(m) => {
            assert!(m.config_boxes.iter().any(|b| &b.box_type == b"esds"));
        }
        _ => panic!("audio stsd not mp4a"),
    }
    // Empty sample tables (fragmented init: samples live in fragments).
    let vstbl = out_moov.tracks[0]
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .unwrap()
        .stbl
        .as_ref()
        .unwrap();
    for c in &vstbl.children {
        if let StblChild::Stsz(s) = c {
            assert!(s.entries.is_empty(), "stsz empty in fragmented init");
        }
    }
}

#[test]
fn media_segment_offsets_and_samples_are_correct() {
    let data = fixture();

    // Locate the FIRST moof + its samples.
    let (moof_off, moof_bytes) = find_top_box(&data, b"moof");
    let moof = MovieFragmentBox::parse_body(&moof_bytes[8..]).expect("parse moof");
    assert_eq!(moof.traf.len(), 2, "fixture moof has two trafs");

    let (vid_id, vid_samples) = extract_samples(&data, moof_off, &moof, 0);
    let (aud_id, aud_samples) = extract_samples(&data, moof_off, &moof, 1);
    assert!(!vid_samples.is_empty() && !aud_samples.is_empty());

    let base_v = moof.traf[0].tfdt.as_ref().unwrap().base_media_decode_time();
    let base_a = moof.traf[1].tfdt.as_ref().unwrap().base_media_decode_time();

    let tracks = [
        FragmentTrackData {
            track_id: vid_id,
            base_media_decode_time: base_v,
            samples: &vid_samples,
        },
        FragmentTrackData {
            track_id: aud_id,
            base_media_decode_time: base_a,
            samples: &aud_samples,
        },
    ];
    let seg = build_media_segment(7, &tracks).expect("build media segment");

    // styp first.
    let (styp_off, styp) = find_top_box(&seg, b"styp");
    assert_eq!(styp_off, 0, "styp is first box");
    assert_eq!(&styp[4..8], b"styp");

    // Re-parse moof + mdat from the produced segment.
    let (out_moof_off, out_moof_bytes) = find_top_box(&seg, b"moof");
    let out_moof = MovieFragmentBox::parse_body(&out_moof_bytes[8..]).unwrap();
    assert_eq!(out_moof.mfhd.sequence_number, 7);
    assert_eq!(out_moof.traf.len(), 2);

    // tfdt preserved.
    assert_eq!(
        out_moof.traf[0]
            .tfdt
            .as_ref()
            .unwrap()
            .base_media_decode_time(),
        base_v
    );
    assert_eq!(
        out_moof.traf[1]
            .tfdt
            .as_ref()
            .unwrap()
            .base_media_decode_time(),
        base_a
    );

    // trun sample counts match the inputs.
    assert_eq!(out_moof.traf[0].trun[0].samples.len(), vid_samples.len());
    assert_eq!(out_moof.traf[1].trun[0].samples.len(), aud_samples.len());

    // data_offset math: video block starts at moof_size + 8 (mdat header),
    // audio block right after the video bytes. Offsets are relative to moof.
    let out_moof_size = out_moof_bytes.len();
    let vid_bytes: usize = vid_samples.iter().map(|s| s.data.len()).sum();
    let v_off = out_moof.traf[0].trun[0].data_offset.unwrap() as usize;
    let a_off = out_moof.traf[1].trun[0].data_offset.unwrap() as usize;
    assert_eq!(
        v_off,
        out_moof_size + 8,
        "video data_offset = moof + mdat hdr"
    );
    assert_eq!(a_off, out_moof_size + 8 + vid_bytes, "audio after video");

    // Walk the produced mdat by the trun sizes and recover byte-exact samples.
    let (_, mdat_bytes) = find_top_box(&seg, b"mdat");
    let mdat = MediaDataBox::parse_box(&mdat_bytes).unwrap();
    // The mdat body begins at out_moof_off + out_moof_size + 8 in the segment;
    // data_offset is relative to the moof, so subtract that base to index mdat.
    let mdat_base_in_seg = out_moof_off + out_moof_size + 8;

    // Video samples.
    let mut cur = v_off + out_moof_off - mdat_base_in_seg;
    for s in &vid_samples {
        assert_eq!(&mdat.data[cur..cur + s.data.len()], &s.data[..]);
        cur += s.data.len();
    }
    // Audio samples.
    let mut cur = a_off + out_moof_off - mdat_base_in_seg;
    for s in &aud_samples {
        assert_eq!(&mdat.data[cur..cur + s.data.len()], &s.data[..]);
        cur += s.data.len();
    }
}
