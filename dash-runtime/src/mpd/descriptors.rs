//! Descriptor scheme URIs and the typed readers over [`Descriptor`] lists.
//!
//! Role values: ISO/IEC 23009-1 5.8.5.5 Table 34. Continuity / connectivity /
//! reset: 5.3.2.4, 5.4.2. URL parameters: Annex I.2. The last-segment-number
//! property is DASH-IF IOP v4.3.

use broadcast_common::impl_spec_display;

use super::model::{Descriptor, UrlQueryInfo};

/// 5.8.5.5: the role scheme.
pub const SCHEME_ROLE: &str = "urn:mpeg:dash:role:2011";
/// 5.3.2.4: Period-continuity signalling.
pub const SCHEME_PERIOD_CONTINUITY: &str = "urn:mpeg:dash:period-continuity:2015";
/// 5.3.2.4: Period-connectivity signalling.
pub const SCHEME_PERIOD_CONNECTIVITY: &str = "urn:mpeg:dash:period-connectivity:2015";
/// 5.4.2: MPD reset.
pub const SCHEME_RESET: &str = "urn:mpeg:dash:reset:2016";
/// Annex I.2: URL parameter signalling (media Segment requests).
pub const SCHEME_URLPARAM: &str = "urn:mpeg:dash:urlparam:2014";
/// Annex I.3: the 2016 superset scheme (not understood).
pub const SCHEME_URLPARAM_2016: &str = "urn:mpeg:dash:urlparam:2016";
/// DASH-IF IOP v4.3: the last Segment number of a live Period.
pub const SCHEME_LAST_SEGMENT_NUMBER: &str = "http://dashif.org/guidelines/last-segment-number";
/// 5.5.3 rule 4: an xlink that removes its element.
pub const XLINK_RESOLVE_TO_ZERO: &str = "urn:mpeg:dash:resolve-to-zero:2013";
/// 5.8.5.2: the MP4 protection scheme descriptor.
pub const SCHEME_MP4_PROTECTION: &str = "urn:mpeg:dash:mp4protection:2011";

/// A 5.8.5.5 role value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// `caption`.
    Caption,
    /// `subtitle`.
    Subtitle,
    /// `main`.
    Main,
    /// `alternate`.
    Alternate,
    /// `supplementary`.
    Supplementary,
    /// `commentary`.
    Commentary,
    /// `dub`.
    Dub,
    /// `description`.
    Description,
    /// `sign`.
    Sign,
    /// `metadata`.
    Metadata,
    /// `enhanced-audio-intelligibility`.
    EnhancedAudioIntelligibility,
    /// `emergency`.
    Emergency,
    /// `forced-subtitle`.
    ForcedSubtitle,
    /// `easyreader`.
    EasyReader,
    /// `karaoke`.
    Karaoke,
    /// Any other token.
    Other,
}

impl Role {
    /// Classify a `Role@value` of the DASH role scheme.
    #[must_use]
    pub fn from_value(value: &str) -> Self {
        match value {
            "caption" => Self::Caption,
            "subtitle" => Self::Subtitle,
            "main" => Self::Main,
            "alternate" => Self::Alternate,
            "supplementary" => Self::Supplementary,
            "commentary" => Self::Commentary,
            "dub" => Self::Dub,
            "description" => Self::Description,
            "sign" => Self::Sign,
            "metadata" => Self::Metadata,
            "enhanced-audio-intelligibility" => Self::EnhancedAudioIntelligibility,
            "emergency" => Self::Emergency,
            "forced-subtitle" => Self::ForcedSubtitle,
            "easyreader" => Self::EasyReader,
            "karaoke" => Self::Karaoke,
            _ => Self::Other,
        }
    }

    /// The Table 34 token, or `"other"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Caption => "caption",
            Self::Subtitle => "subtitle",
            Self::Main => "main",
            Self::Alternate => "alternate",
            Self::Supplementary => "supplementary",
            Self::Commentary => "commentary",
            Self::Dub => "dub",
            Self::Description => "description",
            Self::Sign => "sign",
            Self::Metadata => "metadata",
            Self::EnhancedAudioIntelligibility => "enhanced-audio-intelligibility",
            Self::Emergency => "emergency",
            Self::ForcedSubtitle => "forced-subtitle",
            Self::EasyReader => "easyreader",
            Self::Karaoke => "karaoke",
            Self::Other => "other",
        }
    }
}
impl_spec_display!(Role);

/// The roles declared by `Role` descriptors of the DASH role scheme.
pub fn roles(descriptors: &[Descriptor]) -> impl Iterator<Item = Role> + '_ {
    descriptors
        .iter()
        .filter(|d| d.scheme_id_uri == SCHEME_ROLE)
        .filter_map(|d| d.value.as_deref())
        .map(Role::from_value)
}

/// The first descriptor with `scheme`.
#[must_use]
pub fn find<'a>(descriptors: &'a [Descriptor], scheme: &str) -> Option<&'a Descriptor> {
    descriptors.iter().find(|d| d.scheme_id_uri == scheme)
}

/// The `urn:mpeg:dash:urlparam:2014` information among `descriptors`.
#[must_use]
pub fn url_query_info(descriptors: &[Descriptor]) -> Option<&UrlQueryInfo> {
    descriptors
        .iter()
        .filter(|d| d.scheme_id_uri == SCHEME_URLPARAM)
        .find_map(|d| d.url_query.as_ref())
}

/// Schemes an `EssentialProperty` may name without making the parent element
/// unprocessable (5.8.4.8: a client that does not understand an essential
/// scheme drops the parent).
pub const UNDERSTOOD_ESSENTIAL: &[&str] = &[
    SCHEME_PERIOD_CONTINUITY,
    SCHEME_PERIOD_CONNECTIVITY,
    SCHEME_RESET,
    SCHEME_URLPARAM,
    SCHEME_LAST_SEGMENT_NUMBER,
];

/// Whether this core understands an `EssentialProperty` scheme.
#[must_use]
pub fn essential_understood(descriptor: &Descriptor) -> bool {
    UNDERSTOOD_ESSENTIAL.contains(&descriptor.scheme_id_uri.as_str())
}
