//! Ungameable gate test for AC-3/E-AC-3 fMP4 carriage (#426).
//!
//! Oracle bytes come from ffmpeg's own MP4 muxer (DOLBY-ORACLE.md).
//! The test demuxes real TS fixtures, parses the syncframe BSI, builds the
//! config box, and asserts byte-exact match with ffmpeg's output.

use broadcast_common::{Parse, Serialize};

use transmux::{
    Ac3SpecificBox, Ac3SyncframeInfo, CodecConfig, Ec3SpecificBox, Ec3SyncframeInfo, TrackSpec,
    box_iter, build_init_segment,
};

// Oracle bytes from DOLBY-ORACLE.md
const DAC3_ORACLE: [u8; 3] = [0x50, 0x09, 0x40];
const DEC3_ORACLE: [u8; 5] = [0x06, 0x00, 0x60, 0x02, 0x00];

/// Extract raw audio ES data from a single-audio-PID TS file by collecting TS
/// payload bytes for the given PID, stripping adaptation fields and PUSI pointers.
/// Returns the concatenated elementary stream bytes.
fn extract_es_data(ts_path: &str, target_pid: u16) -> Vec<u8> {
    let data = std::fs::read(ts_path).unwrap();
    let mut es = Vec::new();
    for chunk in data.chunks_exact(188) {
        if chunk[0] != 0x47 {
            continue;
        }
        let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;
        if pid != target_pid {
            continue;
        }
        let afc = (chunk[3] >> 4) & 3;
        let has_payload = (afc & 1) != 0;
        if !has_payload {
            continue;
        }
        let has_adapt = (afc & 2) != 0;
        let pusi = (chunk[1] >> 6) & 1;
        let mut start = 4usize;
        if has_adapt {
            let aflen = chunk[4] as usize;
            start = 5 + aflen;
        }
        if pusi != 0 {
            // PES packet start — skip pointer field + any PES stuffing.
            let pointer = chunk[start] as usize;
            start += 1 + pointer;
        }
        es.extend_from_slice(&chunk[start..]);
    }
    es
}

const AC3_PID: u16 = 0x0100;
const EAC3_PID: u16 = 0x0100;

/// Recursively search for a child box with four-CC `tag` inside a parent
/// box with four-CC `parent_tag`. For audio sample entries (ac-3, ec-3),
/// skips the 28-byte fixed AudioSampleEntry prefix before searching children.
#[allow(clippy::only_used_in_recursion)]
fn find_child_box_body<'a>(
    data: &'a [u8],
    parent_tag: &[u8; 4],
    child_tag: &[u8; 4],
) -> Option<&'a [u8]> {
    const AUDIO_ENTRY_FIXED: usize = 28;
    const AUDIO_FOURCCS: &[[u8; 4]] = &[*b"ac-3", *b"ec-3"];

    for item in box_iter(data) {
        let (bx, _sz) = match item {
            Ok(v) => v,
            Err(_) => continue,
        };
        // If this is an audio sample entry, skip fixed prefix and iterate config boxes.
        if AUDIO_FOURCCS.contains(&bx.header.box_type.0) {
            let config_region = &bx.body[AUDIO_ENTRY_FIXED.min(bx.body.len())..];
            for ch in box_iter(config_region) {
                let (cb_bx, _) = match ch {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if &cb_bx.header.box_type.0 == child_tag {
                    return Some(cb_bx.body);
                }
            }
        }
        // Recurse into container boxes.
        if let Some(found) = find_child_box_body(bx.body, parent_tag, child_tag) {
            return Some(found);
        }
    }
    None
}

#[test]
fn ac3_syncframe_to_dac3_oracle() {
    let es = extract_es_data("../fixtures/ts/dolby/ac3.ts", AC3_PID);
    let info = Ac3SyncframeInfo::from_es(&es).unwrap();

    assert_eq!(info.fscod, 1);
    assert_eq!(info.bsid, 8);
    assert_eq!(info.acmod, 1);
    assert!(!info.lfeon);
    // frmsizecod >> 1 = bit_rate_code; oracle has bit_rate_code=10
    assert_eq!(info.frmsizecod >> 1, 10);

    let dac3 = info.into_dac3();
    let mut buf = [0u8; 3];
    dac3.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[..],
        &DAC3_ORACLE[..],
        "dac3 bytes mismatch with ffmpeg oracle"
    );
}

#[test]
fn eac3_syncframe_to_dec3_oracle() {
    let es = extract_es_data("../fixtures/ts/dolby/eac3.ts", EAC3_PID);
    let info = Ec3SyncframeInfo::from_es(&es).unwrap();

    assert_eq!(info.strmtyp, 0);
    assert_eq!(info.substreamid, 0);
    assert_eq!(info.acmod, 1);
    assert!(!info.lfeon);
    assert_eq!(info.bsid, 16);

    let dec3 = info.into_dec3();
    assert_eq!(dec3.data_rate, 192);
    assert_eq!(dec3.num_ind_sub, 0);

    let mut buf = vec![0u8; dec3.serialized_len()];
    dec3.serialize_into(&mut buf).unwrap();
    assert_eq!(
        &buf[..],
        &DEC3_ORACLE[..],
        "dec3 bytes mismatch with ffmpeg oracle"
    );
}

#[test]
fn dac3_box_round_trip() {
    let box1 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
    let mut buf = [0u8; 3];
    box1.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[..], &DAC3_ORACLE[..]);

    // Mutate acmod → bytes change (proves no passthrough)
    let mut box2 = box1;
    box2.acmod = 2;
    let mut buf2 = [0u8; 3];
    box2.serialize_into(&mut buf2).unwrap();
    assert_ne!(&buf2[..], &DAC3_ORACLE[..]);
}

#[test]
fn dec3_box_round_trip() {
    let box1 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
    let mut buf = vec![0u8; box1.serialized_len()];
    box1.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[..], &DEC3_ORACLE[..]);

    // Mutate acmod → bytes change
    let mut box2 = box1;
    box2.substreams[0].acmod = 2;
    let mut buf2 = vec![0u8; box2.serialized_len()];
    box2.serialize_into(&mut buf2).unwrap();
    assert_ne!(&buf2[..], &DEC3_ORACLE[..]);
}

#[test]
fn ac3_init_segment_sample_entry() {
    let dac3 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
    let sample_rate: u32 = 44100;

    let tracks = vec![TrackSpec::new(
        1,
        sample_rate,
        CodecConfig::Ac3 {
            config: dac3,
            channel_count: 1,
            sample_rate,
            sample_size: 16,
        },
    )];

    let init = build_init_segment(&tracks, 1000).unwrap();

    let dac3_body = find_child_box_body(&init, b"ac-3", b"dac3").unwrap();
    assert_eq!(dac3_body, &DAC3_ORACLE[..], "dac3 in init segment mismatch");
}

#[test]
fn ec3_init_segment_sample_entry() {
    let dec3 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
    let sample_rate: u32 = 44100;

    let tracks = vec![TrackSpec::new(
        1,
        sample_rate,
        CodecConfig::Eac3 {
            config: dec3,
            channel_count: 1,
            sample_rate,
            sample_size: 16,
        },
    )];

    let init = build_init_segment(&tracks, 1000).unwrap();

    let dec3_body = find_child_box_body(&init, b"ec-3", b"dec3").unwrap();
    assert_eq!(dec3_body, &DEC3_ORACLE[..], "dec3 in init segment mismatch");
}

#[test]
fn rfc6381_codes() {
    let dac3 = Ac3SpecificBox::parse(&DAC3_ORACLE).unwrap();
    assert_eq!(dac3.rfc6381(), "ac-3");

    let dec3 = Ec3SpecificBox::parse(&DEC3_ORACLE).unwrap();
    assert_eq!(dec3.rfc6381(), "ec-3");
}
