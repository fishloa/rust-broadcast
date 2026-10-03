//! Before/after output-equality guard for the transmux optimization sweep
//! (#1079/#1080/#1081). Each refactor of a demux/mux output path must leave the
//! output byte-identical on the committed real fixtures. The expected values
//! below were captured from the pre-refactor code (FNV-1a 64 over the full
//! `Debug` rendering of the produced IR / bytes), so a changed byte anywhere
//! fails here.
#![cfg(feature = "std")]

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::{PsDemux, TsDemux, WebmDemux};

fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes
        .iter()
        .fold(OFFSET, |h, &b| (h ^ u64::from(b)).wrapping_mul(PRIME))
}

fn fixture(rel: &str) -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(rel);
    std::fs::read(p).ok()
}

/// Record a mismatch (every fixture is reported, not just the first).
fn check(bad: &mut Vec<String>, name: &str, got: u64, want: u64) {
    eprintln!("golden {name}: {got:#018x}");
    if got != want {
        bad.push(format!("{name}: {got:#018x} != {want:#018x}"));
    }
}

const PS_FIXTURES: &[(&str, u64)] = &[
    ("ps/h264_ac3.ps", 0xe24863d1ee914de7),
    ("ps/ffmpeg-h264-noaud.ps", 0x0f41a64caef35ce5),
    ("ps/ffmpeg-mpeg2video-mp2.ps", 0x8310cf8cecaa56f7),
    ("ps/ffmpeg-mpeg2video-2xac3.ps", 0xa7dbc0815658589c),
    ("ps/ffmpeg-ac3-dts.ps", 0x4f5d8d8a7f521ba2),
    ("mpeg-ps/ffmpeg-mpeg2-ps.mpg", 0x008239034562de4c),
];

#[test]
fn ps_demux_output_is_unchanged() {
    let mut bad = Vec::new();
    for &(rel, want) in PS_FIXTURES {
        let Some(bytes) = fixture(rel) else {
            eprintln!("SKIPPED {rel}: fixture missing");
            continue;
        };
        let media = PsDemux::new().unpackage(bytes.as_slice()).expect(rel);
        check(&mut bad, rel, fnv1a(format!("{media:?}").as_bytes()), want);
    }
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

// Note: `webm/vp9_opus.webm` and `mkv/vp9_opus.mkv` hash identically on purpose.
// The two files carry the same VP9 + Opus content (same encode, remuxed between
// the WebM and Matroska container flavours), and the demuxer's `Media` IR (which
// is what is hashed) holds codec config + samples + timing only, nothing about
// the container flavour, so the `Debug` rendering is genuinely identical.
const WEBM_FIXTURES: &[(&str, u64)] = &[
    ("webm/vp9_opus.webm", 0xfec110a63d715331),
    ("webm/vp8_opus.webm", 0x44663f064ecc766b),
    ("webm/vorbis.webm", 0x67143f51c09cf65e),
    ("mkv/h264_aac.mkv", 0x9847beadbf70bead),
    ("mkv/vp9_opus.mkv", 0xfec110a63d715331),
    ("mkv/hevc_aac.mkv", 0xd77d3469e7e72cf3),
];

#[test]
fn webm_demux_output_is_unchanged() {
    let mut bad = Vec::new();
    for &(rel, want) in WEBM_FIXTURES {
        let Some(bytes) = fixture(rel) else {
            eprintln!("SKIPPED {rel}: fixture missing");
            continue;
        };
        let media = WebmDemux::new().unpackage(bytes.as_slice()).expect(rel);
        check(&mut bad, rel, fnv1a(format!("{media:?}").as_bytes()), want);
    }
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

const TS_FIXTURES: &[(&str, u64)] = &[
    ("ts/h264/main.ts", 0xe8a04393ee137ee8),
    ("ts/h264_aac.ts", 0x51871212618d1ded),
    ("ts/m6-single.ts", 0x826d70c4fe6996f2),
    ("ts/gulli-opengop.ts", 0x322faa14c5c28795),
    ("ts/pts-wrap.ts", 0x446b6222520c1313),
    ("ts/aac-5_1-640k.ts", 0x0c69bd29aae724c7),
    ("ts/dts/dts_core.ts", 0x049622d2b28d2388),
    ("ts/legacy/mpeg2_mp2_pes_misaligned.ts", 0x9cde22b83f9dd6f2),
];

#[test]
fn ts_demux_output_is_unchanged() {
    let mut bad = Vec::new();
    for &(rel, want) in TS_FIXTURES {
        let Some(bytes) = fixture(rel) else {
            eprintln!("SKIPPED {rel}: fixture missing");
            continue;
        };
        let media = TsDemux::new().unpackage(bytes.as_slice()).expect(rel);
        check(&mut bad, rel, fnv1a(format!("{media:?}").as_bytes()), want);
    }
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

#[test]
fn rtmp_read_chunks_output_is_unchanged() {
    use transmux::rtmp::{Message, read_chunks, write_chunks};
    // Several csids interleaved, mixed sizes (some > the 128-byte chunk size,
    // some exact multiples), identical headers so fmt 1/2/3 compression is used.
    let msgs: Vec<Message> = (0..40u32)
        .map(|i| Message {
            csid: [3, 4, 6][(i % 3) as usize],
            message_type_id: [20, 8, 9][(i % 3) as usize],
            message_stream_id: 1,
            timestamp: i * 20,
            body: (0..([5usize, 128, 129, 1000, 256][(i % 5) as usize]))
                .map(|b| (b as u32 ^ i) as u8)
                .collect(),
        })
        .collect();
    let wire = write_chunks(&msgs, 128).unwrap();
    let out = read_chunks(&wire).unwrap();
    assert_eq!(format!("{out:?}"), format!("{msgs:?}"));
    let mut bad = Vec::new();
    check(
        &mut bad,
        "rtmp-mixed",
        fnv1a(format!("{out:?}").as_bytes()),
        0xf067_0165_ca0c_495a,
    );
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

#[test]
fn rtp_stream_depacketiser_output_is_unchanged() {
    use broadcast_common::Package;
    use transmux::pipeline::CodecConfig;
    use transmux::rtp::RtpMediaKind;
    use transmux::{RtpPacketiser, RtpStreamDepacketiser, RtpStreamTrack};
    let Some(data) = fixture("ts/h264_aac.ts") else {
        eprintln!("SKIPPED ts/h264_aac.ts");
        return;
    };
    let media = TsDemux::new().unpackage(data.as_slice()).unwrap();
    let video_only = media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .unwrap();
    let config = video_only.tracks[0].spec.config.clone();
    let mut pk = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    };
    let out = pk.package(&video_only).unwrap();
    let stream = out
        .streams
        .iter()
        .find(|s| s.kind == RtpMediaKind::H264)
        .unwrap();
    let mut d = RtpStreamDepacketiser::new(vec![RtpStreamTrack::new(
        1,
        RtpMediaKind::H264,
        config,
        90_000,
    )]);
    let mut samples = Vec::new();
    for p in &stream.packets {
        samples.extend(d.push(1, &p.as_contiguous()).unwrap());
    }
    samples.extend(d.flush(1).unwrap());
    let mut bad = Vec::new();
    check(
        &mut bad,
        "rtp-stream",
        fnv1a(format!("{samples:?}").as_bytes()),
        0x0dec_cfda_9ae3_7ce8,
    );
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

fn h264_aac_media() -> Option<transmux::Media> {
    let data = fixture("ts/h264_aac.ts")?;
    Some(TsDemux::new().unpackage(data.as_slice()).unwrap())
}

#[test]
fn muxer_outputs_are_unchanged() {
    use broadcast_common::Package;
    use transmux::{MkvMux, ProgressiveMux, TsMux};
    let Some(media) = h264_aac_media() else {
        eprintln!("SKIPPED ts/h264_aac.ts");
        return;
    };
    let mut bad = Vec::new();
    let faststart = ProgressiveMux::new(true).package(&media).unwrap();
    check(
        &mut bad,
        "progressive-faststart",
        fnv1a(&faststart),
        0x0d60444d4345e766,
    );
    let tail = ProgressiveMux::new(false).package(&media).unwrap();
    check(
        &mut bad,
        "progressive-moov-last",
        fnv1a(&tail),
        0x88ca21e79f4aa26c,
    );
    let mkv = MkvMux::new().package(&media).unwrap();
    check(&mut bad, "mkv", fnv1a(&mkv), 0xf3db485ac205dbe1);
    let ts = TsMux::new().package(&media).unwrap();
    check(&mut bad, "ts", fnv1a(&ts), 0xe5b08f3fd235cf03);
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

#[test]
fn fragment_writer_outputs_are_unchanged() {
    use broadcast_common::Package;
    use transmux::{CmafMux, LlSegmenter, SmoothPackager};
    let Some(media) = h264_aac_media() else {
        eprintln!("SKIPPED ts/h264_aac.ts");
        return;
    };
    let mut bad = Vec::new();
    let cmaf = CmafMux::new(1).package(&media).unwrap();
    check(&mut bad, "cmaf", fnv1a(&cmaf), 0xfbf4_2a4d_ad3f_5ea0);
    let smooth = SmoothPackager::default().package(&media).unwrap();
    check(
        &mut bad,
        "smooth",
        fnv1a(format!("{smooth:?}").as_bytes()),
        0xb843_b6bc_d7f7_ee94,
    );
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut ll = LlSegmenter::new(specs, 1000, 2.0, 5).unwrap();
    let chunks = ll.package(&media).unwrap();
    check(
        &mut bad,
        "ll-dash-chunks",
        fnv1a(format!("{chunks:?}").as_bytes()),
        0x0d3e_616c_3bcb_42c7,
    );
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

/// FNV-1a of an output that is bytes (hashed as bytes), or of the error text when
/// the muxer rejects the input (so a changed rejection is also caught).
fn hash_result<E: std::fmt::Display>(r: Result<Vec<u8>, E>) -> u64 {
    match r {
        Ok(bytes) => fnv1a(&bytes),
        Err(e) => fnv1a(format!("ERR:{e}").as_bytes()),
    }
}

/// Muxer / fragment-writer bytes on HEVC and (E-)AC-3 fixtures.
const CODEC_MUX_FIXTURES: &[&str] = &["ts/hevc/main.ts", "ts/dolby/ac3.ts", "ts/dolby/eac3.ts"];

#[test]
fn hevc_and_dolby_muxer_outputs_are_unchanged() {
    use broadcast_common::Package;
    use transmux::{CmafMux, MkvMux, ProgressiveMux, TsMux};
    let want: &[(&str, u64)] = CODEC_MUX_GOLDEN;
    let mut bad = Vec::new();
    let mut i = 0;
    for &rel in CODEC_MUX_FIXTURES {
        let Some(data) = fixture(rel) else {
            eprintln!("SKIPPED {rel}");
            i += 4;
            continue;
        };
        let media = TsDemux::new().unpackage(data.as_slice()).unwrap();
        let outs = [
            (
                "progressive",
                hash_result(ProgressiveMux::new(true).package(&media)),
            ),
            ("mkv", hash_result(MkvMux::new().package(&media))),
            ("ts", hash_result(TsMux::new().package(&media))),
            ("cmaf", hash_result(CmafMux::new(1).package(&media))),
        ];
        for (kind, got) in outs {
            let name = format!("{rel}:{kind}");
            let w = want.get(i).map_or(0, |x| x.1);
            check(&mut bad, &name, got, w);
            i += 1;
        }
    }
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

const CODEC_MUX_GOLDEN: &[(&str, u64)] = &[
    ("ts/hevc/main.ts:progressive", 0x7bb81fa5b4a58807),
    ("ts/hevc/main.ts:mkv", 0x80f5898fe0c52344),
    ("ts/hevc/main.ts:ts", 0xd23cfa1eeeb4653a),
    ("ts/hevc/main.ts:cmaf", 0x7ae4e96ee243e7fb),
    ("ts/dolby/ac3.ts:progressive", 0x89dd710415beb811),
    ("ts/dolby/ac3.ts:mkv", 0xce08b347c090b364),
    ("ts/dolby/ac3.ts:ts", 0x4264345c1902e667),
    ("ts/dolby/ac3.ts:cmaf", 0x6357b5360e973b1c),
    ("ts/dolby/eac3.ts:progressive", 0xa743e6f48ccebd10),
    ("ts/dolby/eac3.ts:mkv", 0xff3f75dbedd01593),
    ("ts/dolby/eac3.ts:ts", 0x8b7254c6d433dc37),
    ("ts/dolby/eac3.ts:cmaf", 0x5e815faf1825ca21),
];

#[test]
fn dash_mpd_output_is_unchanged() {
    use broadcast_common::Package;
    use transmux::DashPackager;
    let mut bad = Vec::new();
    for (i, rel) in ["ts/h264_aac.ts", "ts/hevc/main.ts", "ts/dolby/ac3.ts"]
        .into_iter()
        .enumerate()
    {
        let Some(data) = fixture(rel) else {
            eprintln!("SKIPPED {rel}");
            continue;
        };
        let media = TsDemux::new().unpackage(data.as_slice()).unwrap();
        let got = match DashPackager::default().package(&media) {
            Ok(mpd) => fnv1a(mpd.as_bytes()),
            Err(e) => fnv1a(format!("ERR:{e}").as_bytes()),
        };
        let want = DASH_GOLDEN.get(i).copied().unwrap_or(0);
        check(&mut bad, &format!("mpd:{rel}"), got, want);
    }
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

const DASH_GOLDEN: &[u64] = &[0xc52c2b56957ee88f, 0x0a897d11bfb559ec, 0xe7a3a19c6169f912];

#[cfg(feature = "cenc")]
#[test]
fn cenc_decrypted_output_is_unchanged() {
    use broadcast_common::Decrypt;
    use transmux::cenc_decrypt::{CencDecryptor, KeyMap};
    const KID: [u8; 16] = [
        0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3,
        0xb8,
    ];
    const KEY: [u8; 16] = [
        0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc,
        0xbb,
    ];
    let Some(file) = fixture("mp4/cenc.mp4") else {
        eprintln!("SKIPPED mp4/cenc.mp4");
        return;
    };
    let dec = CencDecryptor::from_fmp4(&file).unwrap();
    let mut media = dec.demux().unwrap();
    let mut bad = Vec::new();
    check(
        &mut bad,
        "cenc-demux-encrypted",
        fnv1a(format!("{media:?}").as_bytes()),
        CENC_GOLDEN[0],
    );
    dec.decrypt(&mut media, &KeyMap::new().with_key(KID, KEY))
        .unwrap();
    let mut bytes = Vec::new();
    for t in &media.tracks {
        for smp in &t.samples {
            bytes.extend_from_slice(&smp.data);
        }
    }
    check(
        &mut bad,
        "cenc-decrypted-bytes",
        fnv1a(&bytes),
        CENC_GOLDEN[1],
    );
    check(
        &mut bad,
        "cenc-decrypted-media",
        fnv1a(format!("{media:?}").as_bytes()),
        CENC_GOLDEN[2],
    );
    assert!(bad.is_empty(), "output changed: {bad:?}");
}

#[cfg(feature = "cenc")]
const CENC_GOLDEN: [u64; 3] = [0xf124776756fb7155, 0xfb42a9ef77a663dd, 0x80b225c275a0c9a2];
