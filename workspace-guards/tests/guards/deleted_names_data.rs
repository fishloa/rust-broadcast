//! Names the streaming-first Phase 3 slice 6 deleted, with the reason a
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
/// `workspace-guards` holds the name table itself; `isobmff` sits below
/// transmux (its two `movie_fragment.rs` comments name the demuxer but cannot
/// reference it) and is owned by a concurrent lane, so its stale comment is
/// fixed there rather than here.
pub const EXEMPT: &[&str] = &["isobmff", "workspace-guards"];

/// Every name the whole-input MP4 demux removal deleted. The replacement is
/// `transmux::mp4_demux::Mp4Demux`, the one streaming stage.
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
];
