//! What the client hands back: [`Output`], drained with
//! [`super::DashClient::next_output`].

use broadcast_common::impl_spec_display;
use transmux::{Sample, TrackId, TrackSpec};

use super::action::ResourceId;
use super::error::FatalKind;
use crate::error::Unsupported;
use crate::selection::{DropReason, RepRef};
use crate::timing::clock::ClockSource;

/// One Representation of a selectable video AdaptationSet, in the shape a
/// host's variant list wants (DESIGN D11).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct VariantInfo {
    /// Where it is in the MPD.
    pub rep: RepRef,
    /// Absolute URL of its Initialization Segment, when it has one.
    pub init_url: Option<String>,
    /// `@bandwidth`, bits per second.
    pub bandwidth: u64,
    /// `(width, height)`.
    pub resolution: Option<(u32, u32)>,
    /// `@codecs`.
    pub codecs: Option<String>,
    /// The `@group` (else the `@id`) of the audio AdaptationSet it pairs
    /// with, when the Period has audio.
    pub audio_group: Option<String>,
}

/// One selectable audio AdaptationSet.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AudioRendition {
    /// AdaptationSet index in the Period.
    pub set: usize,
    /// `@group`, else `@id`.
    pub group: Option<String>,
    /// First `Label`, else `@lang`, else the first Representation's `@id`.
    pub name: String,
    /// `@lang`.
    pub language: Option<String>,
    /// Role `main` (DASH has no DEFAULT flag).
    pub default: bool,
    /// Always `true`: DASH has no AUTOSELECT flag, every set may be chosen.
    pub autoselect: bool,
    /// Absolute init URL of the chosen (else first) Representation.
    pub init_url: Option<String>,
    /// The selectable Representations.
    pub reps: Vec<RepRef>,
}

/// The selection made for one Period.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ManifestSelection {
    /// Period index in the MPD.
    pub period: usize,
    /// Every selectable video Representation, MPD order.
    pub variants: Vec<VariantInfo>,
    /// Index into `variants` of the one playing.
    pub chosen: Option<usize>,
    /// Selectable audio sets.
    pub audio: Vec<AudioRendition>,
    /// Index into `audio` of the one playing.
    pub chosen_audio: Option<usize>,
}

/// Why a set or Representation is not played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropCause {
    /// Selection refused it (text, TS, WebM, trick mode, encryption, ...).
    Selection(DropReason),
    /// No `Initialization`: self-initializing segments are not supported.
    NoInitialization,
    /// A static Representation whose segment count cannot be known.
    UnboundedPlan,
    /// The segment information cannot be addressed (`name` of the error).
    Addressing(&'static str),
    /// A URL could not be built.
    UnusableUrl,
    /// A URL left the MPD origin while `cross_origin_urls` is off.
    CrossOrigin,
    /// A URL used a scheme this client does not fetch.
    Unsupported(Unsupported),
}

impl DropCause {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Selection(r) => r.name(),
            Self::NoInitialization => "no_initialization",
            Self::UnboundedPlan => "unbounded_plan",
            Self::Addressing(n) => n,
            Self::UnusableUrl => "unusable_url",
            Self::CrossOrigin => "cross_origin",
            Self::Unsupported(u) => u.name(),
        }
    }
}
impl_spec_display!(DropCause);

/// A non-fatal, once-per-session observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Notice {
    /// A `ContentProtection` element: encrypted content is not decrypted.
    UnsupportedEncryption,
    /// `EventStream` / `InbandEventStream` present and not surfaced.
    EventStreamIgnored,
    /// `Preselection` present and not used.
    PreselectionIgnored,
    /// A remote element (`xlink:href`) was skipped.
    XlinkSkipped,
    /// `PatchLocation` present; patches are not used.
    PatchIgnored,
    /// A low-latency MPD (`availabilityTimeOffset` or
    /// `availabilityTimeComplete="false"`): played at segment granularity.
    LowLatencyIgnored,
    /// The client requested segments too slowly: the segments it wanted left
    /// the time-shift window and it continued at the oldest one still there.
    FellBehind,
    /// A `UTCTiming` element this client cannot act on (`ntp`, `sntp`, an
    /// unknown scheme) was skipped.
    UtcSchemeUnsupported,
    /// A `UTCTiming` URL left the MPD origin while `cross_origin_urls` is
    /// off and was skipped.
    UtcCrossOrigin,
}

impl Notice {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnsupportedEncryption => "unsupported_encryption",
            Self::EventStreamIgnored => "event_stream_ignored",
            Self::PreselectionIgnored => "preselection_ignored",
            Self::XlinkSkipped => "xlink_skipped",
            Self::PatchIgnored => "patch_ignored",
            Self::LowLatencyIgnored => "low_latency_ignored",
            Self::FellBehind => "fell_behind",
            Self::UtcSchemeUnsupported => "utc_scheme_unsupported",
            Self::UtcCrossOrigin => "utc_cross_origin",
        }
    }
}
impl_spec_display!(Notice);

/// One unit of output.
///
/// Per Period the order is: `Notice`/`TrackDropped` for what was left out,
/// `Manifest`, `PeriodChanged`, then (when the Period does not continue the
/// previous one) `Discontinuity`, `Init` per changed init, `Tracks` when the
/// track set changed, and the `Samples`. `Samples` of one track are in decode
/// order with no gap and no repeat; their timestamps are the container's
/// (`tfdt`) timeline, not a Period-rebased one.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Output {
    /// The selection made for a Period.
    Manifest(ManifestSelection),
    /// A Period starts playing.
    PeriodChanged {
        /// Period index in the MPD.
        index: usize,
        /// `Period@id`.
        id: Option<String>,
        /// PeriodStart, microseconds from the presentation start.
        start_us: i128,
        /// `true` only for a boundary signalled period-continuous
        /// (`urn:mpeg:dash:period-continuity:2015`, 23009-1 5.3.2.4) whose
        /// init is unchanged: nothing is re-announced and no `Discontinuity`
        /// is emitted. Always `false` for the first Period.
        continuous: bool,
    },
    /// The Initialization Segment bytes (`ftyp` + `moov`) a decoder takes
    /// before the samples that follow.
    Init(Vec<u8>),
    /// The full track set; session track ids are 1-based in selection order
    /// (video, then audio) and survive a Representation switch.
    Tracks(Vec<TrackSpec>),
    /// Access units of one track, in decode order.
    Samples {
        /// Session track id.
        track_id: TrackId,
        /// The samples.
        samples: Vec<Sample>,
    },
    /// The timeline may jump: a Period that does not continue the previous
    /// one, a skipped segment, or a Representation switch that cannot be
    /// aligned.
    Discontinuity,
    /// A set or Representation is not played.
    TrackDropped {
        /// Period index.
        period: usize,
        /// AdaptationSet index.
        set: usize,
        /// Representation index when only it was dropped.
        rep: Option<usize>,
        /// Why.
        cause: DropCause,
    },
    /// A track of an init that the demuxer could not model (an encrypted or
    /// unknown sample entry); once per track and init.
    TrackSkipped {
        /// Session track id, when it maps to one.
        track_id: Option<TrackId>,
        /// The sample-entry four-character code.
        fourcc: String,
        /// The demuxer's reason.
        reason: String,
    },
    /// The demuxer skipped fragments of a segment; the others were emitted.
    FragmentDamaged {
        /// The segment.
        id: ResourceId,
        /// How many fragments were skipped.
        skipped: usize,
    },
    /// A non-fatal observation.
    Notice(Notice),
    /// The UTC clock has its first anchor, or its source changed (23009-1
    /// 5.8.5.7; DESIGN 3.4). Not emitted for every refinement.
    ClockSynced {
        /// Where the anchor came from.
        source: ClockSource,
        /// Half-width of its error bar, microseconds.
        uncertainty_us: u64,
    },
    /// A segment of a live presentation was given up on after the configured
    /// number of failures; a `Discontinuity` marks the gap. (A static
    /// presentation ends with a fatal error instead.)
    SegmentLost {
        /// The segment.
        id: ResourceId,
        /// Status of the last failure, when it was an HTTP status.
        status: Option<u16>,
    },
    /// The presentation cannot continue; nothing follows.
    Fatal(FatalKind),
    /// Every selected segment was delivered.
    EndOfStream,
}
