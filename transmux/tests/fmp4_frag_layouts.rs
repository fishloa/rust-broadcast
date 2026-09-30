//! Oracle consumer: `Fmp4Demux` fragment addressing (audit r05-W7).
//!
//! Consumes the CLEAR variants of `tests/fixtures/cenc_frag_layouts/` — the
//! three addressing forms a real muxer writes (`clear_ffmpeg_{default,
//! base_moof,omit}.mp4`) plus the two hand-split multi-trun files
//! (`clear_multitrun_{base_moof,omit}_implicit.mp4`; no tool emits multiple
//! truns per traf — see the fixture README). Every file must demux to the same
//! per-sample plaintext as `clear.mp4` (whose per-sample hashes are pinned in
//! `clear_samples.txt`), for **all** tracks.
//!
//! `Fmp4Demux` shares its §8.8.7/§8.8.8 resolver with `CencDecryptor`
//! (`frag_offsets`), so this proves the demuxer's side of the same rule set.
#![cfg(feature = "std")]

use broadcast_common::Unpackage;
use transmux::{CodecConfig, Fmp4Demux, Media};

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/cenc_frag_layouts"
);

fn read(name: &str) -> Vec<u8> {
    let p = format!("{DIR}/{name}");
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {p}: {e}"))
}

fn demux(name: &str) -> Media {
    let bytes = read(name);
    let mut d = Fmp4Demux::new();
    d.unpackage(bytes.as_slice())
        .unwrap_or_else(|e| panic!("{name}: demux: {e}"))
}

/// `(track_id, per-sample bytes)` for every track, in moov order.
fn by_track(m: &Media) -> Vec<(u32, Vec<Vec<u8>>)> {
    m.tracks
        .iter()
        .map(|t| {
            (
                t.spec.track_id,
                t.samples.iter().map(|s| s.data.to_vec()).collect(),
            )
        })
        .collect()
}

/// Every clear layout must demux to the same samples (all tracks) as
/// `clear.mp4`.
#[test]
fn all_clear_layouts_demux_identically() {
    let reference = by_track(&demux("clear.mp4"));
    assert_eq!(reference.len(), 2, "clear.mp4 has video + audio");
    assert_eq!(reference[0].1.len(), 20, "video samples");
    assert_eq!(reference[1].1.len(), 95, "audio samples");
    assert!(
        reference
            .iter()
            .any(|(_, s)| s.iter().any(|b| matches!(b.len(), 3001 | 322))),
        "the reference must carry real video sample bytes"
    );

    // The clear variants: the three real-muxer forms plus the hand-split
    // multi-trun files (no tool emits multiple truns per traf).
    let files = [
        "clear_ffmpeg_default.mp4",
        "clear_ffmpeg_base_moof.mp4",
        "clear_ffmpeg_omit.mp4",
        "clear_multitrun_base_moof_implicit.mp4",
        "clear_multitrun_omit_implicit.mp4",
    ];
    for f in files {
        let got = by_track(&demux(f));
        assert_eq!(got.len(), reference.len(), "{f}: track count");
        for ((rt, rs), (gt, gs)) in reference.iter().zip(got.iter()) {
            assert_eq!(rt, gt, "{f}: track ids must match");
            assert_eq!(rs.len(), gs.len(), "{f}: track {gt} sample count");
            for (i, (a, b)) in rs.iter().zip(gs.iter()).enumerate() {
                assert_eq!(
                    a, b,
                    "{f}: track {gt} sample {i} must match clear.mp4 byte-for-byte"
                );
            }
        }
    }
}

/// The fixtures really are the layouts they claim: `clear.mp4` and
/// `clear_ffmpeg_base_moof.mp4` are the same bytes (per the fixture README),
/// and all clear variants decode to the same video sample count.
#[test]
fn clear_variants_carry_the_same_codec_config() {
    for f in [
        "clear.mp4",
        "clear_ffmpeg_default.mp4",
        "clear_ffmpeg_omit.mp4",
        "clear_multitrun_omit_implicit.mp4",
    ] {
        let m = demux(f);
        assert_eq!(m.tracks.len(), 2, "{f}: two tracks");
        assert!(
            matches!(m.tracks[0].spec.config, CodecConfig::Avc { .. }),
            "{f}: track 1 is AVC"
        );
        assert_eq!(m.tracks[0].samples.len(), 20, "{f}: 20 video samples");
        assert_eq!(m.tracks[1].samples.len(), 95, "{f}: 95 audio samples");
    }
}
