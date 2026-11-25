//! Names the streaming-first refactors deleted, with the reason a
//! reappearance is a regression.

/// One symbol that must stay gone.
pub struct Banned {
    /// The identifier as it would be spelled in source.
    pub name: &'static str,
    /// Why it may not come back.
    pub reason: &'static str,
    /// The issue that removed it.
    pub issue: &'static str,
}

/// Workspace members whose files are not scanned.
///
/// `workspace-guards` holds the name table itself.
pub const EXEMPT: &[&str] = &["workspace-guards"];

/// Every name a streaming-first slice deleted. A name is matched as a whole
/// identifier, so its replacement (`CmafMuxStage` for `CmafMux`) does not
/// trip it; a name that a live item also spells (`SplicePlan::concat`, the
/// test-side `rtp_run::RtpOutput`) is recorded in the qualified form that
/// names the deleted item instead.
pub const TABLE: &[Banned] = &[
    Banned {
        name: "Fmp4Demux",
        reason: "the one MP4 demuxer is transmux::mp4_demux::Mp4Demux",
        issue: "#4057",
    },
    Banned {
        name: "ProgressiveDemux",
        reason: "the one MP4 demuxer is transmux::mp4_demux::Mp4Demux",
        issue: "#4057",
    },
    Banned {
        name: "demux_progressive",
        reason: "the whole-input progressive parse is gone with ProgressiveDemux",
        issue: "#4057",
    },
    Banned {
        name: "CmafMux",
        reason: "the CMAF muxer is the CmafMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "TsMux",
        reason: "the TS muxer is the TsMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "StreamingTsMux",
        reason: "the TS muxer is the TsMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "TsHlsPackager",
        reason: "classic TS-HLS is the StreamingTsHlsSegmenter stage",
        issue: "#4057",
    },
    Banned {
        name: "TsHlsOutput",
        reason: "classic TS-HLS is the StreamingTsHlsSegmenter stage",
        issue: "#4057",
    },
    Banned {
        name: "TsSegment",
        reason: "a segment is a MuxOutput::Segment carrying SegmentOut",
        issue: "#4057",
    },
    Banned {
        name: "MkvMux",
        reason: "the Matroska muxer is the MkvMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "FlvMux",
        reason: "the FLV muxer is the FlvMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "RtmpMux",
        reason: "the RTMP muxer is the RtmpMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "RtpPacketiser",
        reason: "the RTP muxer is the RtpPacketiserStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "rtp::RtpOutput",
        reason: "RTP output is MuxOutput from RtpPacketiserStage",
        issue: "#4057",
    },
    Banned {
        name: "transmux::RtpOutput",
        reason: "RTP output is MuxOutput from RtpPacketiserStage",
        issue: "#4057",
    },
    Banned {
        name: "ProgressiveMux",
        reason: "progressive MP4 is PlanPass over ProgressiveMuxStage",
        issue: "#4057",
    },
    Banned {
        name: "CencEncryptor",
        reason: "protection is applied by the CencEncryptStage stage",
        issue: "#4057",
    },
    Banned {
        name: "CencDecryptor",
        reason: "protection is removed by the CencDecryptStage stage",
        issue: "#4057",
    },
    Banned {
        name: "TrackEncryption",
        reason: "per-track crypto defaults live on TrackSpec::protection",
        issue: "#4057",
    },
    Banned {
        name: "UnmuxableDataTrack",
        reason: "an unmapped data track is Error::DataIdentityMismatch",
        issue: "#4057",
    },
    Banned {
        name: "rebase_to_zero",
        reason: "timeline rebasing is the RebaseStage stage",
        issue: "#4057",
    },
    Banned {
        name: "apply_offset",
        reason: "a timeline offset is the OffsetStage stage",
        issue: "#4057",
    },
    Banned {
        name: "insert_discontinuity_gap",
        reason: "a timeline gap is the GapStage stage",
        issue: "#4057",
    },
    Banned {
        name: "trickplay::derive_iframe_track",
        reason: "trick-play derivation is the TrickPlayStage stage",
        issue: "#4057",
    },
    Banned {
        name: "trickplay::append_iframe_track",
        reason: "trick-play derivation is the TrickPlayStage stage",
        issue: "#4057",
    },
    Banned {
        name: "splice::concat",
        reason: "concatenation is the SpliceStage stage over a SplicePlan",
        issue: "#4057",
    },
    Banned {
        name: "splice::splice_insert",
        reason: "SSAI splicing is SplicePlan::insert over the SpliceStage stage",
        issue: "#4057",
    },
    Banned {
        name: "SpliceResult",
        reason: "a splice plan reports SplicePoints; a caller owns the output",
        issue: "#4057",
    },
    Banned {
        name: "SegmentMeta",
        reason: "a segment carries SegmentOut { bytes, summary, tracks }",
        issue: "#4057",
    },
    Banned {
        name: "transmux::SmoothOutput",
        reason: "a Smooth fragment is a SegmentOut from SmoothSegmenter",
        issue: "#4057",
    },
    Banned {
        name: "SmoothFragment",
        reason: "a Smooth fragment is a SegmentOut from SmoothSegmenter",
        issue: "#4057",
    },
];
