//! Process-wide Prometheus metrics plumbing (issue #663, P1c).
//!
//! multimux records metrics throughout the crate via the `metrics` facade's
//! macros (`metrics::counter!`/`gauge!`/`histogram!`):
//!
//! - `multimux_route_up` (`ROUTE_UP`) — gauge, labels `route`: 1.0 while
//!   that route's [`crate::route::HealthState`] is `Live`, else 0.0. Set in
//!   `origin::supervisor::supervise_driver` alongside every
//!   `RouteHandle::set_health` call (the supervisor is the one place that
//!   both knows the route name and drives health transitions).
//! - `multimux_source_reconnects_total` (`SOURCE_RECONNECTS_TOTAL`) —
//!   counter, labels `route`: bumped once each time a route's supervisor loop
//!   re-enters `Reconnecting` (a lost connection or ended attempt about to be
//!   retried).
//! - `multimux_active_blocking_requests` (`ACTIVE_BLOCKING_REQUESTS`) —
//!   gauge (no route label — the LL-HLS output's blocking-wait helpers don't
//!   currently know their own route name; see `output::llhls`): count of
//!   LL-HLS blocking-reload/preload-hint requests (RFC 8216bis §6.2.5.2,
//!   §6.2.2) currently parked awaiting new data.
//! - `multimux_http_requests_total` / `multimux_http_request_duration_seconds`
//!   / `multimux_bytes_served_total` (`HTTP_REQUESTS_TOTAL` /
//!   `HTTP_REQUEST_DURATION_SECONDS` / `BYTES_SERVED_TOTAL`) — labels
//!   `route`, `path` (and `status` for the requests counter): recorded by
//!   `origin`'s HTTP middleware for every request the origin serves, root
//!   endpoints (`/metrics`, `/healthz`, `/readyz`) included.
//! - `multimux_parts_produced_total` / `multimux_segments_produced_total`
//!   (`PARTS_PRODUCED_TOTAL` / `SEGMENTS_PRODUCED_TOTAL`) — counters, labels
//!   `route`: bumped in `crate::source::segment::drive_program_segmenters`,
//!   the one place in the driver-backed architecture that actually turns raw
//!   samples into parts/segments, labelled by `RouteHandle::name()` (issue
//!   #809 — these two counters had no emitter at all since the media-plane
//!   port; see that issue and this crate's CHANGELOG for the history: they
//!   silently read zero for a while, which is worse than being entirely
//!   absent, before being deleted outright pending this fix).
//! - `multimux_dvr_pin_rearmed_total` (`DVR_PIN_REARMED_TOTAL`) — counter,
//!   labels `route`: bumped by `crate::dvr::DvrRecorder::handle_terminated`
//!   each time an `ArchiveOverrun::StallIngest` pin is force-expired by the
//!   non-blocking safety valve and the recorder re-arms rather than
//!   stopping for good (only a `Terminate`-policy pin's own intended stop
//!   does that).
//!
//! Cardinality is bounded on purpose: `route` is either a configured stream
//! name or the fixed token `"unknown"`, and `path` is one of a small fixed
//! set of kinds (`playlist`/`segment`/`part`/`init`/`metrics`/`health`/
//! `other`) — never a raw URI.
//!
//! [`install`] wires a single process-wide `metrics-exporter-prometheus`
//! recorder into the `metrics` facade's global recorder slot and hands back a
//! [`PrometheusHandle`] that renders the current snapshot as Prometheus text
//! exposition (served at `GET /metrics` by `origin::router`).

use std::sync::OnceLock;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Gauge: 1.0 while the labeled route's ingest health is `Live`, else 0.0.
/// Labels: `route`.
pub(crate) const ROUTE_UP: &str = "multimux_route_up";

/// Counter: incremented once each time a route's supervisor loop re-enters
/// `Reconnecting`. Labels: `route`.
pub(crate) const SOURCE_RECONNECTS_TOTAL: &str = "multimux_source_reconnects_total";

/// Gauge: count of LL-HLS blocking requests (media-playlist blocking reload,
/// or a preload-hinted part fetch) currently parked awaiting new data,
/// process-wide.
pub(crate) const ACTIVE_BLOCKING_REQUESTS: &str = "multimux_active_blocking_requests";

/// Counter: total HTTP requests served. Labels: `route`, `path`, `status`.
pub(crate) const HTTP_REQUESTS_TOTAL: &str = "multimux_http_requests_total";

/// Histogram: HTTP request duration, in seconds. Labels: `route`, `path`.
pub(crate) const HTTP_REQUEST_DURATION_SECONDS: &str = "multimux_http_request_duration_seconds";

/// Counter: total response bytes served. Labels: `route`, `path`.
pub(crate) const BYTES_SERVED_TOTAL: &str = "multimux_bytes_served_total";

/// Counter: total HTTP requests shed by the global concurrency bound before
/// they reached any route. Labels: `kind` (`ordinary` | `blocking_reload`).
/// Recorded inside `crate::origin::limit` because a shed request never
/// reaches `track_http` (layered inside the bound), so without this the
/// `503`s from overload are invisible in metrics (issue #1083, B).
pub(crate) const HTTP_SHED_TOTAL: &str = "multimux_http_shed_total";

/// Counter: total LL-HLS parts published into a route's `Trunk` by
/// `crate::source::segment::drive_program_segmenters`. Labels: `route`
/// (issue #809).
pub(crate) const PARTS_PRODUCED_TOTAL: &str = "multimux_parts_produced_total";

/// Counter: total segments published into a route's `Trunk` by
/// `crate::source::segment::drive_program_segmenters`. Labels: `route`
/// (issue #809).
pub(crate) const SEGMENTS_PRODUCED_TOTAL: &str = "multimux_segments_produced_total";

/// Counter: total times a route's `DvrRecorder` re-armed its pinning
/// cursor after an `ArchiveOverrun::StallIngest` pin was force-expired by
/// the non-blocking safety valve (`ProgramSegmenter::drain_pending`'s
/// `expire_stalled_pins` call) — never a `Terminate`-policy pin's own
/// intended stop, which does not re-arm. Labels: `route`. Each increment
/// corresponds to a recording gap (see `DvrRecorder::handle_terminated`)
/// whose size in segments is logged alongside it, not carried on this
/// counter itself (would need an unbounded-cardinality label or a second
/// counter this crate has no other use for).
pub(crate) const DVR_PIN_REARMED_TOTAL: &str = "multimux_dvr_pin_rearmed_total";

/// Counter: total times a route's DVR recorder was abandoned because its
/// mutex was found poisoned (a panic left its multi-step state — offset,
/// index, period records — inconsistent, so continuing to persist could write
/// an archive whose index does not match its data). Labels: `route`. Recording
/// stops for that route; this counter is how an operator sees it (issue
/// #1083, D3).
pub(crate) const DVR_FAILED_TOTAL: &str = "multimux_dvr_failed_total";

/// Counter: total live-edge segments/fragments a pull source abandoned after
/// exhausting its bounded tolerated-`404` retries (audit W14a/W14b), so the
/// downstream end has been notified the source is no longer at the live edge.
/// Labels: `source`. A non-zero rate is the signal an operator uses to see a
/// stalled-or-restarted encoder without the route going dark silently.
pub(crate) const PULL_FRAGMENT_ABANDONED_TOTAL: &str = "multimux_pull_fragment_abandoned_total";

/// Counter: total manifest refreshes in which a live stream had no matching
/// `StreamIndex` (audit W14e) — a renamed or retemplated stream. Distinct
/// from `PULL_FRAGMENT_ABANDONED_TOTAL` (a fragment that never arrived) so an
/// operator can tell the two apart. Labels: none.
pub(crate) const PULL_STREAM_REFRESH_MISS_TOTAL: &str = "multimux_pull_stream_refresh_miss_total";

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the process-wide Prometheus recorder exactly once, returning a
/// clone of its handle on every call.
///
/// `metrics::set_global_recorder` can only succeed the *first* time it's called
/// in a process — every subsequent attempt errors. Every
/// [`crate::origin::AppState`] constructed in the same process (including many
/// independent `#[tokio::test]`s in this crate's own test binary, which all
/// share one process) calls this, so it must be idempotent: the [`OnceLock`]
/// installs the recorder on the first call and every call — first or not — gets
/// a clone of the same [`PrometheusHandle`], reading the one shared,
/// process-wide set of metrics.
///
/// Uses `build_recorder()` + `metrics::set_global_recorder` rather than
/// `PrometheusBuilder::install_recorder()`: the latter spawns a background
/// **upkeep thread** (non-daemon) that never exits, which keeps every process
/// alive and makes `cargo nextest` (one process per test) time out on *every*
/// test in this binary — including the pure-sync store tests. `build_recorder`
/// installs no thread; we don't need periodic upkeep for a scrape-rendered
/// exposition.
pub fn install() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            metrics::set_global_recorder(recorder).expect(
                "installing the process-wide Prometheus recorder must succeed the one time \
                     `OnceLock::get_or_init` actually runs the closure",
            );
            handle
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Biting test: `install()` must be callable more than once in the same
    /// process (as every `AppState::new` in this crate's test binary does)
    /// without panicking, and every call must return a handle backed by the
    /// *same* recorder — reverting to a bare (non-idempotent)
    /// `PrometheusBuilder::new().install_recorder().unwrap()` at every call
    /// site would panic on the second call in this process.
    #[test]
    fn install_is_idempotent_and_shares_one_recorder() {
        let a = install();
        let b = install();
        metrics::counter!("multimux_test_idempotent_probe").increment(1);
        // Both handles must observe the same increment, proving they read
        // the same underlying recorder rather than two independent ones.
        assert!(a.render().contains("multimux_test_idempotent_probe"));
        assert!(b.render().contains("multimux_test_idempotent_probe"));
    }
}
