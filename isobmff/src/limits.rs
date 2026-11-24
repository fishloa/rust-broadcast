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

/// Hostile-input caps for the reader and writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    max_depth: u32,
    max_moov_bytes: u64,
    max_samples_per_track: u32,
    max_sample_bytes: u32,
    max_fragment_bytes: u64,
    max_metadata_item_bytes: u64,
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
    }

    #[test]
    fn setters_replace_one_field_at_a_time() {
        let limits = Limits::default()
            .with_max_depth(3)
            .with_max_moov_bytes(1024)
            .with_max_samples_per_track(7)
            .with_max_sample_bytes(9)
            .with_max_fragment_bytes(11)
            .with_max_metadata_item_bytes(13);
        assert_eq!(limits.max_depth(), 3);
        assert_eq!(limits.max_moov_bytes(), 1024);
        assert_eq!(limits.max_samples_per_track(), 7);
        assert_eq!(limits.max_sample_bytes(), 9);
        assert_eq!(limits.max_fragment_bytes(), 11);
        assert_eq!(limits.max_metadata_item_bytes(), 13);
    }
}
