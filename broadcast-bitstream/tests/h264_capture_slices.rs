//! Real-fixture check: every slice header of a broadcast capture, decoded with
//! the stream's own SPS/PPS and this crate alone.
//!
//! `fixtures/ts/h264-aac.ts` is an MPEG-2 transport stream carrying H.264
//! (320x240 Main profile, level 1.3) and AAC. The transport layer is walked with
//! `mpeg-ts` + `mpeg-pes` (dev-dependencies here) so the elementary-stream
//! bytes are real broadcast bytes, not a hand-built bitstream; access-unit
//! boundaries and the slice header are this crate's job.
//!
//! The picture and IDR counts are the ones `transmux`'s demuxer reports for the
//! same file (75 samples, 3 key frames), so the bitstream crate reads the same
//! pictures without the container layer.

#![cfg(feature = "h264")]

use broadcast_bitstream::nal::{
    AVC_NAL_IDR, AVC_NAL_PPS, AVC_NAL_SLICE, AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, nal_unit_type,
};
use broadcast_bitstream::slice_header::{AvcParamSets, AvcPicture};
use broadcast_bitstream::{AccessUnitSplitter, iter_annexb_nals};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::TsPacket;
use std::collections::BTreeMap;

/// One TS packet is 188 bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_SIZE: usize = 188;

/// One coded picture as the capture carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Picture {
    /// `slice_type` as coded (Table 7-6).
    slice_type: u8,
    /// `field_pic_flag`.
    field: bool,
    /// An IDR picture.
    idr: bool,
}

/// The PID whose PES payloads carry the H.264 elementary stream.
fn video_pid(file: &[u8]) -> Option<u16> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        let Some(payload) = ts.payload else {
            continue;
        };
        let Some(pes) = assemblers
            .entry(ts.header.pid.value())
            .or_default()
            .feed(ts.header.pusi, payload)
        else {
            continue;
        };
        if es_of(&pes).is_some_and(|es| {
            iter_annexb_nals(es).any(|nal| nal_unit_type(NalCodec::Avc, nal) == Some(AVC_NAL_SPS))
        }) {
            return Some(ts.header.pid.value());
        }
    }
    None
}

/// Every coded picture of the capture, in decode order, and the number of slice
/// headers parsed.
fn pictures(file: &[u8]) -> (Vec<Picture>, usize) {
    let Some(pid) = video_pid(file) else {
        return (Vec::new(), 0);
    };
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
    let mut params = AvcParamSets::new();
    let (mut out, mut slices) = (Vec::new(), 0usize);
    let mut take = |au: Vec<u8>, params: &mut AvcParamSets, out: &mut Vec<Picture>| {
        if let Some(picture) = picture_of(&au, params) {
            slices += 1;
            out.push(picture);
        }
    };
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        if ts.header.pid.value() != pid {
            continue;
        }
        let Some(payload) = ts.payload else {
            continue;
        };
        let Some(pes) = assemblers
            .entry(pid)
            .or_default()
            .feed(ts.header.pusi, payload)
        else {
            continue;
        };
        let Some(es) = es_of(&pes) else {
            continue;
        };
        if splitter.push(es).is_err() {
            break;
        }
        while let Some(au) = splitter.pop() {
            take(au, &mut params, &mut out);
        }
    }
    // The last PES is only returned at end of stream.
    if let Some(pes) = assemblers.get_mut(&pid).and_then(PesAssembler::flush)
        && let Some(es) = es_of(&pes)
    {
        let _ = splitter.push(es);
    }
    splitter.finish();
    while let Some(au) = splitter.pop() {
        take(au, &mut params, &mut out);
    }
    (out, slices)
}

/// The elementary-stream bytes of one assembled PES packet.
fn es_of(pes: &[u8]) -> Option<&[u8]> {
    PesPacket::parse_unit(pes)
        .ok()
        .map(|(packet, _)| packet.payload)
}

/// One access unit's coded picture, from its first slice header.
fn picture_of(au: &[u8], params: &mut AvcParamSets) -> Option<Picture> {
    let nals: Vec<&[u8]> = iter_annexb_nals(au).collect();
    for nal in &nals {
        if matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(AVC_NAL_SPS | AVC_NAL_PPS)
        ) {
            params.observe_nal(nal);
        }
    }
    let first = nals.iter().find(|nal| {
        matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(AVC_NAL_SLICE | AVC_NAL_IDR)
        )
    })?;
    let (header, _) = params.parse_slice(first)?;
    Some(Picture {
        slice_type: header.slice_type(),
        field: header.field_pic_flag(),
        idr: header.idr(),
    })
}

#[test]
fn every_slice_header_of_the_capture_decodes_to_one_picture_each() {
    let file = test_fixtures::read("ts/h264-aac.ts");
    let (pictures, slices) = pictures(&file);
    assert_eq!(pictures.len(), 75, "75 coded pictures");
    assert_eq!(slices, 75, "one slice per picture in this capture");
    assert_eq!(
        pictures.iter().filter(|p| p.idr).count(),
        3,
        "3 IDR pictures"
    );
    assert!(
        pictures.iter().all(|p| !p.field),
        "the capture is progressive: no field pictures"
    );
}

#[test]
fn idr_pictures_are_intra_slices_and_the_rest_are_not() {
    let file = test_fixtures::read("ts/h264-aac.ts");
    let (pictures, _) = pictures(&file);
    // `slice_type % 5`: 2 = I, 0 = P, 1 = B (Table 7-6).
    for picture in &pictures {
        if picture.idr {
            assert_eq!(picture.slice_type % 5, 2, "{picture:?} is an IDR");
        }
    }
    assert!(
        pictures.iter().any(|p| !p.idr && p.slice_type % 5 != 2),
        "the capture carries inter pictures too"
    );
}

#[test]
fn the_picture_classifier_agrees_with_the_slice_headers() {
    let file = test_fixtures::read("ts/h264-aac.ts");
    let pid = video_pid(&file).expect("an H.264 PID");
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut params = AvcParamSets::new();
    let mut a_units = 0usize;
    let mut accumulated = Vec::new();
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        if ts.header.pid.value() != pid {
            continue;
        }
        let Some(payload) = ts.payload else {
            continue;
        };
        let Some(pes) = assemblers
            .entry(pid)
            .or_default()
            .feed(ts.header.pusi, payload)
        else {
            continue;
        };
        let Some(es) = es_of(&pes) else {
            continue;
        };
        accumulated.extend_from_slice(es);
    }
    if let Some(pes) = assemblers.get_mut(&pid).and_then(PesAssembler::flush)
        && let Some(es) = es_of(&pes)
    {
        accumulated.extend_from_slice(es);
    }
    for au in broadcast_bitstream::split_access_units(NalCodec::Avc, &accumulated) {
        let nals: Vec<&[u8]> = iter_annexb_nals(&au).collect();
        match params.classify_nals(nals.iter().copied()) {
            AvcPicture::Frame => a_units += 1,
            other => panic!("the capture is frame-coded, got {other:?}"),
        }
    }
    assert_eq!(a_units, 75);
}
