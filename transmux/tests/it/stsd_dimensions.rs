//! Coded dimensions of the VVC / AV1 / VP9 / MPEG-2 video sample entries and
//! the 16.16 `tkhd` presentation size.
//!
//! Real fragmented fixtures carry each codec's own sample entry; the track is
//! resized to 1920x1080 and the built init segment read back at the two places
//! a player takes layout from: the VisualSampleEntry `width`/`height`
//! (ISO/IEC 14496-12:2026 §12.1.3) and `tkhd`'s 16.16 `width`/`height`
//! (§8.3.2). A zero here would let a track mux as 0x0 with every other gate
//! green.
use broadcast_common::timescale::Timescale;
use transmux::TrackSpec;
use transmux::pipeline::{self, CodecConfig};

const WIDTH: u16 = 1920;
const HEIGHT: u16 = 1080;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

/// Demux `rel`, resize its video config to 1920x1080 and drop the display
/// geometry so `tkhd` falls back to the codec config's dimensions.
fn resized_spec(rel: &str) -> TrackSpec {
    let bytes =
        std::fs::read(test_fixtures::path(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    let media =
        test_harness::collect::media_from_mp4(&bytes, test_harness::collect::CollectMode::Seekable)
            .unwrap_or_else(|e| panic!("demux {rel}: {e}"));
    let mut spec = media
        .tracks
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{rel} carries no track"))
        .spec;
    match &mut spec.config {
        CodecConfig::Vvc { width, height, .. }
        | CodecConfig::Av1 { width, height, .. }
        | CodecConfig::Vp9 { width, height, .. }
        | CodecConfig::Mpeg2Video { width, height, .. } => {
            *width = WIDTH;
            *height = HEIGHT;
        }
        other => panic!("{rel}: unexpected config {other:?}"),
    }
    spec.with_display(None)
}

/// Read a VisualSampleEntry's `width`/`height`: after the 8-byte box header
/// come 6 reserved bytes, data_reference_index (2), pre_defined (2), reserved
/// (2), pre_defined[3] (12) — 24 bytes before the two 16-bit dimensions.
fn visual_dims(init: &[u8], fourcc: &[u8; 4]) -> (u16, u16) {
    let pos = init
        .windows(4)
        .position(|w| w == fourcc)
        .unwrap_or_else(|| panic!("sample entry {fourcc:?} must be present"));
    let start = pos - 4;
    let size = u32::from_be_bytes(init[start..start + 4].try_into().unwrap()) as usize;
    assert!(size >= 36, "{fourcc:?} sample entry is only {size} bytes");
    let entry = &init[start..start + size];
    let width = u16::from_be_bytes([entry[32], entry[33]]);
    let height = u16::from_be_bytes([entry[34], entry[35]]);
    (width, height)
}

/// `tkhd`'s 16.16 `width`/`height` are the box's last 8 bytes.
fn tkhd_dims(init: &[u8]) -> (u32, u32) {
    let pos = init
        .windows(4)
        .position(|w| w == b"tkhd")
        .expect("tkhd box must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes(init[start..start + 4].try_into().unwrap()) as usize;
    let bytes = &init[start..start + size];
    let width = u32::from_be_bytes(bytes[size - 8..size - 4].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[size - 4..size].try_into().unwrap());
    (width, height)
}

#[test]
fn video_sample_entry_and_tkhd_carry_the_coded_dimensions() {
    let rows: [(&str, &[u8; 4]); 4] = [
        ("mp4/frag/vvc.frag.mp4", b"vvc1"),
        ("mp4/frag/av1.frag.mp4", b"av01"),
        ("mp4/frag/vp9.frag.mp4", b"vp09"),
        ("mp4/frag/mpeg2video.frag.mp4", b"mp4v"),
    ];
    for (rel, fourcc) in rows {
        let spec = resized_spec(rel);
        let init = pipeline::build_init_segment(&[spec], timescale(90_000))
            .unwrap_or_else(|e| panic!("{rel}: build init: {e}"));

        assert_eq!(
            visual_dims(&init, fourcc),
            (WIDTH, HEIGHT),
            "{rel}: the sample entry must carry the coded width/height"
        );
        assert_eq!(
            tkhd_dims(&init),
            (u32::from(WIDTH) << 16, u32::from(HEIGHT) << 16),
            "{rel}: tkhd must carry the dimensions in 16.16 fixed point"
        );
    }
}
