//! Per-crate data for the `one_box_home` guard. Every workspace member is
//! in the table (its `src/` is scanned) or in [`EXEMPT`] with a reason. The
//! only crate that may keep a box walk of its own is `container-probe`, whose
//! individual lines are allowlisted with that reason; add a line for a crate
//! that needs one, never a whole-crate exemption.

use crate::one_box_home::Crate;

/// Members with no ISO BMFF box syntax and no reason to be scanned: `isobmff`
/// is the one home of the box syntax itself; `test-alloc` and `test-fixtures`
/// are shared test helpers holding an allocator and fixture resolution only.
pub const EXEMPT: &[&str] = &[
    "isobmff",
    "isobmff-tokio",
    // A plug-in crate that *owns* its private four-CCs (`acal`, `actm`): naming
    // them outside `isobmff` is the whole point of the example, and it uses
    // `isobmff`'s header writers rather than its own.
    "examples/isobmff-private-format",
    "test-alloc",
    "test-fixtures",
];

const fn plain(name: &'static str) -> Crate {
    Crate { name, allow: &[] }
}

pub const TABLE: &[Crate] = &[
    plain("atsc3"),
    plain("atsc3-route"),
    plain("broadcast-auth"),
    plain("broadcast-client"),
    plain("broadcast-common"),
    plain("broadcast-dash"),
    plain("broadcast-hls"),
    plain("broadcast-loudness"),
    plain("broadcast-smooth"),
    plain("buffering-runtime"),
    plain("caption-convert"),
    plain("cc-data"),
    plain("compliance-probe"),
    Crate {
        name: "container-probe",
        // The one crate that keeps its own header walk: it must depend only on
        // `broadcast-common` at runtime. Its dev-only drift tests pin these
        // constants and the walk to `isobmff` on every ISO BMFF fixture.
        allow: &[
            (
                "src/isobmff.rs",
                "pub(crate) const BOX_HEADER_MIN_SIZE: usize = 8;",
                "container-probe keeps its own walk (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "pub(crate) const SIZE_TO_EOF: u32 = 0;",
                "container-probe keeps its own walk (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "pub(crate) const SIZE_INDICATES_LARGESIZE: u32 = 1;",
                "container-probe keeps its own walk (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "const TYPE_FTYP: [u8; 4] = *b\"ftyp\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "const TYPE_STYP: [u8; 4] = *b\"styp\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "pub(crate) const TYPE_MOOV: [u8; 4] = *b\"moov\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "pub(crate) const TYPE_MOOF: [u8; 4] = *b\"moof\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "const TYPE_SKIP: [u8; 4] = *b\"skip\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "const TYPE_FREE: [u8; 4] = *b\"free\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/isobmff.rs",
                "pub(crate) const TYPE_MDAT: [u8; 4] = *b\"mdat\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
            (
                "src/segment.rs",
                "const TYPE_UUID: [u8; 4] = *b\"uuid\";",
                "container-probe keeps its own box-type constants (broadcast-common only at runtime), pinned by the isobmff drift tests",
            ),
        ],
    },
    plain("dash-runtime"),
    plain("dvb-bbframe"),
    plain("dvb-ci"),
    plain("dvb-ci-runtime"),
    plain("dvb-conformance"),
    plain("dvb-csa"),
    plain("dvb-mabr"),
    plain("dvb-si"),
    plain("dvb-simulcrypt"),
    plain("dvb-stream"),
    plain("dvb-subtitle"),
    plain("dvb-t2mi"),
    plain("dvb-tools"),
    plain("dvb-vbi"),
    plain("dvr-runtime"),
    plain("hls-runtime"),
    plain("media-doctor"),
    plain("media-plane"),
    plain("mpeg-pes"),
    plain("mpeg-ps"),
    plain("mpeg-ts"),
    plain("multimux"),
    plain("multimux-cli"),
    plain("playout-runtime"),
    plain("rdd29"),
    plain("rist-runtime"),
    plain("rmt-flute"),
    plain("rtmp-runtime"),
    plain("rtp-packet"),
    plain("rtsp-runtime"),
    plain("scte104"),
    plain("scte35-splice"),
    plain("smooth-runtime"),
    plain("srt-runtime"),
    plain("ssai-runtime"),
    plain("st12-1"),
    plain("st2022"),
    plain("st291"),
    plain("st337"),
    plain("st377-1"),
    plain("test-harness"),
    plain("timed-metadata"),
    plain("transmux"),
    plain("ts-fix"),
    plain("ttml-subtitle"),
    plain("ule"),
    plain("webrtc-runtime"),
];
