//! Oracle consumer: CENC fragmented-MP4 addressing (audit r05-W7,
//! ISO/IEC 14496-12 §8.8.7 / §8.8.8).
//!
//! Consumes `tests/fixtures/cenc_frag_layouts/` — see its `README.md` and
//! `ORACLES.md`. Nine encrypted files cover every combination of the three base
//! rules (`base_data_offset` present / `default-base-is-moof` / neither) and two
//! trun addressing modes (explicit `data_offset` / implicit, continuing after
//! the previous run). Our decryptor must reproduce `clear.mp4`'s samples
//! **byte-for-byte** for all nine.
//!
//! The plaintext comparison is the only oracle for the omit-tfhd-offset files:
//! no independent decryptor reads them correctly (Bento4 mis-bases the later
//! traf, ffmpeg fails the audio) — see the README's "could not be proven".
#![cfg(feature = "cenc")]

use broadcast_common::{Decrypt, Unpackage};
use transmux::cenc_decrypt::{CencDecryptor, KeyMap};
use transmux::{Fmp4Demux, Media};

/// The fixture's KID/KEY (`cenc_frag_layouts/README.md`).
const KID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];
const KEY: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/cenc_frag_layouts"
);

fn read(name: &str) -> Vec<u8> {
    let p = format!("{DIR}/{name}");
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

/// The clear reference: `clear.mp4` demuxed to its video track's samples.
fn clear_video_samples() -> Vec<Vec<u8>> {
    let bytes = read("clear.mp4");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(bytes.as_slice()).expect("demux clear.mp4");
    video_samples(&media)
}

fn video_samples(media: &Media) -> Vec<Vec<u8>> {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
        .map(|t| t.samples.iter().map(|s| s.data.to_vec()).collect())
        .unwrap_or_default()
}

/// Decrypt one `enc_*.mp4` and return its video samples.
fn decrypt_video(name: &str) -> Vec<Vec<u8>> {
    let bytes = read(name);
    let dec = CencDecryptor::from_fmp4(&bytes).unwrap_or_else(|e| panic!("{name}: from_fmp4: {e}"));
    let mut media = dec.demux().unwrap_or_else(|e| panic!("{name}: demux: {e}"));
    let keys = KeyMap::new().with_key(KID, KEY);
    dec.decrypt(&mut media, &keys)
        .unwrap_or_else(|e| panic!("{name}: decrypt: {e}"));
    video_samples(&media)
}

/// All nine layouts must decrypt to the clear video samples byte-for-byte.
#[test]
fn all_layouts_decrypt_to_clear_video_samples() {
    let clear = clear_video_samples();
    assert!(
        !clear.is_empty(),
        "clear.mp4 must carry video samples to compare against"
    );
    assert_eq!(clear.len(), 20, "fixture has 20 video samples");

    let files = [
        "enc_default_none.mp4",
        "enc_default_explicit.mp4",
        "enc_default_implicit.mp4",
        "enc_base_moof_none.mp4",
        "enc_base_moof_explicit.mp4",
        "enc_base_moof_implicit.mp4",
        "enc_omit_none.mp4",
        "enc_omit_explicit.mp4",
        "enc_omit_implicit.mp4",
    ];
    for f in files {
        let got = decrypt_video(f);
        assert_eq!(
            got.len(),
            clear.len(),
            "{f}: sample count must match the clear reference"
        );
        for (i, (a, b)) in got.iter().zip(clear.iter()).enumerate() {
            assert_eq!(
                a, b,
                "{f}: sample {i} must decrypt to the clear plaintext byte-for-byte"
            );
        }
    }
}
