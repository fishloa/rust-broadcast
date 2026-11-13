//! MPEG-DASH pull ingest source (issue #758): fetch a remote MPD and its
//! segments, demux the fMP4/CMAF bytes and publish the samples into a `Trunk`.
//!
//! # A thin host over `dash-runtime`
//!
//! Everything DASH is the sans-IO core's:
//! [`dash_runtime::client::DashClient`] parses the MPD, resolves addressing
//! for every Period and every selected AdaptationSet, runs the UTC clock
//! ladder (`UTCTiming`, the host clock, the response `Date`), refreshes live
//! MPDs, joins near the live edge, gates each request on the segment's
//! availability, classifies failures and selects the tracks. This module owns
//! only what is multimux's:
//!
//! - `session`: [`DashIngestSession`], the media-plane [`media_plane::ingress::IngestSession`]
//!   over the core. It maps outputs onto `SessionEvent`s (one program, tracks
//!   announced and re-announced, samples), decides what ends a connection
//!   attempt (the route supervisor reconnects), and hands the drive loop the
//!   retries the core asked for. No clock, no sleeping.
//! - `stitch`: one continuous sample timeline across Period boundaries.
//! - `drive`: the tokio loop (reqwest, timeouts, byte ranges, conditional MPD
//!   refreshes, backoff, the host clock anchor).
//!
//! Time comes from one seam, `HostClock`, which the loop reads
//! and hands to the core as arguments; nothing in this module reads a clock.
//!
//! # What a route serves
//!
//! Per Period the core plays one video AdaptationSet (the highest-bandwidth
//! Representation its policy allows) and one audio AdaptationSet, on a single
//! timeline. Every Period of the MPD plays, in order. SegmentTemplate
//! (`$Number$`, `$Time$`), SegmentTimeline, SegmentList and SegmentBase with a
//! `sidx` are addressed. The behaviour changes against the pre-core module are
//! listed in `docs/dash-runtime/DESIGN.md` section 12.3 and the multimux
//! changelog.
//!
//! # Cross-origin URLs
//!
//! A server-side pull must not fetch whatever address a remote manifest names
//! (`http://169.254.169.254/...` as a `BaseURL`). By default every URL derived
//! from the MPD (`BaseURL`, segments, `UTCTiming`) must have the origin of the
//! MPD itself, after redirects; a Representation that leaves it is dropped.
//! A route opts out with [`DashPullRoute::with_cross_origin_urls`]
//! (`allow_cross_origin` in the config file), for the usual case of an MPD
//! served from an origin and segments from a CDN.

use std::sync::Arc;

use media_plane::ingress::{Dialer, HandshakePolicy};
use media_plane::trunk::TrunkConfig;
use reqwest::Client as HttpClient;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    credentials_from_url, resolve_credentials, scope_credentials, strip_userinfo,
};
use crate::source::pull_target::{PullKind, PullRoute};
use broadcast_auth::AuthState;

mod drive;
mod session;
mod stitch;
#[cfg(test)]
mod tests;

pub use session::{DashIngestSession, DashInput, FetchTarget, PROGRAM, dash_config, pull_limits};

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
        _cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_dash_pull(&route, trunk_config, handshake, &route_handle).await
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

/// Constructs a [`DashIngestSession`] — performs **no I/O**.
pub struct DashPullDialer {
    mpd_url: Url,
    dash: dash_runtime::client::DashConfig,
    limits: dash_runtime::client::PullLimits,
}

impl DashPullDialer {
    fn new(
        mpd_url: Url,
        dash: dash_runtime::client::DashConfig,
        limits: dash_runtime::client::PullLimits,
    ) -> Self {
        Self {
            mpd_url,
            dash,
            limits,
        }
    }
}

impl Dialer for DashPullDialer {
    type Session = DashIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> Result<DashIngestSession> {
        Ok(DashIngestSession::with_config(
            self.mpd_url.clone(),
            self.dash,
            self.limits.clone(),
        ))
    }
}

/// Parse an `MPD@availabilityStartTime`-style `xs:dateTime` into Unix seconds
/// through `dash-runtime`'s reader (§5.3.1.2). `None` for any shape its
/// accepted subset rejects. A lexical `Z` guard keeps offset forms and a
/// missing zone out: the reader would convert an offset, this field is UTC.
fn parse_iso8601_utc(s: &str) -> Option<u64> {
    if !s.trim().ends_with('Z') {
        return None;
    }
    let parsed = dash_runtime::timing::datetime::parse_xs_datetime(s).ok()?;
    u64::try_from(parsed.unix_us.div_euclid(MICROS_PER_SECOND)).ok()
}

const MICROS_PER_SECOND: i64 = 1_000_000;

/// `#[doc(hidden)]` test seam over [`parse_iso8601_utc`].
#[doc(hidden)]
pub fn parse_iso8601_utc_for_test(s: &str) -> Option<u64> {
    parse_iso8601_utc(s)
}

fn build_client(route: &DashPullRoute) -> Result<(HttpClient, Url, AuthState)> {
    let parsed = Url::parse(&route.target.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad DASH-pull URL {}: {e}",
            crate::redact::redact_url(&route.target.url)
        ),
    })?;
    let credentials =
        resolve_credentials(route.target.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let credentials = scope_credentials(credentials, &clean_url);
    let http = HttpClient::builder()
        .redirect(crate::source::redirect_policy(route.allow_cross_origin()))
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

/// Publishes into a route's registry; the production [`drive::RouteSink`].
struct RegistrySink<'a> {
    route_handle: &'a Arc<crate::route::RouteHandle>,
    progress: crate::source::DriverProgress,
}

impl drive::RouteSink for RegistrySink<'_> {
    fn advance<'a>(
        &'a mut self,
        driver: &'a media_plane::ingress::IngestDriver<DashIngestSession>,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        crate::source::advance_route(driver, self.route_handle, &mut self.progress)
    }

    fn finish(&mut self, result: Result<()>) -> Result<()> {
        crate::source::finish_vod(self.route_handle, result)
    }
}

/// Drives `route` to completion: dial (no I/O), then pump the core's fetches,
/// feed the answers back and publish the samples until a static MPD has played
/// to its end, or a fetch the failure policy cannot absorb fails. A dynamic MPD
/// never ends on its own.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_dash_pull(
    route: &DashPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<crate::route::RouteHandle>,
) -> Result<()> {
    drive::run(
        route,
        trunk_config,
        handshake,
        Arc::new(crate::clock::SystemHostClock::new()),
        RegistrySink {
            route_handle,
            progress: crate::source::DriverProgress::new(),
        },
    )
    .await
}
