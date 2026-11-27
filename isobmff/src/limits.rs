//! Hostile-input caps for the Phase 2 reader (streaming-first design §4.5).
//!
//! A crate that allocates whatever a downloaded file claims is not safe to hand
//! a download. Every count or size this crate reads from input is checked
//! against one of these caps before anything is allocated from it, so a hostile
//! header can only ever produce a typed [`Error::LimitExceeded`](crate::Error),
//! never an allocation it dictates.
//!
//! Two kinds of field live here and must not be confused:
//!
//! - **Hostile-input caps** bound what a count or size read from the file may
//!   make the crate allocate: walk depth, `moov` size, samples per track, one
//!   sample's size, one fragment's size, one metadata item's size. Their
//!   defaults are named [`DEFAULT_MAX_*`](Limits::default) constants.
//! - **Caller policy** is never chosen by this crate. The progressive writer's
//!   reorder buffer for an append-only sink has no default here; that
//!   constructor takes the cap as an argument.
//!
//! Fields are private and set through the [`with_*`](Limits::with_max_depth)
//! builders, so a caller reads the cap it cares about and leaves the rest at
//! their safe defaults.

/// Bytes in one kibibyte.
const KIB: u64 = 1024;
/// Bytes in one mebibyte.
const MIB: u64 = KIB * KIB;

/// Default maximum box-tree walk depth.
pub const DEFAULT_MAX_DEPTH: u32 = 16;
/// Default maximum `moov` box size, in bytes (256 MiB).
pub const DEFAULT_MAX_MOOV_BYTES: u64 = 256 * MIB;
/// Default maximum samples in one track's sample table.
pub const DEFAULT_MAX_SAMPLES_PER_TRACK: u32 = 50_000_000;
/// Default maximum size of one sample, in bytes (64 MiB).
pub const DEFAULT_MAX_SAMPLE_BYTES: u32 = (64 * MIB) as u32;
/// Default maximum size of one fragment (`moof` + `mdat`), in bytes (64 MiB).
pub const DEFAULT_MAX_FRAGMENT_BYTES: u64 = 64 * MIB;
/// Default maximum size of one metadata item, in bytes (16 MiB).
pub const DEFAULT_MAX_METADATA_ITEM_BYTES: u64 = 16 * MIB;
/// Default maximum number of items in one `MetaBox` — unlimited; the item
/// layer's counts are supplied by the caller and this crate picks no policy
/// number for them.
pub const DEFAULT_UNLIMITED_ITEM_COUNT: u32 = u32::MAX;
/// Default maximum number of item properties in one `ipco` — unlimited.
pub const DEFAULT_UNLIMITED_PROPERTY_COUNT: u32 = u32::MAX;
/// Default maximum number of item references — unlimited.
pub const DEFAULT_UNLIMITED_REFERENCE_COUNT: u32 = u32::MAX;

/// Hostile-input caps for the reader and writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    max_depth: u32,
    max_moov_bytes: u64,
    max_samples_per_track: u32,
    max_sample_bytes: u32,
    max_fragment_bytes: u64,
    max_metadata_item_bytes: u64,
    max_items: u32,
    max_properties: u32,
    max_references: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            max_moov_bytes: DEFAULT_MAX_MOOV_BYTES,
            max_samples_per_track: DEFAULT_MAX_SAMPLES_PER_TRACK,
            max_sample_bytes: DEFAULT_MAX_SAMPLE_BYTES,
            max_fragment_bytes: DEFAULT_MAX_FRAGMENT_BYTES,
            max_metadata_item_bytes: DEFAULT_MAX_METADATA_ITEM_BYTES,
            max_items: DEFAULT_UNLIMITED_ITEM_COUNT,
            max_properties: DEFAULT_UNLIMITED_PROPERTY_COUNT,
            max_references: DEFAULT_UNLIMITED_REFERENCE_COUNT,
        }
    }
}

impl Limits {
    /// Set the maximum box-tree walk depth.
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Set the maximum `moov` box size, in bytes.
    #[must_use]
    pub const fn with_max_moov_bytes(mut self, max_moov_bytes: u64) -> Self {
        self.max_moov_bytes = max_moov_bytes;
        self
    }

    /// Set the maximum samples in one track's sample table.
    #[must_use]
    pub const fn with_max_samples_per_track(mut self, max_samples_per_track: u32) -> Self {
        self.max_samples_per_track = max_samples_per_track;
        self
    }

    /// Set the maximum size of one sample, in bytes.
    #[must_use]
    pub const fn with_max_sample_bytes(mut self, max_sample_bytes: u32) -> Self {
        self.max_sample_bytes = max_sample_bytes;
        self
    }

    /// Set the maximum size of one fragment (`moof` + `mdat`), in bytes.
    #[must_use]
    pub const fn with_max_fragment_bytes(mut self, max_fragment_bytes: u64) -> Self {
        self.max_fragment_bytes = max_fragment_bytes;
        self
    }

    /// Set the maximum size of one metadata item, in bytes.
    #[must_use]
    pub const fn with_max_metadata_item_bytes(mut self, max_metadata_item_bytes: u64) -> Self {
        self.max_metadata_item_bytes = max_metadata_item_bytes;
        self
    }

    /// Set the maximum number of items in one `MetaBox` (unlimited by
    /// default).
    #[must_use]
    pub const fn with_max_items(mut self, max_items: u32) -> Self {
        self.max_items = max_items;
        self
    }

    /// Set the maximum number of item properties in one `ipco` (unlimited by
    /// default).
    #[must_use]
    pub const fn with_max_properties(mut self, max_properties: u32) -> Self {
        self.max_properties = max_properties;
        self
    }

    /// Set the maximum number of item references (unlimited by default).
    #[must_use]
    pub const fn with_max_references(mut self, max_references: u32) -> Self {
        self.max_references = max_references;
        self
    }

    /// The maximum box-tree walk depth.
    #[must_use]
    pub const fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// The maximum `moov` box size, in bytes.
    #[must_use]
    pub const fn max_moov_bytes(&self) -> u64 {
        self.max_moov_bytes
    }

    /// The maximum samples in one track's sample table.
    #[must_use]
    pub const fn max_samples_per_track(&self) -> u32 {
        self.max_samples_per_track
    }

    /// The maximum size of one sample, in bytes.
    #[must_use]
    pub const fn max_sample_bytes(&self) -> u32 {
        self.max_sample_bytes
    }

    /// The maximum size of one fragment (`moof` + `mdat`), in bytes.
    #[must_use]
    pub const fn max_fragment_bytes(&self) -> u64 {
        self.max_fragment_bytes
    }

    /// The maximum size of one metadata item, in bytes.
    #[must_use]
    pub const fn max_metadata_item_bytes(&self) -> u64 {
        self.max_metadata_item_bytes
    }

    /// The maximum number of items in one `MetaBox`.
    #[must_use]
    pub const fn max_items(&self) -> u32 {
        self.max_items
    }

    /// The maximum number of item properties in one `ipco`.
    #[must_use]
    pub const fn max_properties(&self) -> u32 {
        self.max_properties
    }

    /// The maximum number of item references.
    #[must_use]
    pub const fn max_references(&self) -> u32 {
        self.max_references
    }

    /// The largest structure range the reader can demand: the larger of a
    /// whole `moov`/index box and a whole fragment.
    #[must_use]
    pub const fn max_need_bytes(&self) -> u64 {
        if self.max_moov_bytes > self.max_fragment_bytes {
            self.max_moov_bytes
        } else {
            self.max_fragment_bytes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_values() {
        let limits = Limits::default();
        assert_eq!(limits.max_depth(), 16);
        assert_eq!(limits.max_moov_bytes(), 256 * 1024 * 1024);
        assert_eq!(limits.max_samples_per_track(), 50_000_000);
        assert_eq!(limits.max_sample_bytes(), 64 * 1024 * 1024);
        assert_eq!(limits.max_fragment_bytes(), 64 * 1024 * 1024);
        assert_eq!(limits.max_metadata_item_bytes(), 16 * 1024 * 1024);
        assert_eq!(limits.max_items(), u32::MAX);
        assert_eq!(limits.max_properties(), u32::MAX);
        assert_eq!(limits.max_references(), u32::MAX);
    }

    #[test]
    fn setters_replace_one_field_at_a_time() {
        let limits = Limits::default()
            .with_max_depth(3)
            .with_max_moov_bytes(1024)
            .with_max_samples_per_track(7)
            .with_max_sample_bytes(9)
            .with_max_fragment_bytes(11)
            .with_max_metadata_item_bytes(13)
            .with_max_items(15)
            .with_max_properties(17)
            .with_max_references(19);
        assert_eq!(limits.max_depth(), 3);
        assert_eq!(limits.max_moov_bytes(), 1024);
        assert_eq!(limits.max_samples_per_track(), 7);
        assert_eq!(limits.max_sample_bytes(), 9);
        assert_eq!(limits.max_fragment_bytes(), 11);
        assert_eq!(limits.max_metadata_item_bytes(), 13);
        assert_eq!(limits.max_items(), 15);
        assert_eq!(limits.max_properties(), 17);
        assert_eq!(limits.max_references(), 19);
    }
}
