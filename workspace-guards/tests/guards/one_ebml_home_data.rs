//! Per-crate data for the `one_ebml_home` guard: the crates that may spell an
//! EBML element ID outside `broadcast-matroska`, one reviewed line at a time.

use crate::one_ebml_home::Crate;

/// The one home of the EBML/Matroska element IDs (RFC 8794, RFC 9559).
pub const EXEMPT: &[&str] = &["broadcast-matroska"];

pub const ALLOW: &[Crate] = &[
    Crate {
        name: "container-probe",
        allow: &[(
            "src/ebml.rs",
            "const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];",
            "container-probe depends only on broadcast-common at runtime; its dev-only drift tests pin this magic to broadcast-matroska",
        )],
    },
    Crate {
        name: "transmux",
        allow: &[
            (
                "src/webm_demux.rs",
                "const EBML_HEADER: u32 = 0x1A45_DFA3;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const SEGMENT: u32 = 0x1853_8067;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const INFO: u32 = 0x1549_A966;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const TRACKS: u32 = 0x1654_AE6B;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const CLUSTER: u32 = 0x1F43_B675;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const SEEK_HEAD: u32 = 0x114D_9B74;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const CUES: u32 = 0x1C53_BB6B;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const TAGS: u32 = 0x1254_C367;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const CHAPTERS: u32 = 0x1043_A770;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/webm_demux.rs",
                "const ATTACHMENTS: u32 = 0x1941_A469;",
                "transmux's hand-written WebM demuxer moves onto broadcast-matroska in Phase 5 slice 3",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_SEEK_HEAD: [u8; 4] = [0x11, 0x4D, 0x9B, 0x74];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_INFO: [u8; 4] = [0x15, 0x49, 0xA9, 0x66];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_TRACKS: [u8; 4] = [0x16, 0x54, 0xAE, 0x6B];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_CUES: [u8; 4] = [0x1C, 0x53, 0xBB, 0x6B];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
            (
                "src/mkv_mux.rs",
                "const ID_EBML: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];",
                "transmux's hand-written Matroska muxer moves onto broadcast-matroska in Phase 5 slice 4",
            ),
        ],
    },
];
