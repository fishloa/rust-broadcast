//! The MPEG-2 video and MPEG audio parsers against real captures, from this
//! crate alone: the sequence header (size, aspect and frame rate) and the
//! sequence display extension a broadcast stream carries, the picture-level AFD
//! in MPEG-2 user data, and an MP3 elementary stream walked as a frame chain.
//!
//! The transport-stream captures are demuxed through `mpeg-ts`/`mpeg-pes` (the
//! crate's dev-dependencies), so the bytes handed to the parsers are the real
//! broadcast payloads.

use broadcast_bitstream::facts::ActiveFormatDescription;
use broadcast_bitstream::h262::{
    afd_from_mpeg2_user_data, find_sequence_header, parse_sequence_at, picture_user_data_payloads,
};
use broadcast_bitstream::mpeg_audio::{MpegAudioFrameHeader, MpegAudioLayer, MpegAudioVersion};
use broadcast_bitstream::mpeg2::Mpeg2SeqHeader;
use broadcast_bitstream::user_data::DTG1_USER_IDENTIFIER;

mod support;

use support::pes_payloads;

/// The elementary stream of the first PID whose payload opens with an MPEG-2
/// `sequence_header()`.
fn mpeg2_elementary_stream(file: &[u8]) -> Vec<u8> {
    pes_payloads(file)
        .into_values()
        .find(|es| Mpeg2SeqHeader::find(es).is_ok())
        .expect("an MPEG-2 video PES payload")
}

/// The AFD the capture's MPEG-2 picture user data states, and how many
/// picture-level `DTG1` payloads it carries.
fn mpeg2_picture_afd(es: &[u8]) -> (usize, Option<Option<ActiveFormatDescription>>) {
    let identifier = DTG1_USER_IDENTIFIER.to_be_bytes();
    let mut carried = 0usize;
    let mut first = None;
    // The walk is per access unit; a whole elementary stream is split into
    // bounded pieces so one picture's payloads stay together.
    for piece in es.chunks(PICTURE_CHUNK_BYTES) {
        let (_, payloads) = picture_user_data_payloads(piece);
        for payload in payloads {
            let Some(body) = payload.strip_prefix(&identifier) else {
                continue;
            };
            carried += 1;
            if first.is_none() {
                first = afd_from_mpeg2_user_data(body);
            }
        }
    }
    (carried, first)
}

/// A bounded slice of an elementary stream, large enough to hold one picture's
/// `extension_and_user_data(2)` sections.
const PICTURE_CHUNK_BYTES: usize = 64 * 1024;

/// The committed MPEG-2 capture: one 320x240 25 Hz 4:3 sequence, no display
/// extension, square samples.
#[test]
fn committed_capture_sequence_header_states_its_geometry() {
    let ts = test_fixtures::read("ts/legacy/mpeg2-mp2.ts");
    let es = mpeg2_elementary_stream(&ts);

    let base = Mpeg2SeqHeader::find(&es).expect("an in-band sequence_header()");
    assert_eq!((base.width(), base.height()), (320, 240));

    let start = find_sequence_header(&es).expect("a sequence_header() offset");
    let geometry = parse_sequence_at(&es, start).expect("sequence geometry");
    assert_eq!(
        (geometry.coded_width(), geometry.coded_height()),
        (320, 240)
    );
    assert_eq!(geometry.aspect_ratio_information(), 2, "Table 6-3 4:3");
    assert_eq!(geometry.frame_rate(), Some((25, 1)), "Table 6-4");
    assert_eq!(geometry.frame_rate_code(), 3);
    assert_eq!(geometry.sample_aspect_ratio(), Some((1, 1)));
    assert_eq!(geometry.display(), None, "no display extension");
}

/// A capture whose `sequence_display_extension()` states the display colour
/// and the intended display rectangle, plus AFD in the picture user data.
#[test]
fn capture_display_extension_and_afd() {
    let Some(ts) = test_fixtures::require_bytes("suites/tsduck/input/test-040.ts", "mpeg2") else {
        return;
    };
    let streams: Vec<Vec<u8>> = pes_payloads(&ts)
        .into_values()
        .filter(|es| Mpeg2SeqHeader::find(es).is_ok())
        .collect();
    assert!(!streams.is_empty(), "the capture carries MPEG-2 video");

    let mut displays = Vec::new();
    let mut afd = None;
    for es in &streams {
        let start = find_sequence_header(es).expect("a sequence_header() offset");
        let geometry = parse_sequence_at(es, start).expect("sequence geometry");
        if let Some(display) = geometry.display() {
            displays.push((
                geometry.coded_width(),
                geometry.coded_height(),
                geometry.aspect_ratio_information(),
                geometry.frame_rate(),
                geometry.sample_aspect_ratio(),
                display.video_format(),
                display.colour_description(),
                display.display_horizontal_size(),
                display.display_vertical_size(),
            ));
        }
        if afd.is_none() {
            let (carried, first) = mpeg2_picture_afd(es);
            if carried != 0 {
                afd = first;
            }
        }
    }
    // The capture carries two distinct sequence display descriptions: an
    // HD 1280x720 59.94 Hz one with BT.709 colour, and an SD 704x480 29.97 Hz
    // one with BT.601 525 colour.
    assert!(
        displays.contains(&(
            1280,
            720,
            3,
            Some((60_000, 1001)),
            Some((1, 1)),
            5,
            Some((1, 1, 1)),
            1280,
            720
        )),
        "the HD display description: {displays:?}"
    );
    assert!(
        displays.contains(&(
            704,
            480,
            2,
            Some((30_000, 1001)),
            Some((10, 11)),
            5,
            Some((6, 6, 6)),
            704,
            480
        )),
        "the SD display description: {displays:?}"
    );
    assert!(
        !displays.is_empty(),
        "a sequence_display_extension() is present"
    );
    assert_eq!(
        afd,
        Some(Some(ActiveFormatDescription::SameAsCodedFrame)),
        "the capture's first signalled picture AFD"
    );
}

/// The committed MP3 fixture walks as one frame chain to its last byte, each
/// frame the same MPEG-1 Layer III header.
#[test]
fn committed_mp3_walks_as_a_frame_chain() {
    let data = test_fixtures::read("audio/audio.mp3");
    // A leading ID3v2 tag carries no audio; its declared length is 45 bytes.
    let start = ID3_TAG_LEN;
    let mut off = start;
    let mut frames = 0usize;
    let mut first = None;
    while off < data.len() {
        let header = MpegAudioFrameHeader::parse(&data[off..]).expect("a frame header");
        if first.is_none() {
            first = Some(header);
        }
        assert_eq!(header.version(), MpegAudioVersion::Mpeg1);
        assert_eq!(header.layer(), MpegAudioLayer::LayerIII);
        assert_eq!(header.bitrate_bps(), 64_000);
        assert_eq!(header.sample_rate(), 48_000);
        assert_eq!(header.channels(), 1);
        assert_eq!(header.samples_per_frame(), 1152);
        let len = header.frame_length();
        assert!(off + len <= data.len(), "frame {frames} fits");
        off += len;
        frames += 1;
    }
    assert_eq!(frames, 44, "44 frames to the last byte");
    assert_eq!(off, data.len(), "the chain accounts for every byte");
}

/// The fixture's ID3v2 tag length (ID3v2 §3.1): 10-byte header + 35-byte body.
const ID3_TAG_LEN: usize = 45;
