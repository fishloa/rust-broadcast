//! Per-crate data for the `non_exhaustive` guard. Add a line for a new crate.

use crate::non_exhaustive::Crate;

const fn plain(name: &'static str) -> Crate {
    Crate {
        name,
        skip: &[],
        variants: false,
    }
}

/// Members with no `src/lib.rs` (pure binaries: `dvb-tools`, `multimux-cli`) or no
/// public enums in `src/` (`dvb-stream`: async stream adapters + a stats struct;
/// `test-bounded`, `test-alloc`, `test-fixtures`: shared test helpers).
pub const EXEMPT: &[&str] = &[
    "dvb-tools",
    "multimux-cli",
    "dvb-stream",
    "test-bounded",
    "test-alloc",
    "test-fixtures",
];

pub const TABLE: &[Crate] = &[
    plain("atsc3-route"),
    plain("atsc3"),
    plain("broadcast-auth"),
    plain("broadcast-client"),
    plain("broadcast-common"),
    plain("broadcast-hls"),
    plain("broadcast-loudness"),
    plain("buffering-runtime"),
    plain("caption-convert"),
    plain("cc-data"),
    plain("compliance-probe"),
    Crate {
        name: "container-probe",
        skip: &[],
        variants: true,
    },
    plain("dash-runtime"),
    plain("dvb-bbframe"),
    plain("dvb-ci-runtime"),
    plain("dvb-ci"),
    plain("dvb-conformance"),
    plain("dvb-csa"),
    plain("dvb-mabr"),
    plain("dvb-si"),
    plain("dvb-simulcrypt"),
    plain("dvb-subtitle"),
    plain("dvb-t2mi"),
    plain("dvb-vbi"),
    plain("hls-runtime"),
    plain("media-doctor"),
    plain("media-plane"),
    plain("mp4-emsg"),
    plain("mpeg-pes"),
    plain("mpeg-ps"),
    plain("mpeg-ts"),
    plain("multimux"),
    plain("playout-runtime"),
    plain("rdd29"),
    plain("rist-runtime"),
    plain("rmt-flute"),
    plain("rtcp-packet"),
    plain("rtmp-runtime"),
    plain("rtp-packet"),
    plain("rtsp-runtime"),
    plain("scte104"),
    plain("scte35-splice"),
    plain("srt-runtime"),
    plain("ssai-runtime"),
    plain("st12-1"),
    plain("st2022"),
    plain("st291"),
    plain("st337"),
    plain("st377-1"),
    plain("timed-metadata"),
    plain("transmux"),
    plain("ts-fix"),
    plain("tshttp-runtime"),
    plain("ttml-subtitle"),
    plain("ule"),
    plain("webrtc-runtime"),
];
