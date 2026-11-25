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

/// Hostile-input caps for the element walker and typed parsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    max_depth: u32,
    max_element_bytes: u64,
    max_children: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_element_bytes: DEFAULT_MAX_ELEMENT_BYTES,
            max_children: DEFAULT_MAX_CHILDREN,
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
}
