//! The sans-IO reader against the recorded `origin/main` `WebmDemux` values,
//! plus the unknown-size, lacing and Cues-seek gates (streaming-first Phase 5,
//! slice 3).

use std::time::Duration;

use broadcast_core::Serialize;

use broadcast_matroska::ebml::document::parse_document_start;
use broadcast_matroska::ebml::vint::{VintSize, VintWidth};
use broadcast_matroska::ebml::{ElementHeader, ElementId};
use broadcast_matroska::matroska::block::{
    FLAG_KEYFRAME, FLAG_LACING_EBML, FLAG_LACING_FIXED, FLAG_LACING_XIPH,
};
use broadcast_matroska::matroska::ids;
use broadcast_matroska::matroska::{Block, BlockHeader, Lacing};
use broadcast_matroska::reader::SeekMethod;
use broadcast_matroska::writer::{
    BlockFlags, InfoSpec, Layout, TrackSpec, Write, Writer, info_element, tracks_element,
};

const FIXTURES: &[&str] = &[
    "mkv/h264-aac.mkv",
    "mkv/hevc-aac.mkv",
    "mkv/vorbis.webm",
    "mkv/vp8-opus.webm",
    "mkv/vp9-opus.mkv",
    "mkv/vp9-opus.webm",
];

#[test]
fn every_fixture_matches_the_recorded_demux_through_both_drivers() {
    for fixture in FIXTURES {
        let want = super::support::golden(fixture);
        for driver in [
            super::support::Driver::Seekable,
            super::support::Driver::Forward,
        ] {
            let got = super::support::collect(fixture, driver);
            assert!(got.info.is_some(), "{fixture}: no Info step");
            assert!(got.cues <= got.frames.len(), "{fixture}: cue count");
            assert_eq!(
                super::support::summary_lines(&got),
                want.tracks,
                "{fixture}: {driver:?}"
            );
        }
    }
}

#[test]
fn every_fixture_matches_at_every_supply_size() {
    for fixture in FIXTURES {
        let want = super::support::golden(fixture);
        let bytes = test_fixtures::read(fixture);
        for chunk in [1usize, 4096, usize::MAX] {
            let seekable = super::support::collect_bytes_seekable(&bytes, chunk);
            assert_eq!(
                super::support::summary_lines(&seekable),
                want.tracks,
                "{fixture}: seekable chunk {chunk}"
            );
            let forward = super::support::collect_bytes_forward(&bytes, chunk);
            assert_eq!(
                super::support::summary_lines(&forward),
                want.tracks,
                "{fixture}: forward chunk {chunk}"
            );
        }
    }
}

#[test]
fn unknown_size_segment_and_clusters_read_the_same_samples() {
    for fixture in FIXTURES {
        let want = super::support::golden(fixture);
        let bytes = test_fixtures::read(fixture);
        let rewritten = unknown_sizes(&bytes);
        assert_ne!(rewritten, bytes, "{fixture}: rewrite changed nothing");
        for chunk in [4096, usize::MAX] {
            let seekable = super::support::collect_bytes_seekable(&rewritten, chunk);
            assert_eq!(
                super::support::summary_lines(&seekable),
                want.tracks,
                "{fixture}: unknown sizes, seekable chunk {chunk}"
            );
            let forward = super::support::collect_bytes_forward(&rewritten, chunk);
            assert_eq!(
                super::support::summary_lines(&forward),
                want.tracks,
                "{fixture}: unknown sizes, forward chunk {chunk}"
            );
        }
    }
}

#[test]
fn all_three_lacing_forms_yield_one_frame_each() {
    let payloads: [[&[u8]; 3]; 3] = [
        [b"alpha", b"bravo!", b"ch"],
        [b"delta-frame", b"echo", b"foxtrot"],
        [b"fixed-a", b"fixed-b", b"fixed-c"],
    ];
    let mut blocks = Vec::new();
    for (index, frames) in payloads.iter().enumerate() {
        let lacing = match index {
            0 => Lacing::Xiph {
                frames: frames.to_vec(),
            },
            1 => Lacing::Ebml {
                frames: frames.to_vec(),
                first_width: VintWidth::MIN,
                delta_width: VintWidth::MIN,
            },
            _ => Lacing::Fixed {
                frames: frames.to_vec(),
            },
        };
        let flags = match index {
            0 => FLAG_LACING_XIPH,
            1 => FLAG_LACING_EBML,
            _ => FLAG_LACING_FIXED,
        } | FLAG_KEYFRAME;
        let header = BlockHeader::with_values(1, index as i16, flags).unwrap();
        let payload = Block::from_parts(header, lacing).try_to_bytes().unwrap();
        blocks.push(element_bytes(ids::SIMPLE_BLOCK, &payload));
    }
    let document = synthetic(&blocks);
    let collected = super::support::collect_bytes_seekable(&document, usize::MAX);
    assert_eq!(collected.frames.len(), 9);
    for (index, frame) in collected.frames.iter().enumerate() {
        let block = index / 3;
        let lace = index % 3;
        assert_eq!(frame.track, 1);
        assert_eq!(u64::from(frame.lace_index), lace as u64);
        assert_eq!(frame.lace_count, 3);
        assert_eq!(frame.block_pts, block as i64);
        assert!(frame.keyframe);
        let want = if block == 2 && lace == 2 {
            b"fixed-c".as_slice()
        } else {
            payloads[block][lace]
        };
        assert_eq!(frame.data, want, "block {block} lace {lace}");
    }
}

#[test]
fn cues_seek_lands_on_the_preceding_keyframe_and_the_scan_agrees() {
    let with_cues = writer_sample();
    let without = strip_cues(&with_cues);
    let cues_outcome = {
        let mut slice = super::support::Slice::new(&with_cues);
        slice.walk().unwrap();
        slice.reader.seek_to(1, 1500).unwrap()
    };
    let scan_outcome = {
        let mut slice = super::support::Slice::new(&without);
        slice.walk().unwrap();
        slice.reader.seek_to(1, 1500).unwrap()
    };
    assert_eq!(cues_outcome.method, SeekMethod::Cues);
    assert_eq!(scan_outcome.method, SeekMethod::ClusterScan);
    assert_eq!(cues_outcome.offset, scan_outcome.offset);
    assert_eq!(cues_outcome.timestamp, 1000);
    for bytes in [&with_cues, &without] {
        let mut slice = super::support::Slice::new(bytes);
        slice.walk().unwrap();
        let outcome = slice.reader.seek_to(1, 1500).unwrap();
        assert_eq!(outcome.offset, cues_outcome.offset);
        let (timestamp, keyframe) = slice.next_block();
        assert_eq!(timestamp, 1000);
        assert!(keyframe);
    }
}

#[test]
fn a_cues_seek_before_the_first_point_falls_back_to_the_index() {
    let bytes = writer_sample();
    let mut slice = super::support::Slice::new(&bytes);
    slice.walk().unwrap();
    let outcome = slice.reader.seek_to(1, -1).unwrap();
    assert_eq!(outcome.method, SeekMethod::ClusterScan);
    assert_eq!(outcome.timestamp, 0);
}

#[test]
fn a_forward_only_source_refuses_a_backwards_seek() {
    let bytes = writer_sample();
    let mut slice = super::support::Slice::forward(&bytes);
    slice.walk().unwrap();
    let error = slice.reader.seek_to(1, 0).unwrap_err();
    assert!(matches!(
        error,
        broadcast_matroska::Error::BackwardSeek { .. }
    ));
    let _ = bytes;
}

/// Rewrite the top-level `Segment` and every `Cluster` to the unknown size.
fn unknown_sizes(bytes: &[u8]) -> Vec<u8> {
    let (_, header_used) = parse_document_start(bytes).unwrap();
    let mut out = bytes[..header_used].to_vec();
    let (segment, used) = ElementHeader::parse(&bytes[header_used..]).unwrap();
    assert_eq!(segment.id().value(), ids::SEGMENT);
    let payload_start = header_used + used;
    let payload_end = payload_start + usize::try_from(segment.payload_len().unwrap()).unwrap();
    write_header(&mut out, ids::SEGMENT, VintSize::unknown_max());
    let mut position = payload_start;
    while position < payload_end {
        let (header, header_len) = ElementHeader::parse(&bytes[position..]).unwrap();
        let len = usize::try_from(header.payload_len().unwrap()).unwrap();
        let end = position + header_len + len;
        if header.id().value() == ids::CLUSTER {
            write_header(&mut out, ids::CLUSTER, VintSize::unknown_max());
            out.extend_from_slice(&bytes[position + header_len..end]);
        } else if header.id().value() != ids::SEEK_HEAD {
            out.extend_from_slice(&bytes[position..end]);
        }
        position = end;
    }
    out
}

fn write_header(out: &mut Vec<u8>, id: u32, size: VintSize) {
    let header = ElementHeader::new(ElementId::try_from_raw(id).unwrap(), size);
    let mut bytes = vec![0u8; header.serialized_len()];
    header.write(&mut bytes).unwrap();
    out.extend_from_slice(&bytes);
}

fn element_bytes(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_header(
        &mut out,
        id,
        VintSize::minimal(payload.len() as u64).unwrap(),
    );
    out.extend_from_slice(payload);
    out
}

/// A minimal document: Info, one track, one Cluster carrying `blocks`.
fn synthetic(blocks: &[Vec<u8>]) -> Vec<u8> {
    let info = info_element(&InfoSpec::new(1_000_000, 0.0)).unwrap();
    let specs = [TrackSpec {
        track_number: 1,
        track_uid: 1,
        track_type: 1,
        codec_id: "V_VP9",
        codec_private: None,
        default_duration_ns: None,
        video: Some((16, 16)),
        audio: None,
    }];
    let tracks = tracks_element(&specs).unwrap();
    let mut segment = Vec::new();
    segment.extend_from_slice(&info.try_to_bytes().unwrap());
    segment.extend_from_slice(&tracks.try_to_bytes().unwrap());
    let mut cluster = Vec::new();
    cluster.extend_from_slice(&element_bytes(ids::TIMESTAMP, &[0]));
    for block in blocks {
        cluster.extend_from_slice(block);
    }
    segment.extend_from_slice(&element_bytes(ids::CLUSTER, &cluster));
    let mut document = Vec::new();
    document.extend_from_slice(&ebml_header());
    write_header(&mut document, ids::SEGMENT, VintSize::unknown_max());
    document.extend_from_slice(&segment);
    document
}

fn ebml_header() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::EBML_VERSION_ID,
        &[1],
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::EBML_READ_VERSION_ID,
        &[1],
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::EBML_MAX_ID_LENGTH_ID,
        &[4],
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::EBML_MAX_SIZE_LENGTH_ID,
        &[8],
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::DOC_TYPE_ID,
        b"matroska",
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::DOC_TYPE_VERSION_ID,
        &[4],
    ));
    body.extend_from_slice(&element_bytes(
        broadcast_matroska::ebml::document::DOC_TYPE_READ_VERSION_ID,
        &[2],
    ));
    element_bytes(broadcast_matroska::ebml::document::EBML_ID, &body)
}

/// A three-Cluster file with video keyframes at 0, 1000 and 2000 ticks.
fn writer_sample() -> Vec<u8> {
    let info = info_element(&InfoSpec::new(1_000_000, 0.0)).unwrap();
    let specs = [TrackSpec {
        track_number: 1,
        track_uid: 1,
        track_type: 1,
        codec_id: "V_VP9",
        codec_private: None,
        default_duration_ns: None,
        video: Some((16, 16)),
        audio: None,
    }];
    let tracks = tracks_element(&specs).unwrap();
    let mut writer = Writer::new(Layout::new(Duration::from_millis(500)), &info, &tracks).unwrap();
    let mut out = Vec::new();
    for (timestamp, keyframe) in [
        (0i64, true),
        (400, false),
        (1000, true),
        (1400, false),
        (2000, true),
        (2400, false),
    ] {
        let writes = writer
            .push_block(
                1,
                timestamp,
                Some(400),
                BlockFlags::keyframe(keyframe),
                &[0u8; 8],
            )
            .unwrap();
        apply(writes, &mut out);
    }
    apply(writer.finish().unwrap(), &mut out);
    out
}

fn apply(writes: Vec<Write>, out: &mut Vec<u8>) {
    for write in writes {
        match write {
            Write::Append(bytes) => out.extend_from_slice(&bytes),
            Write::At { offset, bytes } => {
                let start = usize::try_from(offset).unwrap();
                out[start..start + bytes.len()].copy_from_slice(&bytes);
            }
            _ => {}
        }
    }
}

fn strip_cues(bytes: &[u8]) -> Vec<u8> {
    let (_, header_used) = parse_document_start(bytes).unwrap();
    let (_segment, used) = ElementHeader::parse(&bytes[header_used..]).unwrap();
    let payload_start = header_used + used;
    let payload_end = bytes.len();
    let mut out = bytes[..header_used].to_vec();
    write_header(&mut out, ids::SEGMENT, VintSize::unknown_max());
    let mut position = payload_start;
    while position < payload_end {
        let (header, header_len) = ElementHeader::parse(&bytes[position..]).unwrap();
        let len = usize::try_from(header.payload_len().unwrap()).unwrap();
        let end = position + header_len + len;
        if header.id().value() != ids::CUES {
            out.extend_from_slice(&bytes[position..end]);
        }
        position = end;
    }
    out
}

#[test]
fn block_group_discard_padding_and_seek_pre_roll_are_reported() {
    use broadcast_matroska::Limits;
    use broadcast_matroska::driver::{DriverStep, FileReader};
    use broadcast_matroska::reader::Mode;
    use std::fs::File;

    let file = File::open(test_fixtures::path("mkv/vp9-opus.webm")).expect("fixture opens");
    let mut reader = FileReader::new(file, Mode::Strict, Limits::new()).expect("driver");
    let mut discard = Vec::new();
    let mut seek_pre_roll = None;
    loop {
        match reader.step().expect("step") {
            DriverStep::Tracks { tracks, .. } => {
                seek_pre_roll = tracks
                    .iter()
                    .find(|info| info.codec_id() == "A_OPUS")
                    .and_then(|info| info.seek_pre_roll_ns());
            }
            DriverStep::Block(block) => discard.push(block.discard_padding),
            DriverStep::Done => break,
            _ => {}
        }
    }
    assert_eq!(
        seek_pre_roll,
        Some(80_000_000),
        "SeekPreRoll is 80 ms on this Opus track"
    );
    let last = discard.len().saturating_sub(1);
    assert_eq!(
        discard[last],
        Some(13_500_000),
        "the final block states 13.5 ms of end padding"
    );
    assert!(
        discard[..last].iter().all(Option::is_none),
        "no other block states DiscardPadding"
    );
}
