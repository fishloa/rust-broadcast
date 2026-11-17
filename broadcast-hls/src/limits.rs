//! Hostile-input bounds for playlist parsing.
//!
//! A playlist is fetched from an origin the player does not control, so its
//! byte size and segment count have a named ceiling and a typed error
//! ([`crate::Error::TooLarge`]) instead of an unbounded allocation (audit 20
//! finding 41, issue #2852).

/// Largest playlist document accepted, in bytes.
pub const MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
/// Most closed Media Segments accepted in one playlist. A one-line segment is
/// at least a few bytes, so a playlist within [`MAX_PLAYLIST_BYTES`] can hold
/// at most a few hundred thousand; this cap is the backstop, not the working
/// bound, and admits a 150 000-segment hostile playlist within the byte cap.
pub const MAX_SEGMENTS: usize = 1_000_000;
/// Largest resource body (init, part or segment) accepted by a client.
pub const MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;

/// The bounds applied by [`crate::MediaPlaylist::parse_with`] and
/// [`crate::MasterPlaylist::parse_with`]; `Default` is the `MAX_*` constants
/// above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// See [`MAX_PLAYLIST_BYTES`].
    pub max_playlist_bytes: usize,
    /// See [`MAX_SEGMENTS`].
    pub max_segments: usize,
    /// See [`MAX_SEGMENT_BYTES`].
    pub max_segment_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_playlist_bytes: MAX_PLAYLIST_BYTES,
            max_segments: MAX_SEGMENTS,
            max_segment_bytes: MAX_SEGMENT_BYTES,
        }
    }
}
