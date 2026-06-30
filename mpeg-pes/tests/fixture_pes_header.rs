//! Real-fixture **byte-identical** round-trip proof for the PES optional header,
//! including its `PES_header_data_length` `0xFF` stuffing
//! (ISO/IEC 13818-1:2007 §2.4.3.7).
//!
//! The whole PES header block (`packet_start_code_prefix` … end of the
//! `PES_header_data_length` region) must round-trip parse → serialize →
//! byte-identical, the workspace's hard wire-codec invariant.
//! `PesHeader::header_stuffing_len` captures the trailing stuffing so the
//! serializer reproduces the header exactly.
//!
//! ## Fixture: `pes-pts-dts-stuffing.ts`
//! Extracted from TSDuck test vector `test-001.ts` (video, `stream_id` `0xE0`):
//! - a **PTS + DTS + stuffing** header (`PES_header_data_length` = 15: 10 bytes
//!   of PTS/DTS + 5 `0xFF` stuffing bytes);
//! - a **PTS-only + stuffing** header (`PES_header_data_length` = 15: 5 bytes of
//!   PTS + 10 `0xFF` stuffing bytes).
//!
//! Both round-trip byte-identical, proving PTS, DTS, and header stuffing on real
//! broadcast bytes.
//!
//! ## Fields proven only synthetically (in-crate unit tests)
//! `escr`, `es_rate`, `dsm_trick_mode`, `additional_copy_info`, `pes_crc`, and
//! `pes_extension` do **not** occur in genuine, byte-identical-roundtrippable
//! form in any available capture: the France TNT and Hot Bird DVB muxes contain
//! **zero** PES headers setting any of these flags (real broadcast PES headers
//! carry only PTS/DTS), and the TSDuck "candidates" are mis-stream-typed bytes
//! that never round-trip byte-identical. These fields are covered by the
//! build-from-fields round-trip unit tests in `mpeg-pes/src/packet.rs`.

use mpeg_pes::{PesPacket, StreamId};
use std::fs;

const TS: usize = 188;

fn fixture(name: &str) -> Vec<u8> {
    fs::read(format!(
        "{}/../fixtures/mpeg-pes/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    ))
    .unwrap_or_else(|e| panic!("fixture {name} must be present: {e}"))
}

/// PES payload of a PUSI TS packet (these fixture packets are payload-only,
/// `adaptation_field_control == 1`).
fn pes_payload(pkt: &[u8]) -> &[u8] {
    assert_eq!(pkt.len(), TS);
    assert!(pkt[1] & 0x40 != 0, "fixture packet must have PUSI set");
    assert_eq!(
        (pkt[3] & 0x30) >> 4,
        1,
        "fixture packet must be payload-only"
    );
    &pkt[4..]
}

/// The PES header block length: 6-byte fixed prefix + 3 header bytes + the
/// `PES_header_data_length` optional/stuffing region.
fn header_block_len(payload: &[u8]) -> usize {
    9 + payload[8] as usize
}

#[test]
fn pes_pts_dts_and_stuffing_byte_identical() {
    let buf = fixture("pes-pts-dts-stuffing.ts");
    assert_eq!(buf.len() % TS, 0);
    let pkts: Vec<&[u8]> = buf.chunks_exact(TS).collect();
    assert!(pkts.len() >= 2, "fixture must hold >=2 packets");

    let mut pts_only_stuffed = 0usize;
    let mut pts_dts_stuffed = 0usize;

    for (i, pkt) in pkts.iter().enumerate() {
        let payload = pes_payload(pkt);
        assert_eq!(
            &payload[0..3],
            &[0x00, 0x00, 0x01],
            "pkt {i}: PES start code"
        );

        let pes = PesPacket::parse(payload).expect("PES parses");
        let hdr = pes.header.as_ref().expect("optional header present");

        // PTS is always present in this fixture.
        assert!(hdr.pts.is_some(), "pkt {i}: PTS must be Some");

        // Real header stuffing must be captured.
        assert!(
            hdr.header_stuffing_len > 0,
            "pkt {i}: this fixture's headers carry stuffing"
        );

        if hdr.dts.is_some() {
            pts_dts_stuffed += 1;
        } else {
            pts_only_stuffed += 1;
        }

        // Byte-identical over the entire PES header block (incl. stuffing).
        let block = header_block_len(payload);
        let mut out = vec![0u8; pes.serialized_len()];
        let n = pes.serialize_into(&mut out).expect("serialize");
        assert!(n >= block);
        assert_eq!(
            &out[..block],
            &payload[..block],
            "pkt {i}: PES header block NOT byte-identical\n  orig: {}\n  out:  {}",
            payload[..block]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>(),
            out[..block]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>(),
        );
    }

    assert!(
        pts_dts_stuffed >= 1,
        "expected a PTS+DTS+stuffing header, got {pts_dts_stuffed}"
    );
    assert!(
        pts_only_stuffed >= 1,
        "expected a PTS-only+stuffing header, got {pts_only_stuffed}"
    );
}

/// The fixture packets are video PES (`stream_id` `0xE0`) — confirms stream_id
/// decode and that we parsed the intended fixture.
#[test]
fn fixture_is_video_pes() {
    let buf = fixture("pes-pts-dts-stuffing.ts");
    let mut count = 0usize;
    for pkt in buf.chunks_exact(TS) {
        let pes = PesPacket::parse(pes_payload(pkt)).expect("PES parses");
        assert_eq!(
            pes.stream_id,
            StreamId(0xE0),
            "expected video stream_id 0xE0"
        );
        count += 1;
    }
    assert!(count >= 2);
}
