//! Hostile-input caps for the element walker and typed parsers.
//!
//! A crate that allocates whatever a downloaded file claims is not safe to hand
//! a download. Every depth, count or size this crate reads from input is
//! checked against one of these caps before anything is allocated from it, so a
//! hostile header can only ever produce a typed
//! [`Error::LimitExceeded`](crate::Error), never an allocation it dictates.
//!
//! These are hostile-input caps, not caller policy: their defaults are named
//! `DEFAULT_MAX_*` constants and a caller that knows better raises them
//! through the [`with_*`](Limits::with_max_depth) builders.

/// Bytes in one kibibyte.
const KIB: u64 = 1024;
/// Bytes in one mebibyte.
const MIB: u64 = KIB * KIB;

/// Default maximum element-tree depth (the Segment, its masters, nested
/// `ChapterAtom`/`SimpleTag`, and their leaves).
pub const DEFAULT_MAX_DEPTH: u32 = 32;
/// Default maximum size of one element's payload, in bytes (256 MiB).
pub const DEFAULT_MAX_ELEMENT_BYTES: u64 = 256 * MIB;
/// Default maximum children in one master element.
pub const DEFAULT_MAX_CHILDREN: u32 = 1_000_000;
/// Default maximum bytes of one buffered level-1 master element (16 MiB).
pub const DEFAULT_MAX_MASTER_BYTES: u64 = 16 * MIB;
/// Default maximum tracks in one Segment.
pub const DEFAULT_MAX_TRACKS: u32 = 256;
/// Default maximum frames one laced Block may carry (RFC 9559 §12: the count
/// byte caps it at 256).
pub const DEFAULT_MAX_LACE_FRAMES: u32 = 256;
/// Default maximum `CuePoint` entries held for seeking.
pub const DEFAULT_MAX_CUES: u32 = 1_000_000;
/// Default maximum `CuePoint` entries held for seeking.
pub const DEFAULT_MAX_CLUSTERS: u32 = 1_000_000;
/// Default maximum bytes of one Block header prefetch: enough for a 256-frame
/// EBML lace of eight-octet sizes.
pub const DEFAULT_MAX_BLOCK_HEADER_BYTES: u64 = 64 * KIB;
/// Default maximum bytes one reader demand may ask a driver to hand over at
/// once.
pub const DEFAULT_MAX_READ_BYTES: u64 = MIB;
/// Default maximum bytes a permissive reader scans for the next level-1 id
/// before it gives up.
pub const DEFAULT_MAX_RESYNC_BYTES: u64 = MIB;

/// Hostile-input caps for the element walker and typed parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    max_depth: u32,
    max_element_bytes: u64,
    max_children: u32,
    max_master_bytes: u64,
    max_tracks: u32,
    max_lace_frames: u32,
    max_cues: u32,
    max_clusters: u32,
    max_block_header_bytes: u64,
    max_read_bytes: u64,
    max_resync_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_element_bytes: DEFAULT_MAX_ELEMENT_BYTES,
            max_children: DEFAULT_MAX_CHILDREN,
            max_master_bytes: DEFAULT_MAX_MASTER_BYTES,
            max_tracks: DEFAULT_MAX_TRACKS,
            max_lace_frames: DEFAULT_MAX_LACE_FRAMES,
            max_cues: DEFAULT_MAX_CUES,
            max_clusters: DEFAULT_MAX_CLUSTERS,
            max_block_header_bytes: DEFAULT_MAX_BLOCK_HEADER_BYTES,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            max_resync_bytes: DEFAULT_MAX_RESYNC_BYTES,
        }
    }
}

impl Limits {
    /// An empty cap set carrying the documented hostile-input defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_element_bytes: DEFAULT_MAX_ELEMENT_BYTES,
            max_children: DEFAULT_MAX_CHILDREN,
            max_master_bytes: DEFAULT_MAX_MASTER_BYTES,
            max_tracks: DEFAULT_MAX_TRACKS,
            max_lace_frames: DEFAULT_MAX_LACE_FRAMES,
            max_cues: DEFAULT_MAX_CUES,
            max_clusters: DEFAULT_MAX_CLUSTERS,
            max_block_header_bytes: DEFAULT_MAX_BLOCK_HEADER_BYTES,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            max_resync_bytes: DEFAULT_MAX_RESYNC_BYTES,
        }
    }

    /// Set the maximum element-tree depth.
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Set the maximum size of one element's payload, in bytes.
    #[must_use]
    pub const fn with_max_element_bytes(mut self, max_element_bytes: u64) -> Self {
        self.max_element_bytes = max_element_bytes;
        self
    }

    /// Set the maximum children in one master element.
    #[must_use]
    pub const fn with_max_children(mut self, max_children: u32) -> Self {
        self.max_children = max_children;
        self
    }

    /// The maximum element-tree depth.
    #[must_use]
    pub const fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// The maximum size of one element's payload, in bytes.
    #[must_use]
    pub const fn max_element_bytes(&self) -> u64 {
        self.max_element_bytes
    }

    /// The maximum children in one master element.
    #[must_use]
    pub const fn max_children(&self) -> u32 {
        self.max_children
    }

    /// Set the maximum bytes of one buffered level-1 master element.
    #[must_use]
    pub const fn with_max_master_bytes(mut self, max_master_bytes: u64) -> Self {
        self.max_master_bytes = max_master_bytes;
        self
    }

    /// Set the maximum tracks in one Segment.
    #[must_use]
    pub const fn with_max_tracks(mut self, max_tracks: u32) -> Self {
        self.max_tracks = max_tracks;
        self
    }

    /// Set the maximum frames one laced Block may carry.
    #[must_use]
    pub const fn with_max_lace_frames(mut self, max_lace_frames: u32) -> Self {
        self.max_lace_frames = max_lace_frames;
        self
    }

    /// Set the maximum `CuePoint` entries held for seeking.
    #[must_use]
    pub const fn with_max_cues(mut self, max_cues: u32) -> Self {
        self.max_cues = max_cues;
        self
    }

    /// Set the maximum Cluster headers retained for a cue-less seek.
    #[must_use]
    pub const fn with_max_clusters(mut self, max_clusters: u32) -> Self {
        self.max_clusters = max_clusters;
        self
    }

    /// Set the maximum bytes of one Block header prefetch.
    #[must_use]
    pub const fn with_max_block_header_bytes(mut self, max_block_header_bytes: u64) -> Self {
        self.max_block_header_bytes = max_block_header_bytes;
        self
    }

    /// Set the maximum bytes one reader demand may ask a driver for at once.
    #[must_use]
    pub const fn with_max_read_bytes(mut self, max_read_bytes: u64) -> Self {
        self.max_read_bytes = max_read_bytes;
        self
    }

    /// Set the maximum bytes a permissive reader scans for the next level-1 id.
    #[must_use]
    pub const fn with_max_resync_bytes(mut self, max_resync_bytes: u64) -> Self {
        self.max_resync_bytes = max_resync_bytes;
        self
    }

    /// The maximum bytes of one buffered level-1 master element.
    #[must_use]
    pub const fn max_master_bytes(&self) -> u64 {
        self.max_master_bytes
    }

    /// The maximum tracks in one Segment.
    #[must_use]
    pub const fn max_tracks(&self) -> u32 {
        self.max_tracks
    }

    /// The maximum frames one laced Block may carry.
    #[must_use]
    pub const fn max_lace_frames(&self) -> u32 {
        self.max_lace_frames
    }

    /// The maximum `CuePoint` entries held for seeking.
    #[must_use]
    pub const fn max_cues(&self) -> u32 {
        self.max_cues
    }

    /// The maximum Cluster headers retained for a cue-less seek.
    #[must_use]
    pub const fn max_clusters(&self) -> u32 {
        self.max_clusters
    }

    /// The maximum bytes of one Block header prefetch.
    #[must_use]
    pub const fn max_block_header_bytes(&self) -> u64 {
        self.max_block_header_bytes
    }

    /// The maximum bytes one reader demand may ask a driver for at once.
    #[must_use]
    pub const fn max_read_bytes(&self) -> u64 {
        self.max_read_bytes
    }

    /// The maximum bytes a permissive reader scans for the next level-1 id.
    #[must_use]
    pub const fn max_resync_bytes(&self) -> u64 {
        self.max_resync_bytes
    }
}
