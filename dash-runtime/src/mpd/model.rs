//! The consume-only MPD model (ISO/IEC 23009-1 clause 5.3, Annex B schema).
//!
//! Every field is typed; nothing keeps source text for a later passthrough.
//! Times are Unix microseconds (`*_us`, `i64`) or [`Duration`]. Element
//! content the core does not act on (`EventStream`, `Preselection`, ...) is
//! recorded only as a flag in [`MpdFeatures`] so a caller can report it.

use std::time::Duration;

use broadcast_common::impl_spec_display;

use crate::error::Unsupported;
use crate::timing::utc_timing::UtcScheme;

/// `MPD@type` (Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PresentationType {
    /// `static` (the default): the whole presentation is available.
    #[default]
    Static,
    /// `dynamic`: segments become available over wall-clock time.
    Dynamic,
}

impl PresentationType {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
        }
    }
}
impl_spec_display!(PresentationType);

/// `@xlink:actuate` (5.5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Actuate {
    /// Resolve when the MPD is processed.
    OnLoad,
    /// Resolve when needed (the default).
    #[default]
    OnRequest,
}

impl Actuate {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::OnLoad => "onLoad",
            Self::OnRequest => "onRequest",
        }
    }
}
impl_spec_display!(Actuate);

/// An `xlink` reference on a Period, AdaptationSet or SegmentList.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Xlink {
    /// `@xlink:href`.
    pub href: String,
    /// `@xlink:actuate`.
    pub actuate: Actuate,
}

/// A single byte range `first-last` (RFC 7233 2.1), inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ByteRange {
    /// First byte.
    pub first: u64,
    /// Last byte; `None` for the open-ended form `first-`.
    pub last: Option<u64>,
}

impl ByteRange {
    /// A closed range.
    #[must_use]
    pub fn new(first: u64, last: u64) -> Self {
        Self {
            first,
            last: Some(last),
        }
    }

    /// Number of bytes; `None` when open-ended or `last < first`.
    #[must_use]
    pub fn len(&self) -> Option<u64> {
        let last = self.last?;
        last.checked_sub(self.first)?.checked_add(1)
    }

    /// Whether the range is empty or open-ended (no byte count).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len().is_none()
    }

    /// The closed `(offset, length)` form, or the typed unsupported error for
    /// an open-ended range.
    ///
    /// # Errors
    /// [`Unsupported::OpenEndedRange`].
    pub fn offset_len(&self) -> Result<(u64, u64), Unsupported> {
        self.len()
            .map(|n| (self.first, n))
            .ok_or(Unsupported::OpenEndedRange)
    }
}

/// `URLType` (Table 18): an `Initialization`/`RepresentationIndex` address.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct UrlType {
    /// `@sourceURL`; absent means the BaseURL itself.
    pub source_url: Option<String>,
    /// `@range`.
    pub range: Option<ByteRange>,
}

/// A `BaseURL` (Table 30) with the DVB extension attributes.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BaseUrl {
    /// Element text.
    pub url: String,
    /// `@serviceLocation`.
    pub service_location: Option<String>,
    /// `@dvb:priority` (ETSI TS 103 285 10.8.2.1).
    pub priority: Option<u32>,
    /// `@dvb:weight`; parsed, never used (the core is deterministic).
    pub weight: Option<u32>,
    /// `@availabilityTimeOffset`, seconds.
    pub availability_time_offset: Option<f64>,
    /// `@availabilityTimeComplete`.
    pub availability_time_complete: Option<bool>,
    /// `@timeShiftBufferDepth`.
    pub time_shift_buffer_depth: Option<Duration>,
}

/// A generic descriptor (5.8.2): `Role`, `Accessibility`,
/// `EssentialProperty`, `SupplementalProperty`, `ContentProtection`, ...
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Descriptor {
    /// `@schemeIdUri`.
    pub scheme_id_uri: String,
    /// `@value`.
    pub value: Option<String>,
    /// `@id`.
    pub id: Option<String>,
    /// `@cenc:default_KID` on `ContentProtection`.
    pub default_kid: Option<String>,
    /// The `UrlQueryInfo` child of an `urn:mpeg:dash:urlparam:2014` property.
    pub url_query: Option<UrlQueryInfo>,
}

/// `up:UrlQueryInfo` (Annex I.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct UrlQueryInfo {
    /// `@useMPDUrlQuery`.
    pub use_mpd_url_query: bool,
    /// `@queryString`, literal.
    pub query_string: Option<String>,
    /// `@queryTemplate`; present means placeholders this core does not
    /// understand.
    pub query_template: Option<String>,
}

/// A `UTCTiming` element (5.8.4.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UtcTiming {
    /// Classified `@schemeIdUri`.
    pub scheme: UtcScheme,
    /// `@schemeIdUri`.
    pub scheme_id_uri: String,
    /// `@value`: the time itself for `direct`, else whitespace-separated URLs.
    pub value: String,
    /// `@id`.
    pub id: Option<String>,
}

impl UtcTiming {
    /// The whitespace-separated entries of `@value`.
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.value.split_ascii_whitespace()
    }
}

/// Which not-acted-on constructs the MPD contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct MpdFeatures {
    /// `PatchLocation` (5.15).
    pub patch_location: bool,
    /// `EventStream` (5.10.2).
    pub event_stream: bool,
    /// `InbandEventStream` (5.10.3).
    pub inband_event_stream: bool,
    /// `Preselection` (5.3.11).
    pub preselection: bool,
    /// `ServiceDescription` (Annex K).
    pub service_description: bool,
    /// `ProducerReferenceTime` (5.12).
    pub producer_reference_time: bool,
    /// `ContentSteering`.
    pub content_steering: bool,
    /// `Resync` (5.3.13).
    pub resync: bool,
    /// An `availabilityTimeOffset`, or `availabilityTimeComplete="false"`,
    /// anywhere: a low-latency presentation.
    pub low_latency: bool,
    /// A Period or AdaptationSet removed because its xlink was
    /// `urn:mpeg:dash:resolve-to-zero:2013`.
    pub xlink_resolve_to_zero: bool,
}

/// The `RepresentationBase` attributes (5.3.7.2) that AdaptationSet and
/// Representation both carry and Representation inherits.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct CommonAttributes {
    /// `@profiles`.
    pub profiles: Option<String>,
    /// `@width`.
    pub width: Option<u32>,
    /// `@height`.
    pub height: Option<u32>,
    /// `@sar`.
    pub sar: Option<String>,
    /// `@frameRate` (`n` or `n/d`).
    pub frame_rate: Option<String>,
    /// `@audioSamplingRate` (`n` or `n n`).
    pub audio_sampling_rate: Option<String>,
    /// `@mimeType`.
    pub mime_type: Option<String>,
    /// `@codecs`.
    pub codecs: Option<String>,
    /// `@maxPlayoutRate`.
    pub max_playout_rate: Option<f64>,
}

/// `SegmentBase` / the Segment Base Information fields (Table 16).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BaseInfo {
    /// `@timescale`.
    pub timescale: Option<u64>,
    /// `@presentationTimeOffset`.
    pub presentation_time_offset: Option<u64>,
    /// `@eptDelta`.
    pub ept_delta: Option<i64>,
    /// `@pdDelta`.
    pub pd_delta: Option<i64>,
    /// `@presentationDuration`.
    pub presentation_duration: Option<u64>,
    /// `@timeShiftBufferDepth`.
    pub time_shift_buffer_depth: Option<Duration>,
    /// `@indexRange`.
    pub index_range: Option<ByteRange>,
    /// `@indexRangeExact`.
    pub index_range_exact: Option<bool>,
    /// `@availabilityTimeOffset`, seconds (may be `INF`).
    pub availability_time_offset: Option<f64>,
    /// `@availabilityTimeComplete`.
    pub availability_time_complete: Option<bool>,
    /// `Initialization`.
    pub initialization: Option<UrlType>,
    /// `RepresentationIndex`.
    pub representation_index: Option<UrlType>,
}

/// `SegmentBase` element.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SegmentBase {
    /// The Table 16 fields.
    pub info: BaseInfo,
}

/// One `S` of a `SegmentTimeline` (Table 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimelineEntry {
    /// `@t`.
    pub t: Option<u64>,
    /// `@n`.
    pub n: Option<u64>,
    /// `@d`.
    pub d: u64,
    /// `@r` (negative: until the next `S@t` or the Period end).
    pub r: i64,
    /// `@k`.
    pub k: Option<u64>,
}

impl TimelineEntry {
    /// An entry with only `d` set (`t`, `n`, `k` absent, `r` 0).
    #[must_use]
    pub fn new(d: u64) -> Self {
        Self {
            t: None,
            n: None,
            d,
            r: 0,
            k: None,
        }
    }
}

/// `SegmentTimeline` (5.3.9.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SegmentTimeline {
    /// The `S` entries in document order.
    pub entries: Vec<TimelineEntry>,
    /// Set when the timeline uses a feature outside 0.1 (`S@k` above 1, an
    /// Ed.6 `Pattern`); addressing then fails with that typed error.
    pub unsupported: Option<Unsupported>,
}

/// The Multiple Segment Base Information fields (Table 17) over [`BaseInfo`].
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct MultiInfo {
    /// The Table 16 fields.
    pub base: BaseInfo,
    /// `@duration`.
    pub duration: Option<u64>,
    /// `@startNumber`.
    pub start_number: Option<u64>,
    /// `@endNumber`.
    pub end_number: Option<u64>,
    /// `SegmentTimeline`.
    pub timeline: Option<SegmentTimeline>,
    /// `BitstreamSwitching`.
    pub bitstream_switching: Option<UrlType>,
}

/// `SegmentTemplate` (5.3.9.4).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SegmentTemplate {
    /// Table 16/17 fields.
    pub multi: MultiInfo,
    /// `@media` template.
    pub media: Option<String>,
    /// `@index` template.
    pub index: Option<String>,
    /// `@initialization` template.
    pub initialization: Option<String>,
    /// `@bitstreamSwitching` template.
    pub bitstream_switching: Option<String>,
}

/// One `SegmentURL` (Table 19).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SegmentUrl {
    /// `@media`.
    pub media: Option<String>,
    /// `@mediaRange`.
    pub media_range: Option<ByteRange>,
    /// `@index`.
    pub index: Option<String>,
    /// `@indexRange`.
    pub index_range: Option<ByteRange>,
}

/// `SegmentList` (5.3.9.3).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SegmentList {
    /// Table 16/17 fields.
    pub multi: MultiInfo,
    /// `@xlink:href`.
    pub xlink: Option<Xlink>,
    /// The `SegmentURL` entries.
    pub urls: Vec<SegmentUrl>,
}

/// The Segment information declared at one hierarchy level: at most one of
/// the three kinds (5.3.9.1).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SegmentLayer {
    /// `SegmentBase`.
    pub base: Option<SegmentBase>,
    /// `SegmentList`.
    pub list: Option<SegmentList>,
    /// `SegmentTemplate`.
    pub template: Option<SegmentTemplate>,
}

/// A `Representation` (5.3.5).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Representation {
    /// `@id`.
    pub id: String,
    /// `@bandwidth`, bits per second.
    pub bandwidth: u64,
    /// `@qualityRanking`.
    pub quality_ranking: Option<u32>,
    /// `@dependencyId` (whitespace-separated Representation ids).
    pub dependency_ids: Vec<String>,
    /// Common attributes (not yet merged with the AdaptationSet's).
    pub common: CommonAttributes,
    /// `BaseURL` alternates.
    pub base_urls: Vec<BaseUrl>,
    /// Segment information declared here.
    pub segment: SegmentLayer,
    /// `ContentProtection`.
    pub content_protections: Vec<Descriptor>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// `AudioChannelConfiguration`.
    pub audio_channel_configurations: Vec<Descriptor>,
}

/// An `AdaptationSet` (5.3.3).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct AdaptationSet {
    /// `@id`.
    pub id: Option<u64>,
    /// `@group`.
    pub group: Option<u64>,
    /// `@lang`.
    pub lang: Option<String>,
    /// `@contentType`.
    pub content_type: Option<String>,
    /// `@par`.
    pub par: Option<String>,
    /// `@segmentAlignment` (`true` or a non-zero count).
    pub segment_alignment: bool,
    /// `@startWithSAP`.
    pub start_with_sap: Option<u8>,
    /// `@bitstreamSwitching`.
    pub bitstream_switching: Option<bool>,
    /// Common attributes.
    pub common: CommonAttributes,
    /// `xlink` reference; the element is then unresolved.
    pub xlink: Option<Xlink>,
    /// `Role` descriptors.
    pub roles: Vec<Descriptor>,
    /// `Accessibility` descriptors.
    pub accessibilities: Vec<Descriptor>,
    /// `Label` texts.
    pub labels: Vec<String>,
    /// `BaseURL` alternates.
    pub base_urls: Vec<BaseUrl>,
    /// Segment information declared here.
    pub segment: SegmentLayer,
    /// `ContentProtection`.
    pub content_protections: Vec<Descriptor>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// `AudioChannelConfiguration`.
    pub audio_channel_configurations: Vec<Descriptor>,
    /// The Representations.
    pub representations: Vec<Representation>,
}

/// A `Period` (5.3.2).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Period {
    /// `@id`.
    pub id: Option<String>,
    /// `@start`.
    pub start: Option<Duration>,
    /// `@duration`.
    pub duration: Option<Duration>,
    /// `@bitstreamSwitching`.
    pub bitstream_switching: bool,
    /// `xlink` reference.
    pub xlink: Option<Xlink>,
    /// `BaseURL` alternates.
    pub base_urls: Vec<BaseUrl>,
    /// Segment information declared here.
    pub segment: SegmentLayer,
    /// `AssetIdentifier`.
    pub asset_identifier: Option<Descriptor>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// The AdaptationSets.
    pub adaptation_sets: Vec<AdaptationSet>,
}

/// An `MPD` (5.3.1).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Mpd {
    /// `@id`.
    pub id: Option<String>,
    /// `@profiles`, split on `,`.
    pub profiles: Vec<String>,
    /// `@type`.
    pub kind: PresentationType,
    /// `@availabilityStartTime`, Unix microseconds.
    pub availability_start_time_us: Option<i64>,
    /// `@availabilityEndTime`, Unix microseconds.
    pub availability_end_time_us: Option<i64>,
    /// `@publishTime`, Unix microseconds.
    pub publish_time_us: Option<i64>,
    /// `@mediaPresentationDuration`.
    pub media_presentation_duration: Option<Duration>,
    /// `@minimumUpdatePeriod`.
    pub minimum_update_period: Option<Duration>,
    /// `@minBufferTime`.
    pub min_buffer_time: Option<Duration>,
    /// `@timeShiftBufferDepth`.
    pub time_shift_buffer_depth: Option<Duration>,
    /// `@suggestedPresentationDelay`.
    pub suggested_presentation_delay: Option<Duration>,
    /// `@maxSegmentDuration`.
    pub max_segment_duration: Option<Duration>,
    /// `@maxSubsegmentDuration`.
    pub max_subsegment_duration: Option<Duration>,
    /// `BaseURL` alternates.
    pub base_urls: Vec<BaseUrl>,
    /// `Location` texts.
    pub locations: Vec<String>,
    /// `UTCTiming` elements in document order (alternatives, in preference
    /// order).
    pub utc_timings: Vec<UtcTiming>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// The Periods.
    pub periods: Vec<Period>,
    /// Constructs seen but not acted on.
    pub features: MpdFeatures,
}
