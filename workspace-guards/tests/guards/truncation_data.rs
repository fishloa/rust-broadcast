//! Per-crate data for the `truncation` guard (#1129): each crate's allowlist
//! verbatim, with its reasons. Add a line for a new crate.

use crate::truncation::Crate;

pub const EXEMPT: &[&str] = &[
    "broadcast-common",
    "broadcast-auth",
    "dvb-si",
    "dvb-conformance",
    "dvb-tools",
    "media-doctor",
    "multimux",
    "multimux-cli",
    "dvb-stream",
    "dvb-ci-runtime",
    "timed-metadata",
    "ts-fix",
    "transmux",
    "rtsp-runtime",
    "hls-runtime",
    "dash-runtime",
    "media-plane",
    "ttml-subtitle",
    "broadcast-hls",
    "broadcast-loudness",
    "buffering-runtime",
    "tshttp-runtime",
    "broadcast-client",
    "dvb-csa",
    "dvb-mabr",
    "atsc3",
    "webrtc-runtime",
    "caption-convert",
    "compliance-probe",
    "ssai-runtime",
    "playout-runtime",
    "container-probe",
    "test-bounded",
    "test-alloc",
    "test-fixtures",
];

pub const TABLE: &[Crate] = &[
    Crate {
        name: "atsc3-route",
        allowlist: &[],
    },
    Crate {
        name: "cc-data",
        allowlist: &[],
    },
    Crate {
        name: "dvb-bbframe",
        allowlist: &[],
    },
    Crate {
        name: "dvb-ci",
        allowlist: &[
            (
                "src/ci_plus/cicam_player.rs",
                "buf[0..2].copy_from_slice(&(loc.len() as u16).to_be_bytes());",
                "write_apdu_header runs first and length::encode_with rejects any APDU body > 0xFFFF (LengthTooLarge), so loc.len() + 2 <= 0xFFFF.",
            ),
            (
                "src/ci_plus/cicam_player.rs",
                "buf[pos..pos + 2].copy_from_slice(&(self.component_types.len() as u16).to_be_bytes());",
                "write_apdu_header rejects body > 0xFFFF first; count * 2 + 2 <= 0xFFFF.",
            ),
            (
                "src/ci_plus/cicam_player.rs",
                "buf[pos + 5..pos + 7].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());",
                "write_apdu_header rejects body > 0xFFFF first; pmt.len() + 7 <= 0xFFFF.",
            ),
            (
                "src/ci_plus/cicam_player.rs",
                "buf[pos + 1..pos + 3].copy_from_slice(&(self.pmt.len() as u16).to_be_bytes());",
                "write_apdu_header rejects body > 0xFFFF first; pmt.len() + 3 <= 0xFFFF.",
            ),
            (
                "src/builder.rs",
                "out.push(body.len() as u8);",
                "body comes from DescriptorLoop::raw_tags, which already validated it fits the declared 8-bit descriptor_length.",
            ),
            (
                "src/ci_plus/low_speed_comms_v4.rs",
                "buf[p + 3] = self.source_addresses.len() as u8;",
                "bounded by write_descriptor_header just above, which rejects descriptor_length > 255 (count * 17 + fixed <= 255).",
            ),
            (
                "src/ci_plus/sample_decryption.rs",
                "w.u16(self.drm_metadata.len() as u16);",
                "validate() (range-checks drm_metadata_length, 16 bits) always runs before write_into.",
            ),
            (
                "src/ci_plus/sample_decryption.rs",
                "w.u8(records.len() as u8);",
                "validate() range-checks number_of_metadata_records (8 bits) before write_into.",
            ),
            (
                "src/ci_plus/sample_decryption.rs",
                "w.u8(tracks.len() as u8);",
                "validate() range-checks number_of_Sample_Tracks (8 bits) before write_into.",
            ),
            (
                "src/ci_plus/sample_decryption.rs",
                "w.u8(track.records.len() as u8);",
                "validate() range-checks the per-track number_of_metadata_records (8 bits) before write_into.",
            ),
        ],
    },
    Crate {
        name: "dvb-simulcrypt",
        allowlist: &[],
    },
    Crate {
        name: "dvb-subtitle",
        allowlist: &[],
    },
    Crate {
        name: "dvb-t2mi",
        allowlist: &[],
    },
    Crate {
        name: "dvb-vbi",
        allowlist: &[],
    },
    Crate {
        name: "mp4-emsg",
        allowlist: &[],
    },
    Crate {
        name: "mpeg-pes",
        allowlist: &[],
    },
    Crate {
        name: "mpeg-ps",
        allowlist: &[],
    },
    Crate {
        name: "mpeg-ts",
        allowlist: &[],
    },
    Crate {
        name: "rdd29",
        allowlist: &[],
    },
    Crate {
        name: "rist-runtime",
        allowlist: &[],
    },
    Crate {
        name: "rmt-flute",
        allowlist: &[],
    },
    Crate {
        name: "rtcp-packet",
        allowlist: &[],
    },
    Crate {
        name: "rtmp-runtime",
        allowlist: &[],
    },
    Crate {
        name: "rtp-packet",
        allowlist: &[],
    },
    Crate {
        name: "scte104",
        allowlist: &[],
    },
    Crate {
        name: "scte35-splice",
        allowlist: &[],
    },
    Crate {
        name: "srt-runtime",
        allowlist: &[],
    },
    Crate {
        name: "st12-1",
        allowlist: &[],
    },
    Crate {
        name: "st2022",
        allowlist: &[],
    },
    Crate {
        name: "st291",
        allowlist: &[],
    },
    Crate {
        name: "st337",
        allowlist: &[],
    },
    Crate {
        name: "st377-1",
        allowlist: &[
            (
                "src/types.rs",
                "out.extend_from_slice(&(items.len() as u32).to_be_bytes());",
                "infallible serialize_uid_batch: wrapping needs >= 2^32 16-byte UL items (a 64 GiB batch); documented unreachable, not testable.",
            ),
            (
                "src/primer.rs",
                "buf[pos..pos + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());",
                "wrapping needs >= 2^32 18-byte primer entries (a 72 GiB pack); serialized_len() could not be allocated first. Documented unreachable.",
            ),
        ],
    },
    Crate {
        name: "ule",
        allowlist: &[],
    },
];
