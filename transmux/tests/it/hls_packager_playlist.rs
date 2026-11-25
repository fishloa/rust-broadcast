//! The content of the CMAF-HLS playlist [`HlsPackager`] renders: the
//! `#EXTINF` presentation span (RFC 8216 §4.3.2.1), the
//! `#EXT-X-TARGETDURATION` that must be at least the rounded `#EXTINF`
//! (§4.3.3.1), and the `#EXT-X-VERSION`/`#EXT-X-MEDIA-SEQUENCE`/
//! `#EXT-X-ENDLIST` tags (§4.3.1.2, §4.3.3.2, §4.3.3.5).
//!
//! Every assertion below is a literal number from the fixture track, not a
//! value recomputed from the packager: three 3 000-tick samples at 90 kHz with
//! a uniform 3 000-tick composition offset span exactly 0.1 s, so a span that
//! adds the offset instead of subtracting it, or a duration scaled by the
//! wrong arithmetic, cannot pass.

use broadcast_common::timescale::Timescale;
use core::num::NonZeroU32;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::nalu_types::{AvcPps, AvcSps};
use transmux::TrackId;
use transmux::media::{HlsPackager, Media, Track};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

const TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = 3_000;
const COMPOSITION_OFFSET: i64 = 3_000;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn avc_spec() -> TrackSpec {
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![
            0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24,
        ])],
        pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    TrackSpec::new(
        TrackId::new(1),
        timescale(TIMESCALE),
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

fn idr_bytes() -> Vec<u8> {
    let nal = [0x65u8, 0x88, 0x84, 0x00];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    data
}

fn duration(ticks: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(ticks)
}

fn package_with(media: &Media, version: u8, msn: u64) -> String {
    let packager = HlsPackager {
        version,
        msn: broadcast_hls::MediaSequence::new(msn),
        ..HlsPackager::default()
    };
    crate::packagers::hls(&packager, media).expect("the fixture media packages")
}

/// Three 3 000-tick samples (`dts` 0/3 000/6 000) whose presentation times are
/// all 3 000 ticks later: the span is 9 000 ticks = 0.1 s, anchored at
/// `pts` 3 000 (0.0333 s), not at zero.
fn offset_track_media() -> Media {
    let samples: Vec<Sample> = (0..3)
        .map(|i| {
            let dts = i64::from(i) * i64::from(FRAME_DUR);
            Sample::timed(
                idr_bytes(),
                dts,
                dts + COMPOSITION_OFFSET,
                duration(FRAME_DUR),
            )
        })
        .collect();
    Media::new(vec![Track::new(avc_spec(), samples)], timescale(TIMESCALE))
}

#[test]
fn version_media_sequence_endlist_and_extinf_are_rendered_from_the_fields() {
    let playlist = package_with(&offset_track_media(), 9, 5);
    assert!(
        playlist.contains("#EXT-X-VERSION:9"),
        "the configured version must be rendered, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXT-X-MEDIA-SEQUENCE:5"),
        "a non-zero media sequence must be rendered, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXT-X-ENDLIST"),
        "a complete playlist ends with EXT-X-ENDLIST, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXTINF:0.100,"),
        "the presentation span is 9 000 ticks at 90 kHz = 0.100 s, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXT-X-TARGETDURATION:1"),
        "an EXTINF of 0.100 needs TARGETDURATION 1, got:\n{playlist}"
    );
}

/// One sample whose presentation time equals its decode time, so the span is
/// exactly the sample duration.
fn uniform_media(ticks: u32) -> Media {
    let sample = Sample::timed(idr_bytes(), 0, 0, duration(ticks));
    Media::new(
        vec![Track::new(avc_spec(), vec![sample])],
        timescale(TIMESCALE),
    )
}

#[test]
fn target_duration_is_the_ceiling_of_a_two_and_a_half_second_span() {
    // 225 000 ticks at 90 kHz = 2.5 s: the ceiling is 3, and RFC 8216
    // §4.3.3.1 requires the target duration to be at least round(2.5) = 3.
    let playlist = package_with(&uniform_media(225_000), 7, 0);
    assert!(
        playlist.contains("#EXTINF:2.500,"),
        "a 2.5 s span renders as 2.500, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXT-X-TARGETDURATION:3"),
        "a 2.5 s span needs TARGETDURATION 3, got:\n{playlist}"
    );
}

#[test]
fn target_duration_of_an_exact_whole_second_span_is_not_rounded_up() {
    // 180 000 ticks at 90 kHz = 2.0 s exactly: the ceiling of a whole number is
    // that number, so the target duration is 2, not 3.
    let playlist = package_with(&uniform_media(180_000), 7, 0);
    assert!(
        playlist.contains("#EXTINF:2,"),
        "an exact 2 s span renders as the integer 2, got:\n{playlist}"
    );
    assert!(
        playlist.contains("#EXT-X-TARGETDURATION:2"),
        "an exact 2 s span needs TARGETDURATION 2, got:\n{playlist}"
    );
}
