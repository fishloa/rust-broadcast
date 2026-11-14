//! MPEG-DASH pull ingest source (issue #758): a remote MPD and its segments pulled by a
//! relay-mode [`broadcast_client::Session`] through the shared session host
//! (`crate::source::client_host`).
//!
//! The session owns DASH: it parses the MPD, resolves addressing for every Period and selected
//! AdaptationSet, runs the UTC clock ladder (`UTCTiming`, the host clock, the response `Date`),
//! refreshes live MPDs (conditionally), joins near the live edge, gates each request on the
//! segment's availability, retries and classifies failures and selects the tracks. The host
//! performs its HTTP requests and anchors the session's wall clock to the host clock (re-read
//! every `utc_resync_us`); the sample timeline is stitched across Period boundaries
//! (`EventIngest::stitching`) because a `Trunk` feeds segmenters that expect time to move
//! forward.
//!
//! Per Period the session plays one video AdaptationSet (the highest-bandwidth Representation
//! its policy allows) and one audio AdaptationSet, on a single timeline.
//!
//! # Cross-origin URLs
//!
//! A server-side pull must not fetch whatever address a remote manifest names
//! (`http://169.254.169.254/...` as a `BaseURL`). By default every URL derived from the MPD
//! (`BaseURL`, segments, `UTCTiming`) must have the origin of the MPD itself, after redirects; a
//! Representation that leaves it is dropped. A route opts out with
//! [`DashPullRoute::with_cross_origin_urls`] (`allow_cross_origin` in the config file), for the
//! usual case of an MPD served from an origin and segments from a CDN.

use std::convert::Infallible;
use std::sync::Arc;

use broadcast_client::{Config, SourceKind};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;

use crate::clock::HostClock;
use crate::error::{MultimuxError, Result};
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep};
use crate::source::pull_target::{PullKind, PullRoute};
#[cfg(any(test, feature = "test-seams"))]
use broadcast_common::units::MICROS_PER_SEC;

#[cfg(test)]
mod tests;

/// Consecutive failures of one segment before a static presentation fails the attempt and a live
/// one skips the segment.
const MAX_SEGMENT_FAILURES: u32 = 4;
/// Consecutive failures of one init or index resource before the attempt ends.
const MAX_INIT_FAILURES: u32 = 4;
/// Segment requests kept outstanding by the session, a share of
/// [`crate::source::MAX_INFLIGHT_FETCHES`] that leaves room for the MPD, the inits and the
/// `UTCTiming` requests.
const SEGMENT_WINDOW: usize = crate::source::MAX_INFLIGHT_FETCHES / 2;
const WHAT: &str = "dash-pull";

/// A remote MPEG-DASH presentation to pull: its MPD URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
pub type DashPullRoute = PullRoute<DashPullKind>;

/// The [`PullKind`] of a [`DashPullRoute`].
#[derive(Debug, Clone, Copy)]
pub struct DashPullKind;

impl PullKind for DashPullKind {
    const ROUTE: &'static str = "DashPullRoute";
    type Extra = bool;

    fn debug_extra(extra: &bool) -> Vec<(&'static str, &dyn std::fmt::Debug)> {
        vec![("allow_cross_origin", extra)]
    }
}

impl crate::source::pull_target::PullDrive for DashPullKind {
    const SEGMENTED: bool = true;

    async fn drive(
        route: std::sync::Arc<crate::source::pull_target::PullRoute<Self>>,
        trunk_config: media_plane::trunk::TrunkConfig,
        handshake: media_plane::ingress::HandshakePolicy,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_dash_pull(&route, trunk_config, handshake, &route_handle, cancel).await
    }
}

impl DashPullRoute {
    /// Lets the MPD name `BaseURL`, segment and `UTCTiming` URLs on another
    /// origin than its own. Off by default; see the module documentation.
    #[must_use]
    pub fn with_cross_origin_urls(mut self, allow: bool) -> Self {
        self.extra = allow;
        self
    }

    pub(crate) fn allow_cross_origin(&self) -> bool {
        self.extra
    }
}

/// Parse an `MPD@availabilityStartTime`-style `xs:dateTime` into Unix seconds
/// through `dash-runtime`'s reader (§5.3.1.2). `None` for any shape its
/// accepted subset rejects. A lexical `Z` guard keeps offset forms and a
/// missing zone out: the reader would convert an offset, this field is UTC.
#[cfg(any(test, feature = "test-seams"))]
fn parse_iso8601_utc(s: &str) -> Option<u64> {
    if !s.trim().ends_with('Z') {
        return None;
    }
    let parsed = dash_runtime::timing::datetime::parse_xs_datetime(s).ok()?;
    u64::try_from(parsed.unix_us.div_euclid(MICROS_PER_SEC as i64)).ok()
}

/// `#[doc(hidden)]` test seam over [`parse_iso8601_utc`].
#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn parse_iso8601_utc_for_test(s: &str) -> Option<u64> {
    parse_iso8601_utc(s)
}

fn session_config(route: &DashPullRoute) -> Config {
    let mut config = Config::relay();
    config.kind = SourceKind::Dash;
    let mut config = config.with_timeouts(route.target.timeouts);
    config.max_queued_bytes = crate::source::SEGMENTED_EVENT_QUEUE_BYTES;
    config.dash.cross_origin_urls = route.allow_cross_origin();
    config.utc_timing_credentials = false;
    config.max_segment_bytes = crate::source::MAX_HTTP_BODY_BYTES;
    config.max_inflight_segments = SEGMENT_WINDOW;
    config.max_segment_failures = MAX_SEGMENT_FAILURES;
    config.max_key_failures = MAX_INIT_FAILURES;
    config
}

/// Drives `route` until a static MPD has played to its end or the session fails outright; a
/// dynamic MPD never ends on its own.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 — see
/// `crate::source::rtsp::run_rtsp`'s own doc for what `crate::source::report_driver_progress`
/// does with it each iteration.
pub async fn run_dash_pull(
    route: &DashPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    run_with_clock(
        route,
        trunk_config,
        handshake,
        route_handle,
        cancel,
        Arc::new(crate::clock::SystemHostClock::new()),
    )
    .await
}

/// [`run_dash_pull`] with the instants taken from `clock`: the one seam a test uses to give the
/// pull a host clock with a known skew.
pub(crate) async fn run_with_clock(
    route: &DashPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    clock: Arc<dyn HostClock>,
) -> Result<()> {
    let timeouts = route.target.timeouts;
    let config = session_config(route);
    let resync_us = config.dash.utc_resync_us;
    let step = SessionStep::open(
        &route.target.url,
        config,
        route.target.auth.clone(),
        SessionLabels::new(WHAT, timeouts).session_retries(),
        Arc::clone(route_handle),
        false,
    )?;
    let mut step = step.with_clock(clock).with_clock_resync(resync_us);
    let driver = IngestDriver::new(
        EventIngest::new().stitching(),
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut sink = tokio::io::sink();
    let err = crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        cancel,
        timeouts,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await;
    let result = match err {
        MultimuxError::Connect { reason } if reason.starts_with("session ended") => Ok(()),
        other => Err(other),
    };
    crate::source::finish_vod(route_handle, result)
}
