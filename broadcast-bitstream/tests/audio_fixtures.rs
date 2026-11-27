//! The audio parsers against the committed captures: an ADTS chain walked to
//! the last byte, the AAC track of a transport-stream capture, and one AC-3,
//! E-AC-3 and DTS elementary stream whose every frame is parsed and whose
//! stated sample rate, channel layout and frame size are pinned.
//!
//! The transport-stream capture is demuxed through `mpeg-ts`/`mpeg-pes` (the
//! crate's dev-dependencies) so the bytes handed to the parsers are the real
//! broadcast payloads, not a hand-built bitstream.

use broadcast_bitstream::aac_asc::{adts_header_len, build_adts_header, parse_adts_header};
use broadcast_bitstream::ac3_syncframe::{
    Ac3SyncframeInfo, Ec3SyncframeInfo, split_ac3_syncframes, split_eac3_syncframes,
};
use broadcast_bitstream::dts_core::{DtsCoreFrameInfo, split_dts_core_frames};
use broadcast_bitstream::{AudioSpecificConfig, SamplingFrequencyIndex};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::TsPacket;
use std::collections::BTreeMap;

/// One TS packet is 188 bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_SIZE: usize = 188;

/// Every PID's assembled PES payloads, in file order.
fn pes_payloads(file: &[u8]) -> BTreeMap<u16, Vec<u8>> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut out: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        let Some(payload) = ts.payload else {
            continue;
        };
        let pid = ts.header.pid.value();
        let assembler = assemblers.entry(pid).or_default();
        let Some(pes) = assembler.feed(ts.header.pusi, payload) else {
            continue;
        };
        let Ok(packet) = PesPacket::parse(&pes) else {
            continue;
        };
        out.entry(pid)
            .or_default()
            .extend_from_slice(packet.payload);
    }
    for (pid, mut assembler) in assemblers {
        let Some(pes) = assembler.flush() else {
            continue;
        };
        let Ok(packet) = PesPacket::parse(&pes) else {
            continue;
        };
        out.entry(pid)
            .or_default()
            .extend_from_slice(packet.payload);
    }
    out
}

/// The elementary stream of the one PID whose payload starts with a valid ADTS
/// header.
fn adts_elementary_stream(file: &[u8]) -> Vec<u8> {
    pes_payloads(file)
        .into_values()
        .find(|es| parse_adts_header(es).is_ok())
        .expect("an AAC PES payload")
}

/// Walk `fixtures/audio/aac.adts` as a chain of ADTS frames.
#[test]
fn adts_fixture_walks_to_the_last_byte_and_rebuilds_each_header() {
    let data = test_fixtures::read("audio/aac.adts");
    let mut off = 0usize;
    let mut frames = 0usize;
    let mut first = None;
    while off < data.len() {
        let hdr = parse_adts_header(&data[off..]).expect("an ADTS header");
        let len = hdr.frame_length() as usize;
        assert!(len >= adts_header_len(&hdr));
        assert!(off + len <= data.len(), "frame {frames} overruns the file");
        if first.is_none() {
            first = Some(hdr);
        }
        // The header the parser read is the header the builder writes back.
        let rebuilt = build_adts_header(
            hdr.profile(),
            hdr.sampling_frequency_index(),
            hdr.channel_configuration(),
            hdr.frame_length(),
        );
        assert_eq!(&data[off..off + rebuilt.len()], &rebuilt, "frame {frames}");
        off += len;
        frames += 1;
    }
    assert_eq!(
        off,
        data.len(),
        "the chain must end exactly at the last byte"
    );
    assert_eq!(frames, 48);

    let first = first.expect("at least one frame");
    assert_eq!(first.profile(), 1, "AAC LC");
    assert_eq!(
        SamplingFrequencyIndex::from(first.sampling_frequency_index()),
        SamplingFrequencyIndex::Fs48000
    );
    assert_eq!(first.channel_configuration(), 1, "mono");
    assert_eq!(first.frame_length(), 274);

    let asc = AudioSpecificConfig::from_adts_header(&first);
    assert_eq!(asc.effective_sampling_frequency(), Some(48_000));
    // Only a header: no `GASpecificConfig` bits, so no frame-length flag.
    assert_eq!(asc.samples_per_frame(), None);
}

/// The AAC track of a real broadcast capture: 131 ADTS frames.
#[test]
fn broadcast_capture_yields_131_aac_frames() {
    let file = test_fixtures::read("ts/h264-aac.ts");
    let es = adts_elementary_stream(&file);
    let mut off = 0usize;
    let mut frames = 0usize;
    while off < es.len() {
        let hdr = parse_adts_header(&es[off..]).expect("an ADTS header");
        let len = hdr.frame_length() as usize;
        if len < adts_header_len(&hdr) || off + len > es.len() {
            break;
        }
        off += len;
        frames += 1;
    }
    assert_eq!(frames, 131);
}

/// One AC-3 elementary stream: every frame parses, with the same sample rate
/// and channel layout throughout.
#[test]
fn ac3_fixture_parses_every_frame() {
    let es = test_fixtures::read("ps/ffmpeg-mpeg2video-2xac3.a0.ac3");
    let frames = split_ac3_syncframes(&es);
    let first = Ac3SyncframeInfo::from_es(&es).expect("an AC-3 syncframe");
    let sample_rate = first.sample_rate();
    let channels = first.channel_count();
    let frame_len = first.frame_len_bytes().expect("a Table 4.13 frame size");
    let mut total = 0usize;
    for frame in &frames {
        let info = Ac3SyncframeInfo::parse_at(frame, 0).expect("an AC-3 syncframe");
        assert_eq!(info.sample_rate(), sample_rate);
        assert_eq!(info.channel_count(), channels);
        assert_eq!(info.frame_len_bytes(), Some(frame.len()));
        total += frame.len();
    }
    assert_eq!(total, es.len());
    assert_eq!(frames.len(), 58);
    assert_eq!(sample_rate, 44_100);
    assert_eq!(channels, 1, "1/0 mono");
    assert_eq!(frame_len, 834);
}

/// One E-AC-3 elementary stream: every access unit parses, with the same
/// sample rate and channel layout throughout.
#[test]
fn eac3_fixture_parses_every_access_unit() {
    let es = test_fixtures::read("crypto/sample-aes-eac3/clear.ec3");
    let units = split_eac3_syncframes(&es);
    let first = Ec3SyncframeInfo::from_es(&es).expect("an E-AC-3 syncframe");
    let sample_rate = first.sample_rate();
    let channels = first.channel_count();
    let frame_len = (first.frmsiz() as usize + 1) * 2;
    let mut total = 0usize;
    for unit in &units {
        let info = unit.info();
        assert_eq!(info.sample_rate(), sample_rate);
        assert_eq!(info.channel_count(), channels);
        total += unit.data().len();
    }
    assert_eq!(total, es.len());
    assert_eq!(units.len(), 13);
    assert_eq!(sample_rate, 48_000);
    assert_eq!(channels, 2, "2/0 stereo");
    assert_eq!(frame_len, 360);
}

/// The DTS core frames of a transport-stream capture: every frame parses, with
/// the same sample rate and channel count throughout.
#[test]
fn dts_fixture_parses_every_core_frame() {
    let file = test_fixtures::read("ts/dts/dts-core.ts");
    let mut es = Vec::new();
    for payload in pes_payloads(&file).into_values() {
        if DtsCoreFrameInfo::from_es(&payload).is_ok() {
            es.extend_from_slice(&payload);
        }
    }
    let frames = split_dts_core_frames(&es);
    let first = DtsCoreFrameInfo::from_es(&es).expect("a DTS core frame");
    let sample_rate = first.sample_rate();
    let channels = first.channel_count();
    let frame_size = first.frame_size();
    let mut total = 0usize;
    for frame in &frames {
        assert_eq!(frame.data().len(), frame_size);
        assert_eq!(frame.samples(), first.samples_per_frame());
        total += frame.data().len();
    }
    assert_eq!(total, es.len());
    assert_eq!(frames.len(), 188);
    assert_eq!(es.len(), 192_512);
    assert_eq!(sample_rate, 48_000);
    assert_eq!(channels, 2, "2/0 stereo");
    assert_eq!(frame_size, 1_024);
}
