//! The consume-only MPD model (ISO/IEC 23009-1 clause 5.3, Annex B schema).
//!
//! Every field is typed; nothing keeps source text for a later passthrough.
//! Times are Unix microseconds (`*_us`, `i64`) or [`Duration`]. Elements the
//! core does not act on (`EventStream`, `Preselection`, ...) are still modelled
//! so [`Mpd::to_xml`] writes back what [`Mpd::parse`] read.

use std::time::Duration;

use broadcast_common::impl_spec_display;

use crate::error::Unsupported;
use crate::utc_timing::UtcScheme;

/// `MPD@type` (Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PresentationType {
    /// `static` (the default): the whole presentation is available.
    #[default]
    Static,
    /// `dynamic`: segments become available over wall-clock time.
    Dynamic,
    /// `list` (ISO/IEC 23009-1 ed.6): the presentation is an ordered list of
    /// Periods, each naming an imported MPD.
    List,
}

impl PresentationType {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
            Self::List => "list",
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Xlink {
    /// `@xlink:href`.
    pub href: String,
    /// `@xlink:actuate`.
    pub actuate: Actuate,
}

/// A single byte range `first-last` (RFC 7233 2.1), inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ByteRange {
    /// First byte.
    pub first: u64,
    /// Last byte; `None` for the open-ended form `first-`.
    pub last: Option<u64>,
}

impl core::fmt::Display for ByteRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.last {
            Some(last) => write!(f, "{}-{}", self.first, last),
            None => write!(f, "{}-", self.first),
        }
    }
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
    /// `cenc:pssh` child text on `ContentProtection`.
    pub pssh: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

/// Which constructs the MPD contained that the core reports rather than
/// models element by element.
///
/// Every flag is derived from content the writer emits, so it survives a parse
/// -> [`Mpd::to_xml`] -> parse round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct MpdFeatures {
    /// `InbandEventStream` (5.10.3).
    pub inband_event_stream: bool,
    /// `ServiceDescription` (Annex K).
    pub service_description: bool,
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
    /// `@codingDependency`.
    pub coding_dependency: Option<bool>,
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

/// `Event@status` (ISO/IEC 23009-1 Annex B `EventStatusType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EventStatus {
    /// `none`.
    None,
    /// `update`.
    Update,
    /// `repeat` (the schema default).
    #[default]
    Repeat,
}

impl EventStatus {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Update => "update",
            Self::Repeat => "repeat",
        }
    }
}
impl_spec_display!(EventStatus);

/// One `Event` of an `EventStream` (ISO/IEC 23009-1 clause 5.10.2, Table 28;
/// Annex B `EventType`).
///
/// An Event whose content is an XML payload (`SelectionInfo`,
/// `InsertPresentation`, `ReplacePresentation` or a foreign-namespace
/// element, e.g. an SCTE 35 `Signal`) is parsed with [`StreamEvent::unsupported`]
/// set and its payload dropped; [`Mpd::to_xml`] then refuses to write the MPD
/// with [`Unsupported::EventXmlPayload`] instead of silently losing it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct StreamEvent {
    /// `@presentationTime`, in the EventStream timescale (default 0).
    pub presentation_time: u64,
    /// `@duration`, in the EventStream timescale.
    pub duration: Option<u64>,
    /// `@id`.
    pub id: Option<u64>,
    /// `@status`; `None` when absent (the schema default is `repeat`).
    pub status: Option<EventStatus>,
    /// `@messageData` (deprecated since ed.4, prohibited by the ed.6 schema).
    pub message_data: Option<String>,
    /// The element's character data, trimmed: a text or base64 message body.
    pub text: String,
    /// `SupplementalProperty` children.
    pub supplemental_properties: Vec<Descriptor>,
    /// `EssentialProperty` children.
    pub essential_properties: Vec<Descriptor>,
    /// `ServiceDescription` children (ed.6 service-description events).
    pub service_descriptions: Vec<ServiceDescription>,
    /// Set when the Event carries an XML payload this model cannot hold.
    pub unsupported: Option<Unsupported>,
}

/// An `EventStream` (ISO/IEC 23009-1 clause 5.10.2, Table 27; Annex B
/// `EventStreamType`) of a Period.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EventStream {
    /// `xlink` reference; the stream is then unresolved.
    pub xlink: Option<Xlink>,
    /// `@schemeIdUri`.
    pub scheme_id_uri: String,
    /// `@value`.
    pub value: Option<String>,
    /// `@timescale`, ticks per second.
    pub timescale: Option<u32>,
    /// `@presentationTimeOffset`, in the timescale.
    pub presentation_time_offset: Option<u64>,
    /// The `Event` children in document order.
    pub events: Vec<StreamEvent>,
    /// `BaseURL` alternates.
    pub base_urls: Vec<BaseUrl>,
    /// `RequestParam` children (`up:ExtendedUrlInfoType`).
    pub request_params: Vec<RequestParam>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
}

/// `RequestParam` (`up:ExtendedUrlInfoType`, ISO/IEC 23009-1 Annex I).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RequestParam {
    /// The `UrlQueryInfoType` attributes.
    pub query: UrlQueryInfo,
    /// `xlink` reference.
    pub xlink: Option<Xlink>,
    /// `@includeInRequests`; `None` when absent (the schema default is `segment`).
    pub include_in_requests: Option<String>,
    /// `@headerParamSource`; `None` when absent (the schema default is `segment`).
    pub header_param_source: Option<String>,
    /// `@sameOriginOnly`; `None` when absent (the schema default is false).
    pub same_origin_only: Option<bool>,
    /// `@header`.
    pub header: Option<String>,
}

/// `ProducerReferenceTime@type` (ISO/IEC 23009-1 clause 5.12, Table 33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ProducerReferenceKind {
    /// `encoder` (the default): the time the encoder produced the sample.
    #[default]
    Encoder,
    /// `captured`: the capture time of the content.
    Captured,
    /// `application`: defined by `@applicationScheme`.
    Application,
}

impl ProducerReferenceKind {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Encoder => "encoder",
            Self::Captured => "captured",
            Self::Application => "application",
        }
    }
}
impl_spec_display!(ProducerReferenceKind);

/// A `ProducerReferenceTime` (ISO/IEC 23009-1 clause 5.12, Table 33) of an
/// AdaptationSet or Representation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ProducerReferenceTime {
    /// `@id`.
    pub id: u32,
    /// `@inband` (default false).
    pub inband: bool,
    /// `@type`.
    pub kind: ProducerReferenceKind,
    /// `@applicationScheme`.
    pub application_scheme: Option<String>,
    /// `@wallClockTime`, the lexical form (NTP-style date-time).
    pub wall_clock_time: String,
    /// `@presentationTime`, in the Representation timescale.
    pub presentation_time: u64,
    /// The `UTCTiming` child naming the wall-clock source.
    pub utc_timing: Option<UtcTiming>,
}

/// A `PatchLocation` (ISO/IEC 23009-1 clause 5.15, Table 41): where an MPD
/// patch is fetched.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct PatchLocation {
    /// The element text (the patch URL).
    pub url: String,
    /// `@serviceLocation`.
    pub service_location: Option<String>,
    /// `@ttl`, seconds the patch location stays valid.
    pub ttl: Option<f64>,
}

/// A `Label` or `GroupLabel` (`LabelType`): its text with `@id` (default 0)
/// and `@lang`.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Label {
    /// `@id`; `0` is the schema default and is omitted on write, so an explicit
    /// `id="0"` parses equal but is not written back.
    pub id: u32,
    /// `@lang`.
    pub lang: Option<String>,
    /// Element text.
    pub text: String,
}

/// A `Resync` (ISO/IEC 23009-1 clause 5.3.13, Table 15) resynchronisation
/// point of an AdaptationSet or Representation.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Resync {
    /// `@type`, the SAP type 0 to 6 (default 0).
    pub kind: u8,
    /// `@dT`, the maximum time offset.
    pub dt: Option<u32>,
    /// `@dImax`, the maximum Resync interval.
    pub d_imax: Option<f64>,
    /// `@dImin`, the minimum Resync interval.
    pub d_imin: Option<f64>,
    /// `@marker`.
    pub marker: bool,
}

/// `Preselection@order` (ISO/IEC 23009-1 clause 5.3.11, Table 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PreselectionOrder {
    /// `undefined` (the default).
    #[default]
    Undefined,
    /// `time-ordered`.
    TimeOrdered,
    /// `fully-ordered`.
    FullyOrdered,
}

impl PreselectionOrder {
    /// Stable label (the attribute token).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::TimeOrdered => "time-ordered",
            Self::FullyOrdered => "fully-ordered",
        }
    }
}
impl_spec_display!(PreselectionOrder);

/// A `Preselection` (ISO/IEC 23009-1 clause 5.3.11, Table 14) of a Period.
///
/// Carries the `RepresentationBase` attributes and children the model has a
/// type for; `Switching`, `RandomAccess`, `ContentPopularityRate` and
/// `SegmentSequenceProperties` set `unsupported` instead of being dropped.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Preselection {
    /// `@id`; `None` when absent (the schema default is `1`).
    pub id: Option<String>,
    /// `@preselectionComponents`, whitespace-separated AdaptationSet ids.
    pub preselection_components: Vec<String>,
    /// `@lang`.
    pub lang: Option<String>,
    /// `@order`; `None` when absent (the schema default is `undefined`).
    pub order: Option<PreselectionOrder>,
    /// The `RepresentationBase` attributes.
    pub common: CommonAttributes,
    /// `AudioChannelConfiguration`.
    pub audio_channel_configurations: Vec<Descriptor>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// `Accessibility` descriptors.
    pub accessibilities: Vec<Descriptor>,
    /// `Role` descriptors.
    pub roles: Vec<Descriptor>,
    /// `Rating` descriptors.
    pub ratings: Vec<Descriptor>,
    /// `Viewpoint` descriptors.
    pub viewpoints: Vec<Descriptor>,
    /// `@tag`.
    pub tag: Option<String>,
    /// `@selectionPriority`; `None` when absent (the schema default is `1`).
    pub selection_priority: Option<u32>,
    /// `@startWithSAP`.
    pub start_with_sap: Option<u8>,
    /// `@scanType`.
    pub scan_type: Option<String>,
    /// `@segmentProfiles`.
    pub segment_profiles: Option<String>,
    /// `@containerProfiles`.
    pub container_profiles: Option<String>,
    /// `@maximumSAPPeriod`, seconds.
    pub maximum_sap_period: Option<f64>,
    /// `FramePacking`.
    pub frame_packings: Vec<Descriptor>,
    /// `ContentProtection`.
    pub content_protections: Vec<Descriptor>,
    /// `OutputProtection`.
    pub output_protection: Option<Descriptor>,
    /// `InbandEventStream`.
    pub inband_event_streams: Vec<Descriptor>,
    /// `GroupLabel` elements.
    pub group_labels: Vec<Label>,
    /// `Label` elements.
    pub labels: Vec<Label>,
    /// `ProducerReferenceTime`.
    pub producer_reference_times: Vec<ProducerReferenceTime>,
    /// `Resync`.
    pub resyncs: Vec<Resync>,
    /// Set when a child the model cannot carry was present.
    pub unsupported: Option<Unsupported>,
}

/// A `ContentSteering` (ISO/IEC 23009-1 clause 5.3.1, Table 3; Annex B
/// `ContentSteeringType`) of the MPD.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ContentSteering {
    /// The element text (the steering server URL).
    pub url: String,
    /// `@defaultServiceLocation`.
    pub default_service_location: Option<String>,
    /// `@queryBeforeStart` (default false).
    pub query_before_start: bool,
    /// `@clientRequirement`; `None` when absent (the schema default is true).
    pub client_requirement: Option<bool>,
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
    /// `InbandEventStream`.
    pub inband_event_streams: Vec<Descriptor>,
    /// `ProducerReferenceTime`.
    pub producer_reference_times: Vec<ProducerReferenceTime>,
    /// `Resync`.
    pub resyncs: Vec<Resync>,
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
    /// `Label` elements.
    pub labels: Vec<Label>,
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
    /// `InbandEventStream`.
    pub inband_event_streams: Vec<Descriptor>,
    /// `ProducerReferenceTime`.
    pub producer_reference_times: Vec<ProducerReferenceTime>,
    /// `Resync`.
    pub resyncs: Vec<Resync>,
    /// The Representations.
    pub representations: Vec<Representation>,
}

/// `ImportedMPD` (ISO/IEC 23009-1 ed.6 `ImportedMpdType`): an `xs:anyURI`
/// element text plus `@earliestResolutionTimeOffset`.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ImportedMpd {
    /// The imported MPD's location (the element text).
    pub url: String,
    /// `@earliestResolutionTimeOffset`, seconds; the schema default is 60.0.
    pub earliest_resolution_time_offset: Option<f64>,
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
    /// `EventStream`.
    pub event_streams: Vec<EventStream>,
    /// `ImportedMPD` (ISO/IEC 23009-1 ed.6 Table `ImportedMpdType`): the MPD a
    /// `@type="list"` Period presents.
    pub imported_mpd: Option<ImportedMpd>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// The AdaptationSets.
    pub adaptation_sets: Vec<AdaptationSet>,
    /// `Preselection`.
    pub preselections: Vec<Preselection>,
    /// `ServiceDescription`.
    pub service_descriptions: Vec<ServiceDescription>,
}

/// `Latency` child of `ServiceDescription` (Annex K).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Latency {
    /// `@target`, milliseconds.
    pub target: Option<u64>,
    /// `@min`, milliseconds.
    pub min: Option<u64>,
    /// `@max`, milliseconds.
    pub max: Option<u64>,
    /// `@referenceId`.
    pub reference_id: Option<u32>,
}

/// `PlaybackRate` child of `ServiceDescription` (Annex K).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct PlaybackRate {
    /// `@min`, lexical form.
    pub min: Option<String>,
    /// `@max`, lexical form.
    pub max: Option<String>,
}

/// `ServiceDescription` (Annex K).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ServiceDescription {
    /// `@id`.
    pub id: Option<u64>,
    /// `Latency`.
    pub latency: Option<Latency>,
    /// `PlaybackRate`.
    pub playback_rate: Option<PlaybackRate>,
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
    /// `PatchLocation`.
    pub patch_locations: Vec<PatchLocation>,
    /// `ServiceDescription`.
    pub service_descriptions: Vec<ServiceDescription>,
    /// `UTCTiming` elements in document order (alternatives, in preference
    /// order).
    pub utc_timings: Vec<UtcTiming>,
    /// `EssentialProperty`.
    pub essential_properties: Vec<Descriptor>,
    /// `SupplementalProperty`.
    pub supplemental_properties: Vec<Descriptor>,
    /// `ContentSteering`.
    pub content_steering: Option<ContentSteering>,
    /// The Periods.
    pub periods: Vec<Period>,
    /// Constructs seen but not acted on.
    pub features: MpdFeatures,
}

impl Mpd {
    /// Whether the MPD declares an `EventStream` in a Period or an
    /// `InbandEventStream` anywhere.
    #[must_use]
    pub fn has_event_streams(&self) -> bool {
        self.features.inband_event_stream
            || self.periods.iter().any(|p| !p.event_streams.is_empty())
    }

    /// Whether any Period declares a `Preselection`.
    #[must_use]
    pub fn has_preselections(&self) -> bool {
        self.periods.iter().any(|p| !p.preselections.is_empty())
    }

    /// Whether the MPD declares a `PatchLocation`.
    #[must_use]
    pub fn has_patch_location(&self) -> bool {
        !self.patch_locations.is_empty()
    }
}
