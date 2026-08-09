//! Prometheus metric-name constants this crate records through the
//! [`metrics`] facade (`metrics::counter!`/`gauge!`), exactly the plumbing
//! `multimux::prometheus` already establishes for the rest of the workspace
//! (issue #663 P1c) — see that module's docs for why the facade (rather than
//! a hand-rolled text renderer, the shape `media_doctor::WatchState` uses) is
//! the right integration point for a *library*: this crate never installs a
//! recorder itself, so the exact same process-wide
//! `metrics_exporter_prometheus::PrometheusHandle` a host already installed
//! (multimux's, or a standalone binary's own) picks up every metric below
//! for free the moment this crate's [`crate::Probe`] methods run.
//!
//! # What is measured, and what is deliberately not
//!
//! Every metric here is backed by a real, currently-implemented check.
//! **`compliance_probe_tr101290_events_total{indicator="PCR_accuracy_error"}`
//! will never appear** — [`dvb_conformance::Indicator::PcrAccuracyError`]
//! (TR 101 290 §5.2 Table 5.0b, 2.4) exists in the upstream crate only for
//! documentation completeness and is never emitted (it would need ±500 ns
//! hardware arrival timing this sans-IO monitor cannot honestly provide —
//! see `dvb-conformance`'s own README). The T-STD buffer-model sub-checks
//! `dvb-conformance` itself documents as partial (`Buffer_error` TBn
//! overflow, `Empty_buffer_error` MBn, `Data_delay_error`'s still-picture
//! 60 s threshold) are exactly as partial here — this crate adds no
//! buffer-model logic of its own. A dashboard panel for one of these must
//! not be read as "checked and passing": it is checked to the extent the
//! upstream crate documents, no further.
//!
//! `compliance_probe_pcr_drift_ppm`/`compliance_probe_pcr_jitter_ppm` are
//! **not** the spec's `PCR_accuracy_error` (2.4) under a different name: they
//! are a software-clock estimate (signalled PCR interval vs. the caller's
//! own arrival [`core::time::Duration`]), explicitly not resolved to the
//! ±500 ns a hardware reference would give. See [`crate::conformance`] for
//! the exact arithmetic and why a discontinuity is suppressed rather than
//! reported as an implausible drift spike.
//!
//! SCTE-35 malformed-section detection
//! (`compliance_probe_scte35_malformed_total`) can only fire on the **wire**
//! path ([`crate::Probe::feed_scte35_section`]) — anything already published
//! into a [`media_plane::Trunk`]'s event log as a
//! [`timed_metadata::TimedEvent`] was, by construction, already parsed
//! successfully by whoever wrote it there, so
//! [`crate::Probe::drain_event_cursor`] structurally cannot observe
//! malformedness. This is stated once here rather than silently producing a
//! metric that is always zero on that path.

/// Counter: TS packets fed to [`crate::Probe::feed_ts_packet`] — the total
/// volume the conformance monitor and PCR tracker have actually analysed.
pub const TS_PACKETS_TOTAL: &str = "compliance_probe_ts_packets_total";

/// Counter: ETSI TR 101 290 indicator events raised, labelled `indicator`
/// (`dvb_conformance::Indicator::name()`) and `priority`
/// (`dvb_conformance::Priority::name()`). See the module docs for why
/// `indicator="PCR_accuracy_error"` never appears.
pub const TR101290_EVENTS_TOTAL: &str = "compliance_probe_tr101290_events_total";

/// Gauge: 1.0 while [`dvb_conformance::ConformanceMonitor`] currently
/// considers the fed packet stream in sync (TR 101 290 §5.2 indicator 1.1
/// hysteresis), else 0.0.
pub const TR101290_IN_SYNC: &str = "compliance_probe_tr101290_in_sync";

/// Gauge: smoothed (EWMA) PCR interval error, in parts-per-million, labelled
/// `pid` (`"0x{pid:04X}"`). See [`crate::conformance`] for the exact
/// arithmetic and its documented honesty limits.
pub const PCR_DRIFT_PPM: &str = "compliance_probe_pcr_drift_ppm";

/// Gauge: instantaneous deviation of the latest PCR interval error from the
/// smoothed drift above, in parts-per-million — a live jitter signal,
/// labelled `pid`.
pub const PCR_JITTER_PPM: &str = "compliance_probe_pcr_jitter_ppm";

/// Counter: [`media_plane::ByteTap`] items lost to producer-side eviction
/// before this probe could poll them
/// (`media_plane::byte_tap::TapItem::Lagged`). A non-zero rate here means the
/// TR 101 290 continuity/PCR indicators for the lost stretch are gapped, not
/// wrong — see `media-plane`'s own `byte_tap` module docs.
pub const TAP_LAGGED_TOTAL: &str = "compliance_probe_tap_lagged_total";

/// Counter: [`media_plane::trunk::EventCursor`] entries lost to ordinary
/// event-log eviction before this probe could poll them
/// (`media_plane::trunk::EventCursorItem::Lagged`).
pub const EVENT_CURSOR_LAGGED_TOTAL: &str = "compliance_probe_event_cursor_lagged_total";

/// Counter: SCTE-35 cues observed, labelled `kind`
/// (`timed_metadata::EventKind::name()` on the Trunk-cursor path, or
/// `"break_start"`/`"break_end"`/`"other"` on the wire path — see
/// [`crate::scte35`]).
pub const SCTE35_CUES_TOTAL: &str = "compliance_probe_scte35_cues_total";

/// Counter: `splice_info_section` bytes that failed to parse
/// (`scte35_splice::SpliceInfoSection::parse` returned `Err`, or the section
/// was `encrypted_packet` with no accessible clear view). Wire path only —
/// see the module docs.
pub const SCTE35_MALFORMED_TOTAL: &str = "compliance_probe_scte35_malformed_total";

/// Counter: a `splice_insert` with `time_specified_flag == 0` (an immediate
/// splice, ANSI/SCTE 35 §9.7.3.1) — genuinely has no `pts_time` to judge
/// future-vs-past against, so it is counted separately rather than folded
/// into either sanity outcome.
pub const SCTE35_IMMEDIATE_TOTAL: &str = "compliance_probe_scte35_immediate_total";

/// Counter: a SCTE-35 cue whose target `pts_time` had already elapsed
/// relative to the probe's reference clock at the moment it was checked
/// (ANSI/SCTE 35 §9.7.3.1 `splice_time.pts_time`) — a cue arriving too late
/// to act on.
pub const SCTE35_PTS_IN_PAST_TOTAL: &str = "compliance_probe_scte35_pts_in_past_total";

/// Counter (Trunk-cursor path only): a SCTE-35-sourced
/// [`timed_metadata::TimedEvent`] whose [`media_plane::EventAnchor`] is still
/// `Segment`/`Utc` — not yet resolved to a media time. Per the B1 discipline
/// documented on `media_plane::trunk`, this crate never fabricates a
/// resolution to judge future-vs-past; it counts the deferral instead.
pub const SCTE35_UNRESOLVED_ANCHOR_TOTAL: &str = "compliance_probe_scte35_unresolved_anchor_total";

/// Counter (Trunk-cursor path only): a `Media`-resolved SCTE-35 cue arrived
/// before [`crate::Probe::note_media_time`] had ever been called, so there is
/// no reference "now" to compare it against. Distinct from
/// `SCTE35_UNRESOLVED_ANCHOR_TOTAL` (that one is the event's own anchor being
/// unresolved; this one is the probe having no playhead reference yet).
pub const SCTE35_NO_REFERENCE_TOTAL: &str = "compliance_probe_scte35_no_reference_total";

/// Counter: `media_doctor::Diagnostic` findings from
/// [`crate::structural::run_structural_checks`], labelled `rule_id`
/// (`media_doctor::Finding::rule_id`) and `severity`
/// (`media_doctor::Severity::name()`).
pub const STRUCTURAL_FINDINGS_TOTAL: &str = "compliance_probe_structural_findings_total";
