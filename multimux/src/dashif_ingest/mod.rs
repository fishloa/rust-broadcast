//! DASH-IF Live Media Ingest (Interface-1) — sans-IO core (increment P1).
//!
//! Request classification ([`classify`]), MPD manifest grouping extraction
//! ([`ManifestGrouping::from_mpd`]), the §5.3 status mapping ([`IngestError::status`]),
//! and the per-track header/segment state machine ([`IngestState`]).
//!
//! No sockets, no tokio, no axum: inputs are plain Rust values. Cited against
//! `private/specs/dashif_live_media_ingest_v1.3.html` §5.2, §5.3, §6.2
//! (items 1, 11, 18, 16 a-h, 17 a-e, 19) and ISO/IEC 23009-1 §5.3.9.4.4
//! (template grammar) as implemented by [`transmux::dash_parse::SegmentTemplate::resolve`].

use std::collections::{HashMap, HashSet};

use axum::http::StatusCode;
use transmux::dash_parse::{Mpd, SegmentTemplate};

/// HTTP method of an ingest request (§5.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IngestMethod {
    Post,
    Put,
}

impl IngestMethod {
    pub fn name(&self) -> &'static str {
        match self {
            IngestMethod::Post => "POST",
            IngestMethod::Put => "PUT",
        }
    }
}

broadcast_common::impl_spec_display!(IngestMethod);

/// Manifest flavour accepted on the ingest path (§6.2.2, §6.2.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestFormat {
    Dash,
    Hls,
}

impl ManifestFormat {
    pub fn name(&self) -> &'static str {
        match self {
            ManifestFormat::Dash => "dash",
            ManifestFormat::Hls => "hls",
        }
    }
}

broadcast_common::impl_spec_display!(ManifestFormat);

/// Addressing mode derived from the MPD's `SegmentTemplate@media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Addressing {
    Number,
    Time,
}

impl Addressing {
    pub fn name(&self) -> &'static str {
        match self {
            Addressing::Number => "number",
            Addressing::Time => "time",
        }
    }
}

broadcast_common::impl_spec_display!(Addressing);

/// Classified shape of one ingest request (DESIGN §2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjectKind {
    Probe,
    Manifest(ManifestFormat),
    Header {
        representation_id: String,
    },
    Segment {
        representation_id: String,
        number: Option<u64>,
        time: Option<u64>,
    },
}

impl ObjectKind {
    pub fn name(&self) -> &'static str {
        match self {
            ObjectKind::Probe => "probe",
            ObjectKind::Manifest(_) => "manifest",
            ObjectKind::Header { .. } => "header",
            ObjectKind::Segment { .. } => "segment",
        }
    }
}

broadcast_common::impl_spec_display!(ObjectKind);

/// Disposition of an accepted segment (§6.2.10/11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentDisposition {
    Accepted,
    Duplicate,
    Regression,
}

impl SegmentDisposition {
    pub fn name(&self) -> &'static str {
        match self {
            SegmentDisposition::Accepted => "accepted",
            SegmentDisposition::Duplicate => "duplicate",
            SegmentDisposition::Regression => "regression",
        }
    }
}

broadcast_common::impl_spec_display!(SegmentDisposition);

/// Metadata extracted from one ingest HTTP request.
#[derive(Debug, Clone, Copy)]
pub struct RequestMeta<'a> {
    pub method: IngestMethod,
    pub path: &'a str,
    pub content_length: Option<u64>,
    pub chunked: bool,
    pub body_empty: bool,
}

/// Grouping extracted from a DASH MPD per §6.2.16.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestGrouping {
    pub init_template: String,
    pub media_template: String,
    pub addressing: Addressing,
    pub switching_sets: Vec<Vec<String>>,
    /// §6.2.16e SHOULD: raw `MPD@availabilityStartTime`.
    pub availability_start_time: Option<String>,
    /// §6.2.16e SHOULD: `Period@start`.
    pub period_start: Option<std::time::Duration>,
}

/// Errors raised by the ingest core.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum IngestError {
    #[error("auth required (§5.3.4)")]
    Auth,

    #[error("unknown publishing point (§5.3.5a)")]
    UnknownPublishingPoint,

    #[error("unsupported media (§5.3.5b)")]
    UnsupportedMedia,

    #[error("segment without a usable header (§5.3.5c)")]
    SegmentWithoutHeader,

    #[error("receiver-side fault (§5.3.5d)")]
    Internal,

    #[error("malformed request (§5.3.5e)")]
    BadRequest,

    #[error("no manifest received yet")]
    NoManifest,

    #[error("MPD has multiple Period elements (§6.2.16h)")]
    MpdMultiplePeriods,

    #[error("MPD SegmentTemplate {attr} template mismatch (§6.2.16a/b/c)")]
    MpdTemplateMismatch { attr: &'static str },

    #[error("MPD contains a BaseURL element (§6.2.16d)")]
    MpdBaseUrlPresent,

    #[error("MPD AdaptationSet/Representation lacks a SegmentTemplate (§6.2.16 intro)")]
    MpdMissingSegmentTemplate,

    #[error("header changed mid-session (§6.2.19)")]
    HeaderChanged,
}

impl IngestError {
    pub fn status(&self) -> StatusCode {
        match self {
            IngestError::Auth => StatusCode::FORBIDDEN,
            IngestError::UnknownPublishingPoint => StatusCode::NOT_FOUND,
            IngestError::SegmentWithoutHeader => StatusCode::PRECONDITION_FAILED,
            IngestError::UnsupportedMedia => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            IngestError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl ManifestGrouping {
    pub fn from_mpd(bytes: &[u8]) -> Result<Self, IngestError> {
        let xml = std::str::from_utf8(bytes).map_err(|_| IngestError::BadRequest)?;
        let mpd = Mpd::parse(xml).map_err(|_| IngestError::BadRequest)?;

        if mpd.periods.len() != 1 {
            return Err(IngestError::MpdMultiplePeriods);
        }
        let period = &mpd.periods[0];

        // BaseURL absent anywhere (§6.2.16d).
        if mpd.base_url.is_some()
            || period.base_url.is_some()
            || mpd
                .periods
                .iter()
                .flat_map(|p| p.adaptation_sets.iter())
                .any(|a| {
                    a.base_url.is_some() || a.representations.iter().any(|r| r.base_url.is_some())
                })
        {
            return Err(IngestError::MpdBaseUrlPresent);
        }

        // Collect effective templates per representation.
        // An AdaptationSet may carry the template on itself (inherited by reps).
        let mut templates: Vec<(String /*rep id*/, SegmentTemplate)> = Vec::new();
        let mut switching_sets: Vec<Vec<String>> = Vec::new();
        for aset in &period.adaptation_sets {
            let mut set_ids: Vec<String> = Vec::new();
            for rep in &aset.representations {
                let tmpl = rep
                    .segment_template
                    .clone()
                    .or_else(|| aset.segment_template.clone());
                let tmpl = match tmpl {
                    Some(t) => t,
                    None => return Err(IngestError::MpdMissingSegmentTemplate),
                };
                templates.push((rep.id.clone(), tmpl));
                set_ids.push(rep.id.clone());
            }
            if !set_ids.is_empty() {
                switching_sets.push(set_ids);
            }
            // AdaptationSet with no reps but its own template: nothing to group.
        }

        if templates.is_empty() {
            return Err(IngestError::MpdMissingSegmentTemplate);
        }

        // Cross-template identity (§6.2.16b/c).
        let first_init = templates[0].1.initialization.clone();
        let first_media = templates[0].1.media.clone();
        for (_, t) in &templates {
            if t.initialization != first_init {
                return Err(IngestError::MpdTemplateMismatch {
                    attr: "initialization",
                });
            }
            if t.media != first_media {
                return Err(IngestError::MpdTemplateMismatch { attr: "media" });
            }
        }

        let init_template = first_init.unwrap_or_default();
        let media_template = first_media.unwrap_or_default();

        // §6.2.16a: init contains $RepresentationID$ exactly once.
        if init_template.matches("$RepresentationID$").count() != 1 {
            return Err(IngestError::MpdTemplateMismatch {
                attr: "initialization",
            });
        }
        // media contains $RepresentationID$ exactly once.
        if media_template.matches("$RepresentationID$").count() != 1 {
            return Err(IngestError::MpdTemplateMismatch { attr: "media" });
        }
        // media contains exactly one of $Number$/$Time$.
        let number_count = count_ident(&media_template, "Number");
        let time_count = count_ident(&media_template, "Time");
        let has_number = number_count > 0;
        let has_time = time_count > 0;
        if !(has_number ^ has_time) {
            return Err(IngestError::MpdTemplateMismatch { attr: "media" });
        }
        if has_number && number_count != 1 {
            return Err(IngestError::MpdTemplateMismatch { attr: "media" });
        }
        if has_time && time_count != 1 {
            return Err(IngestError::MpdTemplateMismatch { attr: "media" });
        }

        let addressing = if has_number {
            Addressing::Number
        } else {
            Addressing::Time
        };

        Ok(ManifestGrouping {
            init_template,
            media_template,
            addressing,
            switching_sets,
            availability_start_time: mpd.availability_start_time.clone(),
            period_start: period.start,
        })
    }
}

fn count_ident(template: &str, name: &str) -> usize {
    // Count `$Name$` and `$Name%...d$` occurrences.
    let mut count = 0;
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        let tail = &rest[start + 1..];
        if let Some(end) = tail.find('$') {
            let ident = &tail[..end];
            let id_name = ident.split('%').next().unwrap_or(ident);
            if id_name == name {
                count += 1;
            }
            rest = &tail[end + 1..];
        } else {
            break;
        }
    }
    count
}

pub fn classify(
    req: &RequestMeta<'_>,
    grouping: Option<&ManifestGrouping>,
) -> Result<ObjectKind, IngestError> {
    if req.body_empty {
        return Ok(ObjectKind::Probe);
    }
    let lower = req.path.to_ascii_lowercase();
    if lower.ends_with(".mpd") {
        return Ok(ObjectKind::Manifest(ManifestFormat::Dash));
    }
    if lower.ends_with(".m3u8") {
        return Ok(ObjectKind::Manifest(ManifestFormat::Hls));
    }

    let g = grouping.ok_or(IngestError::NoManifest)?;

    let known: Vec<&str> = g
        .switching_sets
        .iter()
        .flatten()
        .map(|s| s.as_str())
        .collect();

    if let Some(m) = SegmentTemplate::match_url(&g.init_template, req.path, &known) {
        return Ok(ObjectKind::Header {
            representation_id: m.representation_id,
        });
    }
    if let Some(m) = SegmentTemplate::match_url(&g.media_template, req.path, &known) {
        return Ok(ObjectKind::Segment {
            representation_id: m.representation_id,
            number: m.number,
            time: m.time,
        });
    }

    Err(IngestError::BadRequest)
}

/// Per-track header/segment state machine.
#[derive(Debug, Default)]
pub struct IngestState {
    headers: HashSet<String>,
    last_number: HashMap<String, u64>,
    last_time: HashMap<String, u64>,
    last_segments: HashSet<String>,
}

impl IngestState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_header(&mut self, id: &str) {
        self.headers.insert(id.to_string());
        // Restart the track (§6.2.19): clear monotonic state.
        self.last_number.remove(id);
        self.last_time.remove(id);
        self.last_segments.remove(id);
    }

    pub fn has_header(&self, id: &str) -> bool {
        self.headers.contains(id)
    }

    pub fn on_segment(
        &mut self,
        id: &str,
        number: Option<u64>,
        time: Option<u64>,
    ) -> Result<SegmentDisposition, IngestError> {
        if !self.headers.contains(id) {
            return Err(IngestError::SegmentWithoutHeader);
        }

        // Determine disposition.
        let mut regression = false;
        let mut duplicate = true;
        let mut has_comparable = false;
        let mut any = false;

        if let Some(n) = number {
            any = true;
            if let Some(ln) = self.last_number.get(id) {
                has_comparable = true;
                if n < *ln {
                    regression = true;
                }
                if n != *ln {
                    duplicate = false;
                }
            } else {
                duplicate = false;
            }
        }
        if let Some(t) = time {
            any = true;
            if let Some(lt) = self.last_time.get(id) {
                has_comparable = true;
                if t < *lt {
                    regression = true;
                }
                if t != *lt {
                    duplicate = false;
                }
            } else {
                duplicate = false;
            }
        }
        if !any {
            duplicate = false;
        }

        let disposition = if regression {
            SegmentDisposition::Regression
        } else if has_comparable && duplicate {
            SegmentDisposition::Duplicate
        } else {
            SegmentDisposition::Accepted
        };

        // Update last seen.
        if let Some(n) = number {
            self.last_number.insert(id.to_string(), n);
        }
        if let Some(t) = time {
            self.last_time.insert(id.to_string(), t);
        }

        Ok(disposition)
    }

    pub fn mark_last(&mut self, id: &str) {
        self.last_segments.insert(id.to_string());
    }

    pub fn is_last(&self, id: &str) -> bool {
        self.last_segments.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_exhaustive() {
        // Exhaustive match (no wildcard): adding a variant breaks compile.
        fn expected(e: &IngestError) -> u16 {
            match e {
                IngestError::Auth => 403,
                IngestError::UnknownPublishingPoint => 404,
                IngestError::SegmentWithoutHeader => 412,
                IngestError::UnsupportedMedia => 415,
                IngestError::Internal => 500,
                IngestError::BadRequest => 400,
                IngestError::NoManifest => 400,
                IngestError::MpdMultiplePeriods => 400,
                IngestError::MpdTemplateMismatch { .. } => 400,
                IngestError::MpdBaseUrlPresent => 400,
                IngestError::MpdMissingSegmentTemplate => 400,
                IngestError::HeaderChanged => 400,
            }
        }
        let variants = [
            IngestError::Auth,
            IngestError::UnknownPublishingPoint,
            IngestError::SegmentWithoutHeader,
            IngestError::UnsupportedMedia,
            IngestError::Internal,
            IngestError::BadRequest,
            IngestError::NoManifest,
            IngestError::MpdMultiplePeriods,
            IngestError::MpdTemplateMismatch { attr: "media" },
            IngestError::MpdBaseUrlPresent,
            IngestError::MpdMissingSegmentTemplate,
            IngestError::HeaderChanged,
        ];
        for v in &variants {
            assert_eq!(v.status().as_u16(), expected(v));
        }
    }
}

pub mod session;
pub use session::{DashifIngestSession, IngestObject, SessionError};
