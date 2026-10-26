//! BaseURL alternates and URL resolution (ISO/IEC 23009-1 5.6).
//!
//! Resolution is RFC 3986 5.2 through [`transmux::resolve_base_url_chain`]
//! (which also rejects control characters), never string concatenation
//! (5.6.4 informative note). Alternates (5.6.5) are ordered by DVB `@priority`
//! (ETSI TS 103 285 10.8.2.1, default 1) then document order; `@weight` is
//! parsed but not used, because weighted random choice needs a random source
//! and this core is deterministic.

use url::Url;

use super::descriptors::{SCHEME_URLPARAM, SCHEME_URLPARAM_2016};
use super::model::{AdaptationSet, BaseUrl, Descriptor, Mpd, Period, Representation, UrlQueryInfo};
use crate::limits::MAX_BASE_CANDIDATES;

/// ETSI TS 103 285 10.8.2.1: the `@dvb:priority` default.
pub const DEFAULT_BASE_PRIORITY: u32 = 1;
const QUERY_SEPARATOR: char = '&';
const QUERY_PART_TEMPLATE: &str = "$querypart$";

/// One way of reaching the content: a chain of `BaseURL` texts, one per
/// level that declared any, outermost first.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BaseCandidate {
    /// The `BaseURL` texts, outermost first.
    pub chain: Vec<String>,
    /// `@serviceLocation` of the innermost entry.
    pub service_location: Option<String>,
    /// `@dvb:priority` of the innermost entry (default 1).
    pub priority: u32,
    /// Sum of `@availabilityTimeOffset` over the chain, seconds.
    pub availability_time_offset_s: f64,
}

impl BaseCandidate {
    fn root() -> Self {
        Self {
            chain: Vec::new(),
            service_location: None,
            priority: DEFAULT_BASE_PRIORITY,
            availability_time_offset_s: 0.0,
        }
    }

    /// Resolve `reference` through this candidate onto the MPD's own URL.
    /// `None` when any entry or the reference is unusable.
    #[must_use]
    pub fn resolve(&self, document_url: Option<&Url>, reference: &str) -> Option<String> {
        transmux::resolve_base_url_chain(document_url, &self.chain, reference)
    }
}

/// The candidate locations for the `levels` of base URLs (MPD, Period,
/// AdaptationSet, Representation; a level without `BaseURL` is skipped), best
/// first. Candidates sharing a `@serviceLocation` pair up across levels;
/// otherwise a level's alternates extend the best candidate above.
#[must_use]
pub fn base_candidates(levels: &[&[BaseUrl]]) -> Vec<BaseCandidate> {
    let mut candidates = vec![BaseCandidate::root()];
    for level in levels.iter().filter(|l| !l.is_empty()) {
        let mut ordered: Vec<&BaseUrl> = level.iter().collect();
        ordered.sort_by_key(|b| b.priority.unwrap_or(DEFAULT_BASE_PRIORITY));
        let mut next = Vec::with_capacity(ordered.len());
        for b in ordered {
            let parent = candidates
                .iter()
                .find(|c| c.service_location.is_some() && c.service_location == b.service_location)
                .unwrap_or(&candidates[0]);
            let mut chain = parent.chain.clone();
            chain.push(b.url.clone());
            next.push(BaseCandidate {
                chain,
                service_location: b.service_location.clone(),
                priority: b.priority.unwrap_or(DEFAULT_BASE_PRIORITY),
                availability_time_offset_s: parent.availability_time_offset_s
                    + b.availability_time_offset.unwrap_or(0.0),
            });
            if next.len() >= MAX_BASE_CANDIDATES {
                break;
            }
        }
        candidates = next;
    }
    candidates
}

/// The candidates for one Representation.
#[must_use]
pub fn representation_candidates(
    mpd: &Mpd,
    period: &Period,
    set: &AdaptationSet,
    rep: &Representation,
) -> Vec<BaseCandidate> {
    base_candidates(&[
        &mpd.base_urls,
        &period.base_urls,
        &set.base_urls,
        &rep.base_urls,
    ])
}

/// The `urn:mpeg:dash:urlparam:2014` query of a Representation was written in
/// a form this core does not interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("urlparam descriptor uses a queryTemplate this core does not understand")]
#[non_exhaustive]
pub struct UrlQueryNotUnderstood;

fn level_query(
    descriptors: &[&[Descriptor]],
    mpd_query: Option<&str>,
    out: &mut Vec<String>,
) -> Result<(), UrlQueryNotUnderstood> {
    for list in descriptors {
        for d in list.iter().filter(|d| d.scheme_id_uri == SCHEME_URLPARAM) {
            let Some(info) = d.url_query.as_ref() else {
                continue;
            };
            if let Some(t) = &info.query_template
                && t != QUERY_PART_TEMPLATE
            {
                return Err(UrlQueryNotUnderstood);
            }
            push_query(info, mpd_query, out);
        }
    }
    Ok(())
}

fn push_query(info: &UrlQueryInfo, mpd_query: Option<&str>, out: &mut Vec<String>) {
    let mut parts: Vec<&str> = Vec::new();
    if info.use_mpd_url_query
        && let Some(q) = mpd_query.filter(|q| !q.is_empty())
    {
        parts.push(q);
    }
    if let Some(s) = info.query_string.as_deref().filter(|s| !s.is_empty()) {
        parts.push(s);
    }
    if !parts.is_empty() {
        out.push(parts.join(&QUERY_SEPARATOR.to_string()));
    }
}

/// The query to append to every Media Segment URL of `rep` (Annex I.2.3):
/// Representation level, then AdaptationSet, Period, MPD, joined with `&`.
///
/// `mpd_query` is the query of the MPD's own URL. `None` when no descriptor
/// asks for one. An `urn:mpeg:dash:urlparam:2016` property is not understood
/// (callers treat an essential one as dropping the Representation, a
/// supplemental one as ignored).
///
/// # Errors
/// [`UrlQueryNotUnderstood`] for a `@queryTemplate` other than `$querypart$`.
pub fn segment_url_query(
    mpd: &Mpd,
    period: &Period,
    set: &AdaptationSet,
    rep: &Representation,
    mpd_query: Option<&str>,
) -> Result<Option<String>, UrlQueryNotUnderstood> {
    let mut parts = Vec::new();
    level_query(
        &[&rep.essential_properties, &rep.supplemental_properties],
        mpd_query,
        &mut parts,
    )?;
    level_query(
        &[&set.essential_properties, &set.supplemental_properties],
        mpd_query,
        &mut parts,
    )?;
    level_query(
        &[
            &period.essential_properties,
            &period.supplemental_properties,
        ],
        mpd_query,
        &mut parts,
    )?;
    level_query(
        &[&mpd.essential_properties, &mpd.supplemental_properties],
        mpd_query,
        &mut parts,
    )?;
    if parts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parts.join(&QUERY_SEPARATOR.to_string())))
    }
}

/// Append `query` to `url`: `?` when it has none, `&` when it has one
/// (Annex I.2.3 step 4).
#[must_use]
pub fn append_query(url: &str, query: &str) -> String {
    let (base, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let sep = if base.contains('?') {
        QUERY_SEPARATOR
    } else {
        '?'
    };
    let mut out = format!("{base}{sep}{query}");
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Whether `descriptor` is the 2016 urlparam scheme (never understood).
#[must_use]
pub fn is_unsupported_urlparam(descriptor: &Descriptor) -> bool {
    descriptor.scheme_id_uri == SCHEME_URLPARAM_2016
}
