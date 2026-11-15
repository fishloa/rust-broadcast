//! #2746: `PES_packet_length` was a free field written verbatim from the
//! struct rather than from the content, a payload over its 16-bit ceiling was
//! not representable, and a buffer shorter than the declared length parsed as
//! a whole packet.

use broadcast_common::Serialize;
use mpeg_pes::{PesHeader, PesPacket, StreamId};

fn packet<'a>(header: Option<PesHeader<'a>>, payload: &'a [u8]) -> PesPacket<'a> {
    let mut pes = PesPacket::new(StreamId::new(0xE0), payload);
    pes.header = header;
    pes
}

#[test]
fn the_length_written_is_the_length_of_the_content() {
    let bytes = packet(Some(PesHeader::new()), &[0xAA, 0xBB])
        .try_to_bytes()
        .expect("a small packet serializes");
    assert_eq!(
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::try_from(bytes.len() - 6).expect("fits"),
        "PES_packet_length covers everything after it"
    );
    let parsed = PesPacket::parse(&bytes).expect("own output parses");
    assert_eq!(parsed.payload, &[0xAA, 0xBB]);
}

#[test]
fn a_payload_past_the_16_bit_length_ceiling_is_bounded() {
    let payload = vec![0x5A; 70_000];
    // Video may use the unbounded (`0`) form (ISO/IEC 13818-1 §2.4.3.7).
    let video = packet(Some(PesHeader::new()), &payload).try_to_bytes();
    let video = video.expect("a video packet past the ceiling is unbounded");
    assert_eq!(u16::from_be_bytes([video[4], video[5]]), 0);
    // Audio may not, and the length must not wrap to a smaller value.
    let mut audio = packet(Some(PesHeader::new()), &payload);
    audio.stream_id = StreamId::new(0xC0);
    assert!(matches!(
        audio.try_to_bytes(),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_common::len::FieldOverflow {
                field: "PES_packet_length",
                ..
            }
        ))
    ));
}

#[test]
fn a_buffer_shorter_than_the_declared_length_is_rejected() {
    let bytes = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01, 0xAA,
        0xBB,
    ];
    PesPacket::parse(&bytes).expect("the whole packet parses");
    // Drop the last payload byte: the declared length no longer fits.
    let short = &bytes[..bytes.len() - 1];
    assert_eq!(
        PesPacket::parse(short).unwrap_err(),
        mpeg_pes::Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: bytes.len(),
            have: short.len(),
            what: "PES_packet_length",
        }),
        "a truncated packet is reported, not silently shortened"
    );
}
