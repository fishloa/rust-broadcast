//! Standalone oracle script: tests ffmpeg decryption of both cbcs shapes.
//! Run with: cargo test --test ffmpeg_oracle -- --nocapture

#![cfg(feature = "cenc")]
use broadcast_common::{Encrypt, Package, Unpackage};
use std::path::PathBuf;
use std::process::Command;
use transmux::init_segment::protect_init_segment;
use transmux::movie_fragment::{FragmentProtection, protect_media_segment};
use transmux::{
    CencEncryptor, CencScheme, CmafMux, CodecConfig, ConstantIvSenc, EncryptConfig, IvGen, Media,
    SubsamplePolicy, TrackEncryption,
};

const CBCS_CONSTANT_IV: [u8; 16] = [
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];
const KID: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];
const KEY: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts")
}

fn clear_video_media() -> Option<Media> {
    let path = fixture_path();
    if !path.exists() {
        return None;
    }
    let bytes = std::fs::read(&path).unwrap();
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(bytes.as_slice()).expect("demux");
    Some(
        media
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("avc"),
    )
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_protected_fmp4(media: &mut Media, cfg: &EncryptConfig) -> Vec<u8> {
    CencEncryptor::new(KEY)
        .encrypt(&mut *media, cfg)
        .expect("encrypt");
    let track_id = media.tracks[0].spec.track_id;
    let enc: TrackEncryption = media.tracks[0].encryption.as_ref().expect("enc").clone();
    let raw = CmafMux::new(1).package(&*media).expect("mux");
    let with_protected_init = protect_init_segment(&raw, track_id, &enc).expect("protect init");
    let fp = FragmentProtection {
        track_id,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    protect_media_segment(&with_protected_init, &[fp]).expect("protect segment")
}

fn write_temp(bytes: &[u8], tag: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("ffmpeg_oracle_{}_{}.mp4", tag, std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn ffmpeg_decrypt(in_path: &PathBuf, out_path: &PathBuf, key_hex: &str) -> bool {
    let status = Command::new("ffmpeg")
        .args(["-decryption_key", key_hex, "-i"])
        .arg(in_path)
        .arg("-c")
        .arg("copy")
        .arg("-f")
        .arg("mp4")
        .arg(out_path)
        .arg("-y")
        .status()
        .expect("spawn ffmpeg");
    status.success()
}

#[test]
fn ffmpeg_oracle_emit_shape() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID,
        iv: IvGen::Constant(CBCS_CONSTANT_IV),
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::WholeSample,
        constant_iv_senc: ConstantIvSenc::Emit,
    };
    let protected = build_protected_fmp4(&mut media, &cfg);
    let in_path = write_temp(&protected, "emit");
    let out_path = std::env::temp_dir().join(format!("ffmpeg_emit_out_{}.mp4", std::process::id()));
    let key_hex = to_hex(&KEY);
    let ok = ffmpeg_decrypt(&in_path, &out_path, &key_hex);
    let _ = std::fs::remove_file(&in_path);
    let _ = std::fs::remove_file(&out_path);
    eprintln!(
        "ffmpeg Emit shape decryption: {}",
        if ok { "SUCCESS" } else { "FAILED" }
    );
}

#[test]
fn ffmpeg_oracle_omit_shape() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID,
        iv: IvGen::Constant(CBCS_CONSTANT_IV),
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::WholeSample,
        constant_iv_senc: ConstantIvSenc::Omit,
    };
    let protected = build_protected_fmp4(&mut media, &cfg);
    let in_path = write_temp(&protected, "omit");
    let out_path = std::env::temp_dir().join(format!("ffmpeg_omit_out_{}.mp4", std::process::id()));
    let key_hex = to_hex(&KEY);
    let ok = ffmpeg_decrypt(&in_path, &out_path, &key_hex);
    let _ = std::fs::remove_file(&in_path);
    let _ = std::fs::remove_file(&out_path);
    eprintln!(
        "ffmpeg Omit shape decryption: {}",
        if ok { "SUCCESS" } else { "FAILED" }
    );
}
