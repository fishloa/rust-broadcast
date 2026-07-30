//! Oracle: checks whether ffmpeg can decrypt our `cbcs` output.
//!
//! Spoiler: it cannot.  The `-decryption_key` flag exists and exits 0, but
//! even with actual decoding (no `-c copy`), the output is a 262-byte empty
//! file with no `mdat` and no video track — for both shapes.  We record
//! this negative result honestly rather than silently treating exit-0 as
//! success.  If ffmpeg gains `cbcs` support, this test will fail
//! assertively with `!has_video`, telling us to update the interop table.

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

/// Try ffmpeg decryption with `-decryption_key` and actual decoding (no
/// `-c copy` — that stream-copies the encrypted bitstream without
/// decrypting it).  Returns (exit_ok, output_size, has_mdat_box).
fn ffmpeg_try_decrypt(in_path: &PathBuf, out_path: &PathBuf, key_hex: &str) -> (bool, u64, bool) {
    let output = Command::new("ffmpeg")
        .args(["-decryption_key", key_hex, "-i"])
        .arg(in_path)
        .arg("-f")
        .arg("mp4")
        .arg("-y")
        .arg(out_path)
        .output()
        .expect("spawn ffmpeg");
    let ok = output.status.success();
    let size = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    let has_mdat = if size > 1000 {
        let bytes = std::fs::read(out_path).unwrap_or_default();
        let mut off = 0usize;
        let mut found = false;
        while off + 8 <= bytes.len() {
            let sz =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            if sz < 8 || off + sz > bytes.len() {
                break;
            }
            if &bytes[off + 4..off + 8] == b"mdat" {
                found = true;
                break;
            }
            off += sz;
        }
        found
    } else {
        false
    };
    (ok, size, has_mdat)
}

/// ffmpeg 8.1.2 cannot decrypt our `cbcs` (AES-CBC pattern) output.
/// `-decryption_key` exits 0 but produces a ~262-byte empty file with
/// no `mdat` box and no video track.  This is identical for both shapes.
/// We assert the negative result so a future ffmpeg that *can* decrypt
/// `cbcs` fails this test and tells us to update the interop table.
#[test]
fn ffmpeg_cannot_decrypt_cbcs() {
    let Some(media) = clear_video_media() else {
        return;
    };

    for (tag, constant_iv_senc) in [
        ("emit", ConstantIvSenc::Emit),
        ("omit", ConstantIvSenc::Omit),
    ] {
        let mut m = media.clone();
        let cfg = EncryptConfig {
            scheme: CencScheme::Cbcs,
            kid: KID,
            iv: IvGen::Constant(CBCS_CONSTANT_IV),
            pattern: Some((1, 9)),
            subsample: SubsamplePolicy::WholeSample,
            constant_iv_senc,
        };
        let protected = build_protected_fmp4(&mut m, &cfg);
        let in_path = write_temp(&protected, tag);
        let out_path = std::env::temp_dir().join(format!(
            "ffmpeg_cbcs_{}_out_{}.mp4",
            tag,
            std::process::id()
        ));
        let key_hex = to_hex(&KEY);
        let (ok, size, has_mdat) = ffmpeg_try_decrypt(&in_path, &out_path, &key_hex);
        let _ = std::fs::remove_file(&in_path);
        let _ = std::fs::remove_file(&out_path);

        assert!(
            !has_mdat,
            "ffmpeg {tag}: expected NO video track (ffmpeg cannot decrypt cbcs), \
             but found an mdat box (output_size={size}, exit_ok={ok}) — did ffmpeg \
             gain cbcs support? Update the interop table and this test."
        );
        eprintln!(
            "ffmpeg {tag}: exit=0 output_size={size} has_mdat={has_mdat} \
             -> confirmed negative oracle: ffmpeg cannot decrypt cbcs"
        );
    }
}
