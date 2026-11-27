#![no_main]

use broadcast_bitstream::mpeg_audio::{MpegAudioFrameHeader, MpegAudioLayer, MpegAudioVersion};
use libfuzzer_sys::fuzz_target;

// Fuzz the MPEG-1/2/2.5 audio frame header parser on arbitrary bytes: an
// accepted header must describe a frame at least as long as its own four-byte
// header, with a stated (non-zero) rate and bitrate, and its layer / version
// accessors must agree with a second parse.
fuzz_target!(|data: &[u8]| {
    let Ok(hdr) = MpegAudioFrameHeader::parse(data) else {
        return;
    };
    assert!(hdr.frame_length() >= core::mem::size_of::<u32>());
    assert!(hdr.sample_rate() != 0);
    assert!(hdr.bitrate_bps() != 0);
    assert!(matches!(hdr.channels(), 1 | 2));
    assert!(matches!(
        hdr.layer(),
        MpegAudioLayer::LayerI | MpegAudioLayer::LayerII | MpegAudioLayer::LayerIII
    ));
    assert!(matches!(
        hdr.version(),
        MpegAudioVersion::Mpeg1 | MpegAudioVersion::Mpeg2 | MpegAudioVersion::Mpeg25
    ));
    // The header fields are a pure function of the first four bytes.
    let again = MpegAudioFrameHeader::parse(&data[..4]).expect("an accepted header re-parses");
    assert_eq!(again, hdr);
});
