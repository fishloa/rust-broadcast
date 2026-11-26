#![no_main]

use alloc::sync::Arc;
use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;
use isobmff::cenc::{SubSampleEntry, TrackEncryptionBox};
use libfuzzer_sys::fuzz_target;
use transmux::cenc_decrypt::{CencDecryptStage, KeyMap};
use transmux::ir::{ConstantIvSenc, SampleProtection, TrackProtection};
use transmux::mux_stage::MuxInput;
use transmux::{CencScheme, CodecConfig, Sample, TrackId, TrackSpec};

extern crate alloc;

// Fuzz the per-sample CENC decrypt stage: arbitrary `tenc`/IV/subsample
// metadata must never panic; an IV that disagrees with the track's `tenc`
// must be a typed error, never a cipher run with the wrong IV.
fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let kid = [data[0]; 16];
    let key = [data[1]; 16];
    let scheme = if data[2] & 1 == 0 {
        CencScheme::Cenc
    } else {
        CencScheme::Cbcs
    };
    let iv_size = data[3];
    let constant_iv = if data[4] & 1 == 0 {
        None
    } else {
        Some(data[5..].to_vec())
    };
    let tenc = TrackEncryptionBox {
        version: 0,
        default_crypt_byte_block: data[6] & 0x0f,
        default_skip_byte_block: data[7] & 0x0f,
        default_is_protected: 1,
        default_per_sample_iv_size: iv_size,
        default_kid: kid,
        default_constant_iv: constant_iv,
    };
    let mut spec = TrackSpec::new(
        TrackId::new(1),
        broadcast_core::timescale::Timescale::try_new(90_000).expect("nonzero"),
        CodecConfig::Avc {
            config: isobmff::AVCConfigurationBox::new(isobmff::AVCDecoderConfigurationRecord {
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
            }),
            width: 0,
            height: 0,
        },
    );
    spec.protection = Some(TrackProtection::new(
        scheme,
        tenc,
        ConstantIvSenc::default(),
    ));
    let mut stage = CencDecryptStage::new(KeyMap::new().with_key(kid, key));
    let _ = stage.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO);
    let mut sample = match Sample::new(data.to_vec(), Some(0), Some(0), Some(1), true) {
        Ok(sample) => sample,
        Err(_) => return,
    };
    let rest = &data[8..];
    let subsamples = if rest.len() >= 6 {
        let clear = u16::from_be_bytes([rest[0], rest[1]]);
        let protected = u32::from_be_bytes([rest[2], rest[3], rest[4], rest[5]]);
        vec![SubSampleEntry {
            bytes_of_clear_data: clear,
            bytes_of_protected_data: protected,
        }]
    } else {
        Vec::new()
    };
    sample.protection = Some(Arc::new(SampleProtection {
        iv: Bytes::copy_from_slice(rest.get(6..6 + usize::from(iv_size)).unwrap_or(&[])),
        subsamples,
    }));
    let _ = stage.feed(
        MuxInput::Sample {
            track: TrackId::new(1),
            sample,
        },
        Timestamp::ZERO,
    );
    let _ = stage.finish();
    while stage.poll().is_some() {}
});
