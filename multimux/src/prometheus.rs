//! Process-wide Prometheus metrics plumbing (issue #663, P1c).
//!
//! multimux records metrics throughout the crate via the `metrics` facade's
//! macros (`metrics::counter!`/`gauge!`/`histogram!`):
//!
//! - `multimux_route_up` (`ROUTE_UP`) — gauge, labels `route`: 1.0 while
//!   that route's [`crate::store::HealthState`] is `Live`, else 0.0. Set in
//!   `origin::supervisor::supervise` alongside every `MediaStore::set_health`
//!   call (the supervisor is the one place that both knows the route name and
//!   drives health transitions).
//! - `multimux_source_reconnects_total` (`SOURCE_RECONNECTS_TOTAL`) —
//!   counter, labels `route`: bumped once each time a route's supervisor loop
//!   re-enters `Reconnecting` (a lost connection or ended pipeline about to be
//!   retried).
//! - `multimux_segments_produced_total` / `multimux_parts_produced_total`
//!   (`SEGMENTS_PRODUCED_TOTAL` / `PARTS_PRODUCED_TOTAL`) — counters,
//!   labels `route`: bumped in `pipeline::run_pipeline` every time a
//!   completed segment/part is published into the route's `MediaStore`.
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

/// Counter: incremented once per full segment the pipeline publishes into a
/// route's store. Labels: `route`.
pub(crate) const SEGMENTS_PRODUCED_TOTAL: &str = "multimux_segments_produced_total";

/// Counter: incremented once per part the pipeline publishes into a route's
/// store. Labels: `route`.
pub(crate) const PARTS_PRODUCED_TOTAL: &str = "multimux_parts_produced_total";

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
