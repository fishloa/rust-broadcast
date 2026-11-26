#![no_main]

use broadcast_common::stage::{Stage, Timestamp};
use libfuzzer_sys::fuzz_target;
use transmux::cenc_encrypt::{CencEncryptStage, EncryptConfig, IvGen, SubsamplePolicy};
use transmux::mux_stage::MuxInput;
use transmux::{
    CencScheme, CodecConfig, Sample, TrackId, TrackSpec,
};

// Fuzz the per-sample CENC encrypt stage: arbitrary track configuration,
// sample bytes and IV list must never panic, and a rejected sample must not
// exhaust the IV counter.
fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    let key = [data[0]; 16];
    let kid = [data[1]; 16];
    let scheme = if data[2] & 1 == 0 {
        CencScheme::Cenc
    } else {
        CencScheme::Cbcs
    };
    let policy = if data[3] & 1 == 0 {
        SubsamplePolicy::Video
    } else {
        SubsamplePolicy::WholeSample
    };
    let mut cfg = EncryptConfig::new(scheme, kid, policy);
    match data[4] % 3 {
        0 => cfg = cfg.with_iv(IvGen::Counter),
        1 => cfg = cfg.with_iv(IvGen::Constant([data[5]; 16])),
        _ => {
            let mut ivs = Vec::new();
            for chunk in data[6..].chunks(8) {
                ivs.push(chunk.to_vec());
            }
            cfg = cfg.with_iv(IvGen::Explicit(ivs));
        }
    }
    let Ok(mut stage) = CencEncryptStage::new(key, cfg) else {
        return;
    };
    let record = isobmff::AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 0,
        profile_compatibility: 0,
        level_indication: 0,
        length_size_minus_one: 3,
        sps: Vec::new(),
        pps: Vec::new(),
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    let spec = TrackSpec::new(
        TrackId::new(1),
        broadcast_core::timescale::Timescale::try_new(90_000).expect("nonzero"),
        CodecConfig::Avc {
            config: isobmff::AVCConfigurationBox::new(record),
            width: 0,
            height: 0,
        },
    );
    let _ = stage.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO);
    let sample = Sample::new(data.to_vec(), Some(0), Some(0), Some(1), true);
    if let Ok(sample) = sample {
        let _ = stage.feed(
            MuxInput::Sample {
                track: TrackId::new(1),
                sample,
            },
            Timestamp::ZERO,
        );
    }
    let _ = stage.finish();
    while stage.poll().is_some() {}
});
