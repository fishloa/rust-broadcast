//! What the engine hands back: [`Output`], drained with
//! [`super::SmoothClient::next_output`].

use broadcast_common::impl_spec_display;
use transmux::{Sample, TrackId, TrackSpec};

use super::action::StreamId;
use crate::error::SmoothError;

/// One selectable video `QualityLevel`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QualityInfo {
    /// `QualityLevel@Bitrate`, bits per second.
    pub bitrate: u64,
    /// `(MaxWidth, MaxHeight)`.
    pub resolution: Option<(u32, u32)>,
    /// The RFC 6381 codec string, when it can be derived.
    pub codecs: Option<String>,
}

/// Why a manifest could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestFault {
    /// The fetch failed (transport, timeout or HTTP status).
    Fetch,
    /// The body is not UTF-8.
    Utf8,
    /// The body is not a parseable Smooth manifest.
    Parse,
    /// A chunk timeline could not be enumerated.
    Timeline,
}

impl ManifestFault {
    /// A stable machine-readable label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Utf8 => "utf8",
            Self::Parse => "parse",
            Self::Timeline => "timeline",
        }
    }
}

impl_spec_display!(ManifestFault);

/// Something the engine gave up on and carried on without.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dropped {
    /// A manifest fetch or parse failed and will be retried.
    Manifest(ManifestFault),
    /// A live fragment kept failing and was skipped.
    Fragment {
        /// The stream it belonged to.
        stream: StreamId,
        /// The final HTTP status, when HTTP failed it.
        status: Option<u16>,
    },
    /// A fragment URL template did not resolve against the manifest URL.
    FragmentUrl {
        /// The stream it belonged to.
        stream: StreamId,
    },
    /// A stream vanished from the refreshed manifest for too long and was dropped.
    StreamLost {
        /// The stream that was dropped.
        stream: StreamId,
    },
}

/// One thing the host should act on.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Output {
    /// The video `QualityLevel`s on offer and the index chosen.
    Qualities {
        /// One entry per `QualityLevel` of the first video stream.
        qualities: Vec<QualityInfo>,
        /// Index into `qualities`.
        chosen: usize,
    },
    /// The first manifest fetch answered 404/410.
    ManifestNotFound,
    /// Every stream's first fragment resolved: the track set, one per stream, ids
    /// `1..` in stream order.
    Tracks(Vec<TrackSpec>),
    /// One demuxed sample, timestamps in its track's timescale.
    Sample {
        /// The track carrying it.
        track: TrackId,
        /// The sample.
        sample: Sample,
    },
    /// A gap in the timeline: samples were skipped or the live window moved past
    /// the last fragment.
    Discontinuity,
    /// Carried on without something.
    Dropped(Dropped),
    /// A static presentation played to its end.
    Ended,
    /// The session is over.
    Fatal(SmoothError),
}
