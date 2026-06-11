//! EPG convenience layer.
//!
//! This module provides a high-level store for EIT (Event Information Table) data,
//! making it easy to query the "now and next" events for a service and export
//! a full EPG schedule.
//!
//! It wraps [`crate::collect::EitCollector`] to handle the multi-section
//! reassembly of EIT tables and maintains a deduplicated, time-ordered event list
//! for each service keyed by `(original_network_id, transport_stream_id, service_id)`.
//!
//! # Memory bounds
//!
//! The store is bounded by default to prevent memory-exhaustion from hostile
//! or pathological EIT input — an attacker who controls
//! `original_network_id`/`transport_stream_id`/`service_id`/`event_id` could
//! otherwise grow the cache without bound. Two caps apply:
//!
//! * **`max_services`** (default [`DEFAULT_MAX_SERVICES`]) — caps the number of
//!   distinct [`ServiceKey`] entries. When the cap is reached, incoming events
//!   for new services are skipped until a service is removed via
//!   [`EpgStore::retain_services`] or [`EpgStore::clear`].
//! * **`max_events_per_service`** (default [`DEFAULT_MAX_EVENTS_PER_SERVICE`]) —
//!   caps the number of events stored per service. When a service's cap is
//!   reached, new events (by `event_id`) are skipped; existing event_ids are
//!   still updated on version churn.
//!
//! The policy is *skip-until-space* — the same as
//! [`crate::carousel::ModuleReassembler`] — so long-running consumers should
//! call `retain_services` or `clear` periodically to free capacity.
//!
//! # Quickstart
//!
//! ```rust
//! use dvb_si::epg::{EpgStore, ServiceKey};
//! use dvb_si::collect::SectionSetCollector;
//! use chrono::{TimeZone, Utc};
//!
//! let mut store = EpgStore::new();
//!
//! // Feed EIT sections (from a TS demux, file, etc.)
//! // store.feed(&eit_section_bytes)?;
//!
//! let key = ServiceKey {
//!     original_network_id: 1,
//!     transport_stream_id: 1,
//!     service_id: 100,
//! };
//!
//! // Query now/next (requires EIT present/following data to be fed first)
//! let now = Utc.with_ymd_and_hms(2026, 6, 10, 20, 0, 0).unwrap();
//! let (now_evt, next_evt) = store.now_and_next(key, now);
//! if let Some(evt) = now_evt {
//!     println!("Now:  {} (until {})",
//!         evt.event_name.as_deref().unwrap_or("?"),
//!         evt.start_time.map(|t| t + evt.duration.unwrap_or_default())
//!             .map(|e| e.to_string()).unwrap_or_default());
//! }
//! if let Some(evt) = next_evt {
//!     println!("Next: {} at {}",
//!         evt.event_name.as_deref().unwrap_or("?"),
//!         evt.start_time.map(|t| t.to_string()).unwrap_or_default());
//! }
//! // Print tonight's schedule (events from 20:00 to midnight)
//! let tonight = Utc.with_ymd_and_hms(2026, 6, 10, 20, 0, 0).unwrap();
//! let midnight = Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0).unwrap();
//! if let Some(events) = store.schedule(key, tonight, midnight) {
//!     for evt in &events {
//!         println!("{:>5}  {}",
//!             evt.start_time.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(),
//!             evt.event_name.as_deref().unwrap_or("?"));
//!     }
//! }
//! ```
//!
//! # Pruning policy
//!
//! The store accumulates events within its configured caps. To bound growth
//! under schedule churn, use [`EpgStore::retain_services`] to remove services
//! that are no longer of interest and [`EpgStore::clear`] to reset all state at
//! a carousel boundary.  The underlying [`crate::collect::EitCollector`] handles
//! version-driven section-set replacement automatically — when the
//! `version_number` on a sub-table changes, the old partial set is discarded and
//! a new one begins.  Callers scanning a full carousel cycle can `clear()` and
//! start fresh, or `retain_services` to keep only the active service list.

use crate::collect::CollectResult;
use std::collections::HashMap;

/// Logical key identifying a service across the DVB network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceKey {
    /// original_network_id from the EIT/SDT.
    pub original_network_id: u16,
    /// transport_stream_id from the EIT/SDT.
    pub transport_stream_id: u16,
    /// service_id.
    pub service_id: u16,
}

/// A parental rating entry from a
/// [`ParentalRatingDescriptor`](crate::descriptors::parental_rating::ParentalRatingDescriptor).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Rating {
    /// Three-character ISO 3166 country code.
    pub country: String,
    /// Minimum recommended age.
    pub value: u8,
}

/// A content reference identifier entry from a
/// [`ContentIdentifierDescriptor`](crate::descriptors::content_identifier::ContentIdentifierDescriptor).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Crid {
    /// CRID type (0x01 = series, 0x02 = programme, 0x03 = recommendation).
    pub crid_type: u8,
    /// The CRID locator string.
    pub crid: String,
}

/// An item (description-value pair) from an extended event descriptor fragment.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtendedItem {
    /// Item description.
    pub description: String,
    /// Item value.
    pub item: String,
}

/// A content genre nibble triplet from a
/// [`ContentDescriptor`](crate::descriptors::content::ContentDescriptor).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ContentNibble {
    /// Content nibble level 1 (category).
    pub level_1: u8,
    /// Content nibble level 2 (sub-category).
    pub level_2: u8,
    /// User-defined byte.
    pub user: u8,
}

/// A decoded view of an EPG event.
///
/// Extracted from [`crate::collect::CompleteEitEvent`] with commonly needed
/// descriptor fields pre-decoded and extended text concatenated per
/// EN 300 468 §6.2.15.
///
/// # Limitations
///
/// Only the first descriptor of each kind (short_event, content,
/// parental_rating, content_identifier) is decoded per event; EIT events
/// may carry multiple language variants and only the first is taken.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EpgEvent {
    /// 16-bit event_id.
    pub event_id: u16,
    /// Decoded start time (MJD + BCD UTC), if valid.
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Decoded BCD duration, if valid.
    pub duration: Option<core::time::Duration>,
    /// 3-bit running status.
    pub running_status: u8,
    /// free_CA_mode.
    pub free_ca_mode: bool,
    /// Decoded short event name (from
    /// [`ShortEventDescriptor`](crate::descriptors::short_event::ShortEventDescriptor)),
    /// if present and decodeable.
    pub event_name: Option<String>,
    /// Decoded short event text, if present and decodeable.
    pub event_text: Option<String>,
    /// Concatenated extended event text from all
    /// [`ExtendedEventDescriptor`](crate::descriptors::extended_event::ExtendedEventDescriptor)
    /// fragments, per EN 300 468 §6.2.15. Fragments are sorted by
    /// `descriptor_number` and concatenated directly (no separator).
    pub extended_text: Option<String>,
    /// Accumulated extended event items (description, value) from all
    /// [`ExtendedEventDescriptor`](crate::descriptors::extended_event::ExtendedEventDescriptor)
    /// fragments, sorted by `descriptor_number`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub extended_items: Vec<ExtendedItem>,
    /// Content genre entries from
    /// [`ContentDescriptor`](crate::descriptors::content::ContentDescriptor).
    #[cfg_attr(feature = "serde", serde(default))]
    pub content_nibbles: Vec<ContentNibble>,
    /// Parental rating entries from
    /// [`ParentalRatingDescriptor`](crate::descriptors::parental_rating::ParentalRatingDescriptor).
    #[cfg_attr(feature = "serde", serde(default))]
    pub ratings: Vec<Rating>,
    /// CRID entries from
    /// [`ContentIdentifierDescriptor`](crate::descriptors::content_identifier::ContentIdentifierDescriptor).
    #[cfg_attr(feature = "serde", serde(default))]
    pub crids: Vec<Crid>,
}

/// Serialisable service data exposed by [`EpgStore`] serde export.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct ServiceData {
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    service_name: Option<String>,
    events: Vec<EpgEvent>,
}

/// Default cap on distinct [`ServiceKey`] entries (services with cached EPG data).
///
/// 1024 services is generous — a single DVB transponder typically carries 20–50
/// services — while bounding a hostile stream that rotates `service_id` /
/// `transport_stream_id` / `original_network_id` to force unbounded map growth.
pub const DEFAULT_MAX_SERVICES: usize = 1024;

/// Default cap on events stored per service.
///
/// 8192 events (~28 days of 5-minute-granularity schedule entries) is far
/// above real 7-day EPG depth while bounding per-service event accumulation
/// from a hostile stream that rotates `event_id` without re-versioning.
pub const DEFAULT_MAX_EVENTS_PER_SERVICE: usize = 8192;

/// A store for EIT data, providing high-level access to program events.
///
/// Wraps a [`crate::collect::EitCollector`] and maintains a deduplicated,
/// event list per service. Optionally accepts SDT data to attach service names.
///
/// Serde export serializes the cache as a map of `ServiceKey` → service data.
///
/// # Memory bounds
///
/// The store is bounded by two configurable caps (see [module docs](self) for
/// rationale and default values). Use [`with_max_services`](Self::with_max_services)
/// and [`with_max_events_per_service`](Self::with_max_events_per_service) to
/// tune them.
///
/// # Limitations
///
/// Events are deduplicated by `event_id` and stored within the configured
/// per-service cap. Events removed from a re-versioned schedule, or events
/// already in the past, remain in the store until evicted by
/// [`retain_services()`](Self::retain_services) or [`clear()`](Self::clear).
///
/// # Example
///
/// ```no_run
/// use dvb_si::epg::{EpgStore, ServiceKey};
/// use chrono::Utc;
///
/// let mut store = EpgStore::new();
/// // store.feed(&eit_section_bytes).unwrap();
///
/// let key = ServiceKey {
///     original_network_id: 1,
///     transport_stream_id: 1,
///     service_id: 100,
/// };
///
/// let (now_evt, _next) = store.now_and_next(key, Utc::now());
/// if let Some(e) = now_evt {
///     println!("Now playing: {:?}", e.event_name);
/// }
/// ```
#[derive(Debug)]
pub struct EpgStore {
    collector: crate::collect::EitCollector,
    cache: HashMap<ServiceKey, ServiceEpg>,
    max_services: usize,
    max_events_per_service: usize,
}

impl Default for EpgStore {
    fn default() -> Self {
        Self {
            collector: crate::collect::EitCollector::default(),
            cache: HashMap::new(),
            max_services: DEFAULT_MAX_SERVICES,
            max_events_per_service: DEFAULT_MAX_EVENTS_PER_SERVICE,
        }
    }
}

#[derive(Debug, Default)]
struct ServiceEpg {
    service_name: Option<String>,
    /// Deduplicated by event_id. Latest version wins (later inserts overwrite).
    events: HashMap<u16, EpgEvent>,
}

impl EpgStore {
    /// Create a new, empty EPG store with default caps
    /// ([`DEFAULT_MAX_SERVICES`], [`DEFAULT_MAX_EVENTS_PER_SERVICE`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the service-count cap (default [`DEFAULT_MAX_SERVICES`]).
    /// When the cap is reached, events for new services are skipped until
    /// [`retain_services`](Self::retain_services) or [`clear`](Self::clear)
    /// frees capacity.
    #[must_use]
    pub fn with_max_services(mut self, max_services: usize) -> Self {
        self.max_services = max_services;
        self
    }

    /// Replace the per-service event cap (default
    /// [`DEFAULT_MAX_EVENTS_PER_SERVICE`]). When a service reaches its cap, new
    /// events (by `event_id`) are skipped; existing event_ids are still updated
    /// on version churn.
    #[must_use]
    pub fn with_max_events_per_service(mut self, max_events_per_service: usize) -> Self {
        self.max_events_per_service = max_events_per_service;
        self
    }

    /// Replace the underlying collector's logical-key cap (default
    /// [`crate::collect::DEFAULT_MAX_LOGICAL_KEYS`]). See
    /// [`crate::collect::EitCollector::with_max_logical_keys`].
    #[must_use]
    pub fn with_collector_max_logical_keys(mut self, max_logical_keys: usize) -> Self {
        self.collector = self.collector.with_max_logical_keys(max_logical_keys);
        self
    }

    /// Feed one EIT section into the store.
    ///
    /// If a table becomes complete, its events are merged into the cache
    /// (deduplicated by `event_id`, later insertions overwrite).
    ///
    /// # Errors
    ///
    /// Returns a [`crate::collect::CollectError`] if the section is malformed.
    pub fn feed(&mut self, bytes: &[u8]) -> CollectResult<()> {
        self.feed_with_pid(None, bytes)
    }

    /// Feed one EIT section with PID context into the store.
    pub fn feed_with_pid(&mut self, pid: Option<u16>, bytes: &[u8]) -> CollectResult<()> {
        if let Some(completed) = self.collector.push_section_with_pid(pid, bytes)? {
            let tables = completed.tables()?;
            for table in &tables {
                let key = ServiceKey {
                    original_network_id: table.original_network_id,
                    transport_stream_id: table.transport_stream_id,
                    service_id: table.service_id,
                };
                if self.cache.len() >= self.max_services && !self.cache.contains_key(&key) {
                    continue;
                }
                let svc = self.cache.entry(key).or_default();
                for event in &table.events {
                    if svc.events.len() >= self.max_events_per_service
                        && !svc.events.contains_key(&event.event_id)
                    {
                        continue;
                    }
                    svc.events.insert(event.event_id, event_to_epg(event));
                }
            }
        }
        Ok(())
    }

    /// Feed completed SDT data to attach service names.
    ///
    /// Accepts a parsed [`crate::collect::CompleteSdt`] from a
    /// [`crate::collect::SectionSetCollector`].
    pub fn feed_sdt(&mut self, sdt: &crate::collect::CompleteSdt<'_>) {
        for svc in &sdt.services {
            let key = ServiceKey {
                original_network_id: sdt.original_network_id,
                transport_stream_id: sdt.transport_stream_id,
                service_id: svc.service_id,
            };
            let entry = self.cache.entry(key).or_default();
            entry.service_name = extract_service_name(svc.descriptors.descriptors());
        }
    }

    /// Get the "now" and "next" events for a service.
    ///
    /// Searches the event list for the given service and returns the event
    /// currently on-air ("now") and the next upcoming event ("next") based
    /// on reference time `at`.
    ///
    /// "now" is the event where `at` falls within `[start, start + duration)`.
    /// "next" is the event with the earliest `start_time` strictly after `at`
    /// (not just the first such event in arbitrary iteration order).
    ///
    /// An event ending exactly at `at` is NOT considered "now" (exclusive end).
    ///
    /// Returns `(None, None)` when the service is unknown or no event matches.
    pub fn now_and_next(
        &self,
        key: ServiceKey,
        at: chrono::DateTime<chrono::Utc>,
    ) -> (Option<&EpgEvent>, Option<&EpgEvent>) {
        let Some(svc) = self.cache.get(&key) else {
            return (None, None);
        };

        let now = svc.events.values().find(|e| {
            if let (Some(start), Some(dur)) = (e.start_time, e.duration) {
                let end = start + dur;
                return at >= start && at < end;
            }
            false
        });

        let next = svc
            .events
            .values()
            .filter(|e| {
                if let Some(start) = e.start_time {
                    start > at
                } else {
                    false
                }
            })
            .min_by_key(|e| e.start_time);

        (now, next)
    }

    /// Query events with start times in the half-open range `[from, to)`.
    ///
    /// Returns events sorted by start time (valid times first, then by
    /// event_id). Events without a decodable start time are excluded.
    #[must_use]
    pub fn schedule(
        &self,
        key: ServiceKey,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
    ) -> Option<Vec<&EpgEvent>> {
        let svc = self.cache.get(&key)?;
        let mut events: Vec<&EpgEvent> = svc
            .events
            .values()
            .filter(|e| {
                if let Some(start) = e.start_time {
                    start >= from && start < to
                } else {
                    false
                }
            })
            .collect();
        events.sort_by(|a, b| cmp_event_by_start(a, b));
        Some(events)
    }

    /// Return the service name for a given key, if SDT data was fed.
    #[must_use]
    pub fn service_name(&self, key: ServiceKey) -> Option<&str> {
        self.cache.get(&key).and_then(|s| s.service_name.as_deref())
    }

    /// Iterate the [`ServiceKey`]s of every service with cached EIT data, so
    /// callers can walk the whole EPG (e.g. render a grid) without knowing the
    /// service ids in advance. Order is unspecified.
    pub fn services(&self) -> impl Iterator<Item = ServiceKey> + '_ {
        self.cache.keys().copied()
    }

    /// Return all events for a service, sorted by start time
    /// (events without a valid start time sort last, then by event_id).
    #[must_use]
    pub fn events(&self, key: ServiceKey) -> Option<Vec<&EpgEvent>> {
        let svc = self.cache.get(&key)?;
        let mut events: Vec<&EpgEvent> = svc.events.values().collect();
        events.sort_by(|a, b| cmp_event_by_start(a, b));
        Some(events)
    }

    /// Return the number of services with cached EIT data.
    #[must_use]
    pub fn service_count(&self) -> usize {
        self.cache.len()
    }

    /// Return the total number of events across all services.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.cache.values().map(|s| s.events.len()).sum()
    }

    /// Retain only services matching the given predicate.
    ///
    /// Both the event cache and the underlying collector partial state
    /// for rejected keys are removed.
    pub fn retain_services<F>(&mut self, mut keep: F)
    where
        F: FnMut(&ServiceKey) -> bool,
    {
        self.cache.retain(|key, _| keep(key));
        self.collector.retain_logical(|lk| {
            keep(&ServiceKey {
                original_network_id: lk.original_network_id,
                transport_stream_id: lk.transport_stream_id,
                service_id: lk.service_id,
            })
        });
    }

    /// Clear all cached EIT data and reset the internal collector.
    pub fn clear(&mut self) {
        self.collector.clear();
        self.cache.clear();
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for EpgStore {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.cache.len()))?;
        for (key, svc) in &self.cache {
            let data = ServiceData {
                service_name: svc.service_name.clone(),
                events: {
                    let mut evts: Vec<EpgEvent> = svc.events.values().cloned().collect();
                    evts.sort_by(cmp_event_by_start);
                    evts
                },
            };
            let key_str = format!(
                "{}-{}-{}",
                key.original_network_id, key.transport_stream_id, key.service_id
            );
            m.serialize_entry(&key_str, &data)?;
        }
        m.end()
    }
}

fn cmp_event_by_start(a: &EpgEvent, b: &EpgEvent) -> std::cmp::Ordering {
    match (a.start_time, b.start_time) {
        (Some(at), Some(bt)) => at.cmp(&bt).then_with(|| a.event_id.cmp(&b.event_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.event_id.cmp(&b.event_id),
    }
}

fn event_to_epg(e: &crate::collect::CompleteEitEvent<'_>) -> EpgEvent {
    let (event_name, event_text) = extract_short_event(e.descriptors.descriptors());
    let (extended_text, extended_items) = extract_extended(e.descriptors.descriptors());
    let content_nibbles = extract_content(e.descriptors.descriptors());
    let ratings = extract_ratings(e.descriptors.descriptors());
    let crids = extract_crids(e.descriptors.descriptors());

    EpgEvent {
        event_id: e.event_id,
        start_time: e.start_time(),
        duration: e.duration(),
        running_status: e.running_status,
        free_ca_mode: e.free_ca_mode,
        event_name,
        event_text,
        extended_text,
        extended_items,
        content_nibbles,
        ratings,
        crids,
    }
}

fn extract_short_event(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> (Option<String>, Option<String>) {
    for desc in descriptors {
        if let Ok(crate::descriptors::AnyDescriptor::ShortEvent(se)) = desc {
            return (
                Some(se.event_name.decode().into_owned()),
                Some(se.text.decode().into_owned()),
            );
        }
    }
    (None, None)
}

struct ExtendedFragment {
    descriptor_number: u8,
    text: String,
    items: Vec<ExtendedItem>,
}

fn extract_extended(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> (Option<String>, Vec<ExtendedItem>) {
    use crate::descriptors::AnyDescriptor;

    let mut fragments: Vec<ExtendedFragment> = descriptors
        .iter()
        .filter_map(|d| {
            if let Ok(AnyDescriptor::ExtendedEvent(ee)) = d {
                let text = ee.text.decode().into_owned();
                let items: Vec<ExtendedItem> = ee
                    .items
                    .iter()
                    .map(|i| ExtendedItem {
                        description: i.description.decode().into_owned(),
                        item: i.value.decode().into_owned(),
                    })
                    .collect();
                if !text.is_empty() || !items.is_empty() {
                    Some(ExtendedFragment {
                        descriptor_number: ee.descriptor_number,
                        text,
                        items,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    if fragments.is_empty() {
        return (None, Vec::new());
    }

    // Sort by descriptor_number per EN 300 468 §6.2.15.
    fragments.sort_by_key(|f| f.descriptor_number);

    let extended_text: String = fragments.iter().map(|f| f.text.as_str()).collect();

    let extended_items: Vec<ExtendedItem> = fragments.into_iter().flat_map(|f| f.items).collect();

    let text = if extended_text.is_empty() {
        None
    } else {
        Some(extended_text)
    };

    (text, extended_items)
}

fn extract_content(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> Vec<ContentNibble> {
    for desc in descriptors {
        if let Ok(crate::descriptors::AnyDescriptor::Content(ct)) = desc {
            return ct
                .entries
                .iter()
                .map(|e| ContentNibble {
                    level_1: e.nibble_1,
                    level_2: e.nibble_2,
                    user: e.user_byte,
                })
                .collect();
        }
    }
    Vec::new()
}

fn extract_ratings(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> Vec<Rating> {
    for desc in descriptors {
        if let Ok(crate::descriptors::AnyDescriptor::ParentalRating(pr)) = desc {
            return pr
                .entries
                .iter()
                .map(|e| Rating {
                    country: e.country_code.as_str().into_owned(),
                    value: e.rating,
                })
                .collect();
        }
    }
    Vec::new()
}

fn extract_crids(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> Vec<Crid> {
    use crate::descriptors::content_identifier::CridLocation;
    for desc in descriptors {
        if let Ok(crate::descriptors::AnyDescriptor::ContentIdentifier(ci)) = desc {
            return ci
                .entries
                .iter()
                .filter_map(|e| match e.location {
                    CridLocation::Inline(bytes) => {
                        let s = String::from_utf8_lossy(bytes).into_owned();
                        Some(Crid {
                            crid_type: e.crid_type,
                            crid: s,
                        })
                    }
                    _ => None,
                })
                .collect();
        }
    }
    Vec::new()
}

fn extract_service_name(
    descriptors: &[crate::Result<crate::descriptors::AnyDescriptor<'_>>],
) -> Option<String> {
    for desc in descriptors {
        if let Ok(crate::descriptors::AnyDescriptor::Service(svc)) = desc {
            return Some(svc.service_name.decode().into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Build the bytes of a minimal short_event_descriptor.
    fn short_event_bytes(name: &[u8], text: &[u8]) -> Vec<u8> {
        let lang = b"eng";
        let mut v = Vec::new();
        v.push(0x4Du8); // tag
        v.push((3 + 1 + name.len() + 1 + text.len()) as u8); // length
        v.extend_from_slice(lang);
        v.push(name.len() as u8);
        v.extend_from_slice(name);
        v.push(text.len() as u8);
        v.extend_from_slice(text);
        v
    }

    /// Build the bytes of a minimal EIT present/following section
    /// with one event. Returns bytes formated as a complete TS section
    /// (including CRC-32).
    #[allow(clippy::too_many_arguments)]
    fn eit_pf_section(
        service_id: u16,
        ts_id: u16,
        on_id: u16,
        event_id: u16,
        version: u8,
        start_raw: [u8; 5],
        dur_raw: [u8; 3],
        descriptors: &[u8],
    ) -> Vec<u8> {
        let table_id = 0x4Eu8;

        // Header: 3 + ext_header(5) + post_ext(6) = 14
        // Event: 12 + descriptors.len()
        // CRC: 4
        let ev_len = 12 + descriptors.len();
        let section_length = 5 + 6 + ev_len + 4;
        let total = 3 + section_length;

        let mut buf = vec![0u8; total];
        buf[0] = table_id;
        buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&service_id.to_be_bytes());
        // reserved(2)=0b11, version, current_next=1
        buf[5] = 0xC0 | ((version & 0x1F) << 1) | 0x01;
        buf[6] = 0; // section_number
        buf[7] = 0; // last_section_number
        buf[8..10].copy_from_slice(&ts_id.to_be_bytes());
        buf[10..12].copy_from_slice(&on_id.to_be_bytes());
        buf[12] = 0; // segment_last_section_number
        buf[13] = 0x5F; // last_table_id

        // Event
        let ev_off = 14;
        buf[ev_off..ev_off + 2].copy_from_slice(&event_id.to_be_bytes());
        buf[ev_off + 2..ev_off + 7].copy_from_slice(&start_raw);
        buf[ev_off + 7..ev_off + 10].copy_from_slice(&dur_raw);
        let dll = descriptors.len() as u16;
        buf[ev_off + 10] = ((dll >> 8) as u8) & 0x0F;
        buf[ev_off + 11] = (dll & 0xFF) as u8;
        buf[ev_off + 12..ev_off + 12 + descriptors.len()].copy_from_slice(descriptors);

        // CRC-32
        let crc_pos = total - 4;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..].copy_from_slice(&crc.to_be_bytes());
        buf
    }

    /// Build start-time raw bytes (16-bit MJD + 24-bit BCD) for a given
    /// year/month/day/hour.
    fn start_raw(year: i32, month: u32, day: u32, hour: u32) -> [u8; 5] {
        let mjd = mjd_approx(year, month, day);
        let mjd_bytes = mjd.to_be_bytes();
        let bcd_hour = ((hour / 10 * 16) + (hour % 10)) as u8;
        [
            mjd_bytes[0],
            mjd_bytes[1],
            bcd_hour,
            0, // minute BCD
            0, // second BCD
        ]
    }

    /// Quick MJD approximation for test dates (2026-06-10 = MJD 61785).
    fn mjd_approx(year: i32, month: u32, day: u32) -> u16 {
        assert!(
            (year, month, day) == (2026, 6, 10),
            "mjd_approx only supports 2026-06-10"
        );
        61785
    }

    /// Build content_descriptor (tag 0x54) wire bytes.
    fn content_descriptor_bytes(entries: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut v = vec![0x54u8, (entries.len() * 2) as u8];
        for &(n1, n2, u) in entries {
            v.push((n1 << 4) | n2);
            v.push(u);
        }
        v
    }

    /// Build parental_rating_descriptor (tag 0x55) wire bytes.
    fn parental_rating_bytes(entries: &[([u8; 3], u8)]) -> Vec<u8> {
        let mut v = vec![0x55u8, (entries.len() * 4) as u8];
        for (country, rating) in entries {
            v.extend_from_slice(country);
            v.push(*rating);
        }
        v
    }

    /// Build content_identifier_descriptor (tag 0x76) wire bytes with inline
    /// CRIDs.
    fn content_identifier_bytes(entries: &[(u8, &[u8])]) -> Vec<u8> {
        let body_len: usize = entries.iter().map(|(_, data)| 2 + data.len()).sum();
        let mut v = vec![0x76u8, body_len as u8];
        for (crid_type, data) in entries {
            v.push(crid_type << 2); // location=0b00 inline
            v.push(data.len() as u8);
            v.extend_from_slice(data);
        }
        v
    }

    // ------------------------------------------------------------------
    // Basic tests
    // ------------------------------------------------------------------

    #[test]
    fn new_store_is_empty() {
        let store = EpgStore::new();
        assert_eq!(store.service_count(), 0);
        assert_eq!(store.event_count(), 0);
    }

    #[test]
    fn feed_empty_is_error() {
        let mut store = EpgStore::new();
        assert!(store.feed(&[]).is_err());
    }

    #[test]
    fn now_and_next_no_data_returns_none() {
        let store = EpgStore::new();
        let now = Utc::now();
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        assert_eq!(store.now_and_next(key, now), (None, None));
    }

    #[test]
    fn service_key_ordering() {
        let a = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 100,
        };
        let b = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 2,
            service_id: 200,
        };
        assert!(a < b);
    }

    fn empty_event(
        id: u16,
        start: Option<chrono::DateTime<chrono::Utc>>,
        dur: Option<core::time::Duration>,
    ) -> EpgEvent {
        EpgEvent {
            event_id: id,
            start_time: start,
            duration: dur,
            running_status: 0,
            free_ca_mode: false,
            event_name: None,
            event_text: None,
            extended_text: None,
            extended_items: Vec::new(),
            content_nibbles: Vec::new(),
            ratings: Vec::new(),
            crids: Vec::new(),
        }
    }

    #[test]
    fn events_sorts_valid_before_invalid() {
        let valid = empty_event(
            1,
            Some(Utc::now()),
            Some(core::time::Duration::from_secs(3600)),
        );
        let invalid = empty_event(2, None, None);

        let mut events = [&invalid, &valid];
        events.sort_by(|a, b| cmp_event_by_start(a, b));
        assert_eq!(events[0].event_id, 1);
        assert_eq!(events[1].event_id, 2);
    }

    // ------------------------------------------------------------------
    // §6.2.15 extended event text chaining
    // ------------------------------------------------------------------

    #[test]
    fn extended_text_chaining_per_spec_6_2_15() {
        use crate::descriptors::extended_event::ExtendedEventDescriptor;
        use crate::descriptors::AnyDescriptor;
        use crate::text::{DvbText, LangCode};

        // Fragment 1: descriptor_number=2, last_descriptor_number=3
        // "The quick " + item ("Director", "Alice")
        let frag1 = ExtendedEventDescriptor {
            descriptor_number: 2,
            last_descriptor_number: 3,
            language_code: LangCode(*b"eng"),
            items: vec![crate::descriptors::extended_event::ExtendedEventItem {
                description: DvbText::new(b"Director"),
                value: DvbText::new(b"Alice"),
            }],
            text: DvbText::new(b"The quick "),
        };

        // Fragment 2: descriptor_number=0, last_descriptor_number=3
        // "brown fox" + item ("Year", "2026")
        let frag2 = ExtendedEventDescriptor {
            descriptor_number: 0,
            last_descriptor_number: 3,
            language_code: LangCode(*b"eng"),
            items: vec![crate::descriptors::extended_event::ExtendedEventItem {
                description: DvbText::new(b"Year"),
                value: DvbText::new(b"2026"),
            }],
            text: DvbText::new(b"brown fox"),
        };

        // Fragment 3: descriptor_number=3, last_descriptor_number=3
        // "jumps." + no items
        let frag3 = ExtendedEventDescriptor {
            descriptor_number: 3,
            last_descriptor_number: 3,
            language_code: LangCode(*b"eng"),
            items: vec![],
            text: DvbText::new(b"jumps."),
        };

        // Fragment 4: descriptor_number=1, last_descriptor_number=3
        // empty text + item ("Genre", "Thriller") — dropped by the chaining
        // helper (text is empty but items present → included)
        let frag4 = ExtendedEventDescriptor {
            descriptor_number: 1,
            last_descriptor_number: 3,
            language_code: LangCode(*b"eng"),
            items: vec![crate::descriptors::extended_event::ExtendedEventItem {
                description: DvbText::new(b"Genre"),
                value: DvbText::new(b"Thriller"),
            }],
            text: DvbText::new(b""),
        };

        // Feed fragments out of order via AnyDescriptor.
        let descriptors: Vec<crate::Result<AnyDescriptor<'_>>> = vec![
            Ok(AnyDescriptor::ExtendedEvent(frag1)), // dn=2
            Ok(AnyDescriptor::ExtendedEvent(frag4)), // dn=1
            Ok(AnyDescriptor::ExtendedEvent(frag3)), // dn=3
            Ok(AnyDescriptor::ExtendedEvent(frag2)), // dn=0
        ];

        let (text, items) = extract_extended(&descriptors);

        // Text concatenated in descriptor_number order: 0,1,2,3
        assert_eq!(text.as_deref(), Some("brown foxThe quick jumps."));

        // Items accumulated in descriptor_number order: dn=0 ("Year"/"2026"),
        // dn=1 ("Genre"/"Thriller"), dn=2 ("Director"/"Alice"), dn=3 (none)
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0],
            ExtendedItem {
                description: "Year".into(),
                item: "2026".into()
            }
        );
        assert_eq!(
            items[1],
            ExtendedItem {
                description: "Genre".into(),
                item: "Thriller".into()
            }
        );
        assert_eq!(
            items[2],
            ExtendedItem {
                description: "Director".into(),
                item: "Alice".into()
            }
        );
    }

    // ------------------------------------------------------------------
    // now_and_next boundary correctness
    // ------------------------------------------------------------------

    #[test]
    fn now_and_next_event_boundary() {
        let t1000 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();
        let t1100 = Utc.with_ymd_and_hms(2026, 6, 10, 11, 0, 0).unwrap();
        let t1200 = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        // Event 1: 10:00-11:00
        // Event 2: 12:00-13:00
        let sec = core::time::Duration::from_secs(3600);
        let ev1 = EpgEvent {
            event_id: 1,
            start_time: Some(t1000),
            duration: Some(sec),
            running_status: 0,
            free_ca_mode: false,
            event_name: Some("Event 1".into()),
            event_text: None,
            extended_text: None,
            extended_items: vec![],
            content_nibbles: vec![],
            ratings: vec![],
            crids: vec![],
        };
        let ev2 = EpgEvent {
            event_id: 2,
            start_time: Some(t1200),
            duration: Some(sec),
            running_status: 0,
            free_ca_mode: false,
            event_name: Some("Event 2".into()),
            event_text: None,
            extended_text: None,
            extended_items: vec![],
            content_nibbles: vec![],
            ratings: vec![],
            crids: vec![],
        };

        // Set up store manually (bypass feed).
        let mut store = EpgStore::new();
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let svc = store.cache.entry(key).or_default();
        svc.events.insert(1, ev1);
        svc.events.insert(2, ev2);

        // At 10:30 — now=Event 1, next=Event 2
        let at = Utc.with_ymd_and_hms(2026, 6, 10, 10, 30, 0).unwrap();
        let (now, next) = store.now_and_next(key, at);
        assert_eq!(now.unwrap().event_id, 1);
        assert_eq!(next.unwrap().event_id, 2);

        // At 11:00 exactly — event 1 just ended (exclusive end),
        // now=None, next=Event 2
        let (now, next) = store.now_and_next(key, t1100);
        assert!(now.is_none(), "event ending at query time must NOT be now");
        assert_eq!(next.unwrap().event_id, 2);

        // At 12:00 exactly — now=Event 2 (start == at, inclusive start),
        // next=None
        let (now, next) = store.now_and_next(key, t1200);
        assert_eq!(now.unwrap().event_id, 2);
        assert!(next.is_none());
    }

    // ------------------------------------------------------------------
    // now_and_next: earliest-future-event selection (not arbitrary order)
    // ------------------------------------------------------------------

    #[test]
    fn now_and_next_returns_earliest_future_event() {
        // Build a service with events out of insertion order:
        // Event 14:00 inserted first, Event 12:00 second, Event 16:00 third.
        // Query at 10:00 — "next" must be Event 12:00 (earliest future),
        // not Event 14:00 (which would win if the code used arbitrary
        // HashMap iteration order).
        let t1200 = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
        let t1400 = Utc.with_ymd_and_hms(2026, 6, 10, 14, 0, 0).unwrap();
        let t1600 = Utc.with_ymd_and_hms(2026, 6, 10, 16, 0, 0).unwrap();
        let t1000 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();

        let sec = core::time::Duration::from_secs(3600);

        fn named_event(
            id: u16,
            start: chrono::DateTime<chrono::Utc>,
            dur: core::time::Duration,
            name: &str,
        ) -> EpgEvent {
            EpgEvent {
                event_id: id,
                start_time: Some(start),
                duration: Some(dur),
                running_status: 0,
                free_ca_mode: false,
                event_name: Some(name.into()),
                event_text: None,
                extended_text: None,
                extended_items: vec![],
                content_nibbles: vec![],
                ratings: vec![],
                crids: vec![],
            }
        }

        let mut store = EpgStore::new();
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let svc = store.cache.entry(key).or_default();
        // Insert out of order — 14:00 first, 12:00 second, 16:00 third
        svc.events.insert(3, named_event(3, t1400, sec, "Event 14"));
        svc.events.insert(1, named_event(1, t1200, sec, "Event 12"));
        svc.events.insert(2, named_event(2, t1600, sec, "Event 16"));

        // "next" at 10:00 must be the earliest future — event 1 at 12:00
        let (_now, next) = store.now_and_next(key, t1000);
        let next = next.expect("next event must exist");
        assert_eq!(
            next.event_id, 1,
            "next must be earliest future event (12:00), not first in iteration order"
        );
    }

    // ------------------------------------------------------------------
    // extract_content / extract_ratings / extract_crids through feed
    // ------------------------------------------------------------------

    #[test]
    fn extract_content_ratings_crids_through_feed() {
        let content = content_descriptor_bytes(&[(3, 1, 0xAA), (4, 2, 0xBB)]);
        let ratings = parental_rating_bytes(&[(*b"FRA", 0x05), (*b"GBR", 0x01)]);
        let crids = content_identifier_bytes(&[
            (0x01, b"crid://bbc.co.uk/prog123"),
            (0x03, b"crid://bbc.co.uk/rec456"),
        ]);

        let mut descriptors = Vec::new();
        descriptors.extend_from_slice(&content);
        descriptors.extend_from_slice(&ratings);
        descriptors.extend_from_slice(&crids);

        let sr = start_raw(2026, 6, 10, 10);
        let eit = eit_pf_section(100, 1, 1, 1, 0, sr, [1, 0, 0], &descriptors);

        let mut store = EpgStore::new();
        store.feed(&eit).unwrap();

        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let events = store.events(key).unwrap();
        assert_eq!(events.len(), 1);
        let ev = &events[0];

        assert_eq!(ev.content_nibbles.len(), 2);
        assert_eq!(
            ev.content_nibbles[0],
            ContentNibble {
                level_1: 3,
                level_2: 1,
                user: 0xAA
            }
        );
        assert_eq!(
            ev.content_nibbles[1],
            ContentNibble {
                level_1: 4,
                level_2: 2,
                user: 0xBB
            }
        );

        assert_eq!(ev.ratings.len(), 2);
        assert_eq!(ev.ratings[0].country, "FRA");
        assert_eq!(ev.ratings[0].value, 0x05);
        assert_eq!(ev.ratings[1].country, "GBR");
        assert_eq!(ev.ratings[1].value, 0x01);

        assert_eq!(ev.crids.len(), 1 + 1); // one series + one recommendation
        assert_eq!(ev.crids[0].crid_type, 0x01);
        assert_eq!(ev.crids[0].crid, "crid://bbc.co.uk/prog123");
        assert_eq!(ev.crids[1].crid_type, 0x03);
        assert_eq!(ev.crids[1].crid, "crid://bbc.co.uk/rec456");
    }

    #[test]
    fn extract_service_name_through_feed_sdt() {
        use crate::collect::SectionSetCollector;

        // Build a service_descriptor (tag 0x48) with provider="BBC", service_name="BBC ONE HD"
        let svc_desc = {
            let provider = b"BBC";
            let name = b"BBC ONE HD";
            let mut v = vec![0x48u8, (1 + 1 + provider.len() + 1 + name.len()) as u8];
            v.push(0x01); // service_type = TV SD
            v.push(provider.len() as u8);
            v.extend_from_slice(provider);
            v.push(name.len() as u8);
            v.extend_from_slice(name);
            v
        };

        // Build an SDT section (table_id 0x42) with one service.
        let sdt_bytes = {
            let dll = svc_desc.len() as u16;
            // Service entry: 5 bytes header + descriptors
            let svc_entry_len = 5 + dll as usize;
            // Section: 3 (header) + 5 (ext) + 3 (post_ext) = 11 + svc + 4 (crc)
            let section_length: u16 = 5 + 3 + svc_entry_len as u16 + 4;
            let mut buf = vec![0u8; 3 + section_length as usize];
            buf[0] = 0x42; // SDT actual
            buf[1] = 0xB0 | ((section_length >> 8) as u8 & 0x0F);
            buf[2] = (section_length & 0xFF) as u8;
            buf[3..5].copy_from_slice(&1u16.to_be_bytes()); // ts_id
            buf[5] = 0xC1; // version=0, cni=1
            buf[6] = 0; // section_number
            buf[7] = 0; // last_section_number
            buf[8..10].copy_from_slice(&1u16.to_be_bytes()); // original_network_id
            buf[10] = 0xFF; // reserved

            // Service entry
            let off = 11;
            buf[off..off + 2].copy_from_slice(&100u16.to_be_bytes()); // service_id=100
            buf[off + 2] = 0xFC; // flags
            buf[off + 3] = ((dll >> 8) as u8) & 0x0F;
            buf[off + 4] = (dll & 0xFF) as u8;
            buf[off + 5..off + 5 + svc_desc.len()].copy_from_slice(&svc_desc);

            // CRC
            let crc_off = buf.len() - 4;
            let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_off]);
            buf[crc_off..].copy_from_slice(&crc.to_be_bytes());
            buf
        };

        let mut collector = SectionSetCollector::new();
        let complete = collector.push_section(&sdt_bytes).unwrap().unwrap();
        let sdt = complete.sdt().unwrap();

        let mut store = EpgStore::new();
        store.feed_sdt(&sdt);

        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        assert_eq!(store.service_name(key), Some("BBC ONE HD"));
        assert_eq!(store.service_count(), 1);
    }

    // ------------------------------------------------------------------
    // Version churn: bounded growth
    // ------------------------------------------------------------------

    #[test]
    fn version_churn_bounded_growth() {
        // Feed an event, then feed the same event_id with updated data.
        // Store size must stay at 1 event.
        let s = |hh: u32| {
            let t = Utc.with_ymd_and_hms(2026, 6, 10, hh, 0, 0).unwrap();
            let days = 61785u16; // MJD for 2026-06-10
            let mjd_bytes = days.to_be_bytes();
            let bcd_time = [(hh / 10 * 16 + hh % 10) as u8, 0, 0];
            (
                [
                    mjd_bytes[0],
                    mjd_bytes[1],
                    bcd_time[0],
                    bcd_time[1],
                    bcd_time[2],
                ],
                t,
            )
        };

        let (start1, _) = s(10);
        let (start2, _) = s(14);

        let desc1 = short_event_bytes(b"News at 10", b"");
        let desc2 = short_event_bytes(b"News at 14", b"");

        let eit1 = eit_pf_section(100, 1, 1, 1, 0, start1, [1, 0, 0], &desc1);
        let eit2 = eit_pf_section(100, 1, 1, 1, 1, start2, [1, 0, 0], &desc2);

        let mut store = EpgStore::new();
        store.feed(&eit1).unwrap();
        assert_eq!(store.event_count(), 1);
        store.feed(&eit2).unwrap();
        // Same event_id should overwrite, not duplicate
        assert_eq!(store.event_count(), 1);

        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let evts = store.events(key).unwrap();
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].event_name.as_deref(), Some("News at 14"));
    }

    // ------------------------------------------------------------------
    // schedule range query
    // ------------------------------------------------------------------

    #[test]
    fn schedule_range_query() {
        let t0900 = Utc.with_ymd_and_hms(2026, 6, 10, 9, 0, 0).unwrap();
        let t1000 = Utc.with_ymd_and_hms(2026, 6, 10, 10, 0, 0).unwrap();
        let t1100 = Utc.with_ymd_and_hms(2026, 6, 10, 11, 0, 0).unwrap();
        let t1200 = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();

        let sec = core::time::Duration::from_secs(1800);
        let mut store = EpgStore::new();
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let svc = store.cache.entry(key).or_default();
        for (id, t) in [(1, t0900), (2, t1000), (3, t1100)] {
            svc.events.insert(
                id,
                EpgEvent {
                    event_id: id,
                    start_time: Some(t),
                    duration: Some(sec),
                    running_status: 0,
                    free_ca_mode: false,
                    event_name: Some(format!("Event {id}")),
                    event_text: None,
                    extended_text: None,
                    extended_items: vec![],
                    content_nibbles: vec![],
                    ratings: vec![],
                    crids: vec![],
                },
            );
        }

        // [10:00, 12:00) → events 2 and 3
        let events = store.schedule(key, t1000, t1200).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, 2);
        assert_eq!(events[1].event_id, 3);

        // [12:00, 13:00) → empty
        let events = store.schedule(key, t1200, t1100).unwrap();
        assert!(events.is_empty());
    }

    // ------------------------------------------------------------------
    // Cap enforcement: max_services bounds service-count growth
    // ------------------------------------------------------------------

    #[test]
    fn max_services_capped() {
        // Feed 3 distinct services with a cap of 2 — only the first 2 should
        // be retained; the third is skipped until space frees.
        let mut store = EpgStore::new().with_max_services(2);

        let desc = short_event_bytes(b"Test", b"");

        // Service 100
        let sr1 = start_raw(2026, 6, 10, 10);
        let eit1 = eit_pf_section(100, 1, 1, 1, 0, sr1, [1, 0, 0], &desc);
        store.feed(&eit1).unwrap();
        assert_eq!(store.service_count(), 1);

        // Service 200
        let sr2 = start_raw(2026, 6, 10, 11);
        let eit2 = eit_pf_section(200, 1, 1, 3, 0, sr2, [1, 0, 0], &desc);
        store.feed(&eit2).unwrap();
        assert_eq!(store.service_count(), 2);

        // Service 300 — should be skipped (cap 2 already hit, new key)
        let sr3 = start_raw(2026, 6, 10, 12);
        let eit3 = eit_pf_section(300, 1, 1, 5, 0, sr3, [1, 0, 0], &desc);
        store.feed(&eit3).unwrap();
        assert_eq!(
            store.service_count(),
            2,
            "third service must be rejected when cap is full"
        );

        // Verify service 300 has no entry
        let key300 = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 300,
        };
        assert!(
            store.events(key300).is_none(),
            "rejected service must not appear"
        );

        // Clearing frees space — service 300 can now be stored
        store.clear();
        store.feed(&eit3).unwrap();
        assert_eq!(store.service_count(), 1);
        assert!(store.events(key300).is_some());
    }

    // ------------------------------------------------------------------
    // Cap enforcement: max_events_per_service bounds per-service events
    // ------------------------------------------------------------------

    #[test]
    fn max_events_per_service_capped() {
        // Feed 4 distinct event_ids into one service with a cap of 3.
        // The 4th event must be skipped.
        let mut store = EpgStore::new().with_max_events_per_service(3);

        let desc = short_event_bytes(b"Test", b"");
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };

        for (version, (event_id, hour)) in [(10, 10u32), (20, 11), (30, 12), (40, 13)]
            .iter()
            .enumerate()
        {
            let sr = start_raw(2026, 6, 10, *hour);
            let eit = eit_pf_section(100, 1, 1, *event_id, version as u8, sr, [1, 0, 0], &desc);
            store.feed(&eit).unwrap();
        }

        assert_eq!(store.event_count(), 3, "4th event must be skipped at cap 3");

        // Version churn on existing event_id still works:
        let sr_v2 = start_raw(2026, 6, 10, 15);
        let eit_v2 = eit_pf_section(100, 1, 1, 10, 1, sr_v2, [1, 0, 0], &desc);
        store.feed(&eit_v2).unwrap();
        assert_eq!(
            store.event_count(),
            3,
            "version churn on existing event_id must not increase count"
        );

        let evts = store.events(key).unwrap();
        let ev10 = evts.iter().find(|e| e.event_id == 10).unwrap();
        assert_eq!(
            ev10.event_name.as_deref(),
            Some("Test"),
            "existing event updated"
        );
    }

    // ------------------------------------------------------------------
    // serde round-trip
    // ------------------------------------------------------------------

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_store_as_json() {
        let t = Utc.with_ymd_and_hms(2026, 6, 10, 20, 0, 0).unwrap();
        let mut store = EpgStore::new();
        let key = ServiceKey {
            original_network_id: 1,
            transport_stream_id: 1,
            service_id: 100,
        };
        let svc = store.cache.entry(key).or_default();
        svc.service_name = Some("BBC One".into());
        svc.events.insert(
            1,
            EpgEvent {
                event_id: 1,
                start_time: Some(t),
                duration: Some(core::time::Duration::from_secs(3600)),
                running_status: 4,
                free_ca_mode: false,
                event_name: Some("The News".into()),
                event_text: Some("Today's headlines".into()),
                extended_text: None,
                extended_items: vec![],
                content_nibbles: vec![ContentNibble {
                    level_1: 1,
                    level_2: 1,
                    user: 0,
                }],
                ratings: vec![],
                crids: vec![],
            },
        );

        let json = serde_json::to_string(&store).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let svc_data = &v["1-1-100"];
        assert_eq!(svc_data["service_name"], "BBC One");
        assert_eq!(svc_data["events"][0]["event_name"], "The News");
        assert_eq!(
            svc_data["events"][0]["content_nibbles"][0],
            serde_json::json!({"level_1": 1, "level_2": 1, "user": 0})
        );
    }
}
