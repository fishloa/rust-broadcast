//! In-band CEA-608 carriage: real captures whose H.264 video carries ATSC
//! A/53 `GA94` caption SEI (`fixtures/captions/**`, Apache-2.0, see
//! `fixtures/captions/PROVENANCE.md`) go through demux -> IR -> TS mux ->
//! demux with every caption byte intact. transmux treats samples as opaque, so
//! the caption SEI NAL units must come out exactly as they went in.
//!
//! Oracle: the number of `GA94` + `user_data_type_code 0x03` markers (the 5
//! bytes `47 41 39 34 03`, which contain no zero byte and so are never
//! altered by emulation prevention) per capture is the independent Python
//! extractor's frame count (`fixtures/captions/*.cc_data.txt` line count).

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{Media, TsDemux, TsMux};

const GA94_MARKER: [u8; 5] = *b"GA94\x03";

fn marker_count(media: &Media) -> usize {
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .flat_map(|t| t.samples.iter())
        .map(|s| {
            s.data
                .windows(GA94_MARKER.len())
                .filter(|w| *w == GA94_MARKER)
                .count()
        })
        .sum()
}

fn video_payloads(media: &Media) -> Vec<Vec<u8>> {
    media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .flat_map(|t| t.samples.iter().map(|s| s.data.to_vec()))
        .collect()
}

fn assert_carried(ts_path: &str, reference_path: &str) {
    let ts = test_fixtures::read(ts_path);
    let want = test_fixtures::read_to_string(reference_path)
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();
    let media = TsDemux::new().unpackage(&ts).expect("demux the capture");
    assert_eq!(
        marker_count(&media),
        want,
        "{ts_path}: captions seen on demux"
    );

    let remuxed = TsMux::new().package(&media).expect("mux the IR back to TS");
    let media2 = TsDemux::new().unpackage(&remuxed).expect("re-demux");
    assert_eq!(
        marker_count(&media2),
        want,
        "{ts_path}: captions lost across demux -> mux -> demux"
    );
    assert_eq!(
        video_payloads(&media2),
        video_payloads(&media),
        "{ts_path}: video access-unit bytes (and so their caption SEI) changed"
    );
}

#[test]
fn mux_js_multi_channel_608_survives_ts_remux() {
    assert_carried(
        "captions/mux.js/multi-channel-608-captions.ts",
        "captions/mux.js/multi-channel-608-captions.cc_data.txt",
    );
}

#[test]
fn shaka_608_survives_ts_remux() {
    assert_carried(
        "captions/shaka/captions-test.ts",
        "captions/shaka/captions-test.cc_data.txt",
    );
}

/// fMP4 (DASH) flavour: mux.js `dash-608-captions-init.mp4` + `-seg.m4s` carry
/// three `GA94` SEI messages in the video samples (counted independently with
/// `grep -c` over the segment bytes). They survive demux and a CMAF re-mux.
#[test]
fn mux_js_fmp4_608_survives_cmaf_remux() {
    use transmux::{CmafMux, Fmp4Demux};
    let mut buf = test_fixtures::read("captions/mux.js/dash-608-captions-init.mp4");
    buf.extend_from_slice(&test_fixtures::read(
        "captions/mux.js/dash-608-captions-seg.m4s",
    ));
    let raw_markers = buf
        .windows(GA94_MARKER.len())
        .filter(|w| *w == GA94_MARKER)
        .count();
    assert_eq!(raw_markers, 3, "independent count over the raw fMP4 bytes");
    let media = Fmp4Demux::new().unpackage(&buf).expect("demux fMP4");
    assert_eq!(marker_count(&media), 3);
    let out = CmafMux::new(1).package(&media).expect("CMAF re-mux");
    let media2 = Fmp4Demux::new().unpackage(&out).expect("re-demux");
    assert_eq!(marker_count(&media2), 3);
    assert_eq!(video_payloads(&media2), video_payloads(&media));
}
