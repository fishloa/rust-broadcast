//! Actions the caller must perform IO for — [`Action`] out of
//! [`crate::client::LlHlsClient::poll`].

use alloc::format;
use alloc::string::String;

/// Identifies one fetchable resource: the initialisation segment, a Low-Latency
/// HLS partial segment ("part", RFC 8216bis §4.4.4.9), or a whole media
/// segment. Used to correlate an [`Action::FetchResource`] with the matching
/// [`crate::client::LlHlsClient::on_resource`] call.
///
/// `Part`/`Segment` are keyed by Media Sequence Number (RFC 8216 §4.3.3.2) +
/// (for parts) the 0-based Part Index within that segment — stable identity
/// independent of the URI, since a delta/blocking reload can re-describe the
/// same resource with the same URI more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ResourceId {
    /// The Media Initialisation Section (`EXT-X-MAP`, RFC 8216bis §4.4.4.5).
    Init,
    /// One partial segment (RFC 8216bis §4.4.4.9): `msn`'s Media Sequence
    /// Number, `part` its 0-based Part Index.
    Part {
        /// Media Sequence Number of the parent (possibly still-open) segment.
        msn: u64,
        /// 0-based Part Index within that segment.
        part: u64,
    },
    /// One whole media segment (RFC 8216 §4.3.3, non-LL full-segment
    /// fallback — a segment whose parts were never individually fetched).
    Segment {
        /// Media Sequence Number.
        msn: u64,
    },
}

/// A pending blocking Playlist Reload request (RFC 8216bis §6.2.5.2): the
/// client has consumed the playlist up to (and possibly including) a given
/// Partial Segment and wants the server to hold the response until something
/// newer exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockingReload {
    /// `_HLS_msn` — Media Sequence Number of the next segment (open or not
    /// yet begun) the client wants.
    pub msn: u64,
    /// `_HLS_part` — 0-based Part Index within `msn`. `None` means a bare
    /// `_HLS_msn` request: RFC 8216bis §6.2.5.2 says this waits for `msn` to
    /// become a **closed** Media Segment, distinct from `_HLS_part=0` (which
    /// is satisfied by just the first part).
    pub part: Option<u64>,
}

/// One unit of IO the caller must perform, in response to a
/// [`crate::client::LlHlsClient::poll`] call. The client core never touches a socket
/// or a clock itself — every `Action` names exactly what to fetch and, for a
/// playlist reload, how to shape the request.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Action {
    /// Fetch (or long-poll) the Media Playlist.
    FetchPlaylist {
        /// The playlist URL.
        url: String,
        /// `Some` to add `_HLS_msn`/`_HLS_part` query parameters requesting a
        /// Blocking Playlist Reload (RFC 8216bis §6.2.5.2); `None` for a
        /// plain (non-blocking) GET — used when the last-seen playlist did
        /// not advertise `CAN-BLOCK-RELOAD` support.
        blocking: Option<BlockingReload>,
        /// Add `_HLS_skip=YES` (RFC 8216bis §6.2.5.1) requesting a Playlist
        /// Delta Update — only set once the client has already seen a full
        /// playlist to reconstruct a delta response against, and the last
        /// playlist advertised `CAN-SKIP-UNTIL`.
        skip: bool,
    },
    /// Fetch one resource (init/part/segment).
    FetchResource {
        /// Correlates the eventual [`crate::client::LlHlsClient::on_resource`] call.
        id: ResourceId,
        /// The resource URL (already resolved against the playlist URL).
        url: String,
        /// `Some((offset, length))` to fetch only that byte sub-range
        /// (RFC 8216bis §4.4.4.2/§4.4.4.9's `EXT-X-BYTERANGE`/`BYTERANGE`);
        /// `None` fetches the entire resource.
        byte_range: Option<(u64, u64)>,
    },
    /// A non-blocking-reload timing hint: wait about this many milliseconds
    /// before issuing the next `FetchPlaylist`, derived from
    /// `#EXT-X-TARGETDURATION` (RFC 8216 §4.3.3.1's "SHOULD NOT ... more
    /// frequently than once every Target Duration" reload guidance, halved
    /// for headroom) when the origin does not support blocking reload.
    WaitMs(u64),
}

impl Action {
    /// For [`Action::FetchPlaylist`], the fully query-augmented URL to fetch:
    /// `_HLS_msn`/`_HLS_part` (RFC 8216bis §6.2.5.2) appended per
    /// [`Self::FetchPlaylist`]'s `blocking` field, then `_HLS_skip=YES`
    /// (RFC 8216bis §6.2.5.1) if `skip` is set. `None` for
    /// [`Action::FetchResource`]/[`Action::WaitMs`] (nothing to augment —
    /// use their `url`/byte-range fields directly).
    ///
    /// This is a convenience for building the real HTTP request; the typed
    /// `blocking`/`skip` fields remain the source of truth (e.g. for a
    /// caller using a query-parameter API rather than string concatenation).
    pub fn playlist_request_url(&self) -> Option<String> {
        match self {
            Action::FetchPlaylist {
                url,
                blocking,
                skip,
            } => {
                let mut u = url.clone();
                if let Some(b) = blocking {
                    u = super::url::append_query(&u, &format!("_HLS_msn={}", b.msn));
                    if let Some(part) = b.part {
                        u = super::url::append_query(&u, &format!("_HLS_part={part}"));
                    }
                }
                if *skip {
                    u = super::url::append_query(&u, "_HLS_skip=YES");
                }
                Some(u)
            }
            Action::FetchResource { .. } | Action::WaitMs(_) => None,
        }
    }
}
