//! Per-crate data for the `bounded_input` guard. The scope is every workspace
//! member's non-test `src/` and its `examples/`, so a new member adds a line
//! here (or lands in `EXEMPT` with its reason) or `table_covers_every_workspace_member`
//! fails. The only allowlisted file is the one capped whole-document read.

use crate::bounded_input::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate { name, allow: &[] }
}

/// Test-support crates (`publish = false`): they read fixtures and source
/// files by design, never a media capture in production code.
pub const EXEMPT: &[&str] = &[
    "test-fixtures",
    "test-harness",
    "test-alloc",
    "workspace-guards",
];

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-common",
        // The one capped whole-document read.
        allow: &[("src/bounded.rs", "the one capped whole-document read")],
    },
    plain("broadcast-core"),
    plain("broadcast-codec"),
    plain("broadcast-bitstream"),
    plain("broadcast-matroska"),
    plain("broadcast-auth"),
    plain("dvb-si"),
    plain("dvb-t2mi"),
    plain("dvb-bbframe"),
    plain("dvb-conformance"),
    plain("dvb-tools"),
    plain("media-doctor"),
    plain("multimux"),
    plain("multimux-cli"),
    plain("scte35-splice"),
    plain("dvb-stream"),
    plain("mpeg-pes"),
    plain("dvb-subtitle"),
    plain("mpeg-ps"),
    plain("scte104"),
    plain("cc-data"),
    plain("dvb-ci"),
    plain("st291"),
    plain("ule"),
    plain("dvb-simulcrypt"),
    plain("dvb-vbi"),
    plain("rmt-flute"),
    plain("dvb-ci-runtime"),
    plain("timed-metadata"),
    plain("mpeg-ts"),
    plain("ts-fix"),
    plain("transmux"),
    plain("rtsp-runtime"),
    plain("rtmp-runtime"),
    plain("srt-runtime"),
    plain("rtp-packet"),
    plain("st12-1"),
    plain("st337"),
    plain("rdd29"),
    plain("st377-1"),
    plain("hls-runtime"),
    plain("dash-runtime"),
    plain("media-plane"),
    plain("ttml-subtitle"),
    plain("broadcast-hls"),
    plain("broadcast-dash"),
    plain("broadcast-loudness"),
    plain("buffering-runtime"),
    plain("broadcast-client"),
    plain("dvb-csa"),
    plain("dvb-mabr"),
    plain("atsc3"),
    plain("atsc3-route"),
    plain("st2022"),
    plain("rist-runtime"),
    plain("webrtc-runtime"),
    plain("caption-convert"),
    plain("compliance-probe"),
    plain("ssai-runtime"),
    plain("playout-runtime"),
    plain("container-probe"),
    plain("broadcast-smooth"),
    plain("smooth-runtime"),
    plain("dvr-runtime"),
    plain("isobmff"),
    plain("isobmff-tokio"),
    plain("examples/isobmff-private-format"),
];
