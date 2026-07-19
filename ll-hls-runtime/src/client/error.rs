//! Error type returned by [`crate::client::LlHlsClient`].

use alloc::string::String;
use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants [`crate::client::LlHlsClient`] can return.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The playlist bytes fed to [`crate::client::LlHlsClient::on_playlist`]
    /// are not valid UTF-8 (RFC 8216 §4.1 playlists are UTF-8 text).
    #[error("playlist is not valid UTF-8: {0}")]
    PlaylistNotUtf8(#[from] core::str::Utf8Error),

    /// `transmux::hls::MediaPlaylist::parse` rejected the playlist text (a
    /// known tag with a missing/unparsable required attribute).
    #[error("playlist parse: {0}")]
    PlaylistParse(#[from] transmux::Error),

    /// [`crate::client::LlHlsClient::on_resource`] was fed bytes for a
    /// [`crate::client::ResourceId`] the client never requested (a
    /// caller/driver bug, or a stale/duplicate delivery after the client
    /// already moved past it).
    #[error("resource delivered for an id the client did not request: {id:?}")]
    UnrequestedResource {
        /// The unexpected id.
        id: super::action::ResourceId,
    },

    /// Defensive-only: a Part/Segment reached the demux step with no init
    /// segment cached. In normal operation this cannot happen —
    /// [`crate::client::LlHlsClient::on_resource`] buffers any Part/Segment
    /// that arrives before the init segment and replays it once the init is
    /// delivered, so callers may complete fetches in any order.
    #[error("resource {id:?} delivered before the init segment was available")]
    InitNotYetAvailable {
        /// The id that could not be demuxed yet.
        id: super::action::ResourceId,
    },

    /// Demuxing the concatenation of the init segment + a fetched Part/Segment
    /// resource (via `transmux::Fmp4Demux`) failed.
    #[error("demux of resource {id:?} failed: {source}")]
    Demux {
        /// The id whose bytes failed to demux.
        id: super::action::ResourceId,
        /// The underlying transmux error.
        #[source]
        source: transmux::Error,
    },

    /// A playlist URI (segment/part/map/rendition-report) could not be
    /// resolved against the playlist's own URL.
    #[error("could not resolve URI {uri:?} against base {base:?}: {reason}")]
    UriResolve {
        /// The (possibly relative) URI from the playlist.
        uri: String,
        /// The base URL it was resolved against.
        base: String,
        /// Human-readable explanation.
        reason: &'static str,
    },
}
