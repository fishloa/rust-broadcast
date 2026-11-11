//! Per-crate data for the `truncation` guard (#1129): each crate's allowlist
//! verbatim, with its reasons. Add a line for a new crate.

use crate::truncation::Crate;

pub const EXEMPT: &[&str] = &[
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
    "rtsp-runtime",
    "hls-runtime",
    "dash-runtime",
    "media-plane",
    "ttml-subtitle",
    "broadcast-hls",
    "broadcast-loudness",
    "buffering-runtime",
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
    "test-alloc",
    "test-fixtures",
];

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-dash",
        allowlist: &[],
    },
    Crate {
        name: "broadcast-common",
        allowlist: &[],
    },
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
    Crate {
        name: "transmux",
        allowlist: &[
            (
                "src/annexb.rs",
                "out.extend_from_slice(&(nal.len() as u32).to_be_bytes());",
                "length_prefix, the one 4-byte NAL length writer: its NALs come from \
                     TS demux (PES buffer capped at 4 MiB, ts_demux MAX_PES_BUFFER_BYTES) \
                     and RTP depacketisation (FU-A / STAP-A reassembler caps), never near 4 GiB.",
            ),
            (
                "src/h262.rs",
                "u8::try_from(aspect).unwrap_or(u8::MAX),",
                "Annex E aspect_ratio_information is a 4-bit code copied through from a parsed sequence header; these builders only re-emit parsed fields.",
            ),
            (
                "src/h262.rs",
                "debug_assert_eq!(u8::try_from(id).unwrap_or(u8::MAX), SEQUENCE_EXTENSION_ID);",
                "A debug assertion comparing against a constant, not a length write.",
            ),
            (
                "src/h262.rs",
                "u8::try_from(id).unwrap_or(u8::MAX),",
                "sequence_extension_id is the fixed 1-byte extension id copied through from a parsed value; the clamp is unreachable.",
            ),
            (
                "src/h262.rs",
                "u8::try_from(p).unwrap_or(u8::MAX),",
                "Chroma-format flag in the sequence extension; copied through from a parsed value.",
            ),
            (
                "src/h262.rs",
                "u8::try_from(t).unwrap_or(u8::MAX),",
                "Chroma-format flag in the sequence extension; copied through from a parsed value.",
            ),
            (
                "src/h262.rs",
                "u8::try_from(m).unwrap_or(u8::MAX),",
                "Chroma-format flag in the sequence extension; copied through from a parsed value.",
            ),
            (
                "src/h262.rs",
                "video_format: u8::try_from(video_format).unwrap_or(u8::MAX),",
                "video_format is a 3-bit Annex E code copied through from a parsed sequence header.",
            ),
        ],
    },
];
