#![no_main]

use broadcast_common::Serialize;
use libfuzzer_sys::fuzz_target;
use transmux::{CodecConfig, DemuxEvent};

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;

/// Issue #1171 contract: every config the demuxer hands a consumer must
/// serialize — `to_bytes()` panics on a `FieldOverflow`, so a demuxer-built
/// value that fails `try_to_bytes()` is a bug in the demuxer, not the caller.
fn assert_config_serializes(cfg: &CodecConfig) {
    macro_rules! ok {
        ($b:expr) => {
            assert!($b.try_to_bytes().is_ok(), "demuxer emitted an unserializable config")
        };
    }
    match cfg {
        CodecConfig::Avc { config, .. } => ok!(config),
        CodecConfig::Hevc { config, .. } => ok!(config),
        CodecConfig::Aac { esds, .. }
        | CodecConfig::Mpeg2Video { esds, .. }
        | CodecConfig::MpegAudio { esds, .. } => ok!(esds),
        CodecConfig::Ac3 { config, .. } => ok!(config),
        CodecConfig::Eac3 { config, .. } => ok!(config),
        CodecConfig::Dts { config, .. } => ok!(config),
        CodecConfig::MpegH { config, .. } => ok!(config),
        _ => {}
    }
}

fn drain(d: &mut transmux::StreamingTsDemux) {
    while let Some(ev) = d.poll_event() {
        if let DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) = &ev {
            assert_config_serializes(&spec.config);
        }
    }
}

// Fuzz `StreamingTsDemux` with well-formed-ish 188-byte packets (T5): each
// chunk's sync byte is forced to `0x47` so the packet-boundary resync in
// `mpeg_ts::resync` locks immediately, letting the fuzzer spend its budget on
// what's actually new on this branch — `parse_pat`, `parse_pmt_section_header`,
// the PMT-diff state machine (add/remove/reclassify a stream across PMT
// versions), and the new CRC path — rather than mostly re-discovering "find
// the next 0x47". The remaining 187 bytes of each packet (PID, flags,
// continuity counter, adaptation field, PSI/PES payload) stay fully
// arbitrary, so PAT/PMT section bytes, CRC words, and PES headers are all
// fuzzer-controlled.
fuzz_target!(|data: &[u8]| {
    let mut d = transmux::StreamingTsDemux::new();
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    for chunk in data.chunks(TS_PACKET_SIZE - 1) {
        packet[0] = SYNC_BYTE;
        packet[1..1 + chunk.len()].copy_from_slice(chunk);
        // Any bytes beyond a short final chunk stay `0xFF` stuffing from the
        // previous iteration's leftovers being overwritten in place; zero the
        // tail explicitly so no fuzz iteration ever cross-contaminates the
        // next with left-over data from a longer prior chunk.
        for b in packet[1 + chunk.len()..].iter_mut() {
            *b = 0xFF;
        }
        d.feed(&packet);
        drain(&mut d);
    }
    d.finish();
    drain(&mut d);
});
