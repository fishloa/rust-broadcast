//! The AC-3/E-AC-3 CRC predicate over a real capture's own syncframe:
//! ETSI TS 102 366 §E.2.2 — a syncframe carries a `crc2` word that drives the
//! CRC register to zero, and two bytes cannot hold one.

use broadcast_bitstream::ac3_syncframe::{AC3_SYNCWORD, eac3_frame_crc_ok};
use transmux::pipeline::CodecConfig;

/// The first sample of the committed Dolby E-AC-3 capture: a real syncframe
/// with a real `crc2`.
fn first_real_eac3_frame() -> Vec<u8> {
    let ts = test_fixtures::read("ts/dolby/eac3.ts");
    let media = test_harness::collect::media_from_ts(&ts).expect("demux the capture");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }))
        .expect("an E-AC-3 track");
    track.samples[0].data.to_vec()
}

/// The CRC check must be judged on a payload: a "frame" that is only the
/// syncword has an empty checked region, whose CRC is trivially zero, so it
/// must not verify.
#[test]
fn crc_check_rejects_a_syncword_only_frame() {
    assert!(!eac3_frame_crc_ok(&[]));
    assert!(!eac3_frame_crc_ok(&AC3_SYNCWORD.to_be_bytes()));
    let frame = first_real_eac3_frame();
    assert_eq!(&frame[..2], &AC3_SYNCWORD.to_be_bytes());
    assert!(
        eac3_frame_crc_ok(&frame),
        "the capture's first frame verifies"
    );
}
