//! Options for [`crate::MediaPlaylist::parse_with_options`].
//!
//! Unmodelled tags are normally playlist-level: `MediaPlaylist::parse` keeps
//! them verbatim in [`crate::MediaPlaylist::extra_tags`] and re-renders them
//! before the first segment. A vendor tag that describes one segment instead
//! (the DVR archive's per-segment timing tag) has to stay in that segment's
//! position — RFC 8216bis §4.4.1 makes a tag's position relative to segments
//! load-bearing, `private/specs-verified/hls/ietf/rfc8216bis-playlist-tags.md`
//! — so the caller names it here and parsing attaches it to the
//! following segment's [`crate::MediaSegment::pre_tags`].

use alloc::vec::Vec;

use crate::CustomTagName;

/// Playlist parse options. `Default` (no segment-scoped vendor tags) is
/// exactly [`crate::MediaPlaylist::parse`]'s behaviour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseOptions {
    /// Vendor tags that apply to the following segment rather than the whole
    /// playlist, kept in that segment's position on re-render.
    pub segment_scoped_tags: Vec<CustomTagName>,
}

impl ParseOptions {
    /// Add a vendor tag name to [`Self::segment_scoped_tags`].
    #[must_use]
    pub fn with_segment_scoped_tag(mut self, tag: CustomTagName) -> Self {
        self.segment_scoped_tags.push(tag);
        self
    }

    /// Whether `name` (without the leading `#`) is declared segment-scoped.
    #[must_use]
    pub fn is_segment_scoped(&self, name: &str) -> bool {
        self.segment_scoped_tags
            .iter()
            .any(|tag| tag.as_str() == name)
    }
}
