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

use broadcast_auth::Credentials;
use media_plane::ingress::{Dialer, HandshakePolicy};
use media_plane::trunk::TrunkConfig;
use reqwest::Client as HttpClient;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    ScopedCredentials, credentials_from_url, resolve_credentials, scope_credentials, strip_userinfo,
};
use crate::source::{IngestTimeouts, Source};

#[cfg(test)]
mod differential;
mod drive;
#[cfg(test)]
mod legacy_plan;
mod session;
mod stitch;
#[cfg(test)]
mod tests;

pub use session::{DashIngestSession, DashInput, FetchTarget, PROGRAM, dash_config, pull_limits};

/// A remote MPEG-DASH presentation to pull: its MPD URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct DashPullRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo
    /// — see `crate::source::http_auth::resolve_credentials`.
    auth: Option<Credentials>,
    /// Whether MPD-derived URLs may leave the MPD's origin.
    allow_cross_origin: bool,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for DashPullRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashPullRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .field("allow_cross_origin", &self.allow_cross_origin)
            .finish()
    }
}

impl DashPullRoute {
    /// Build a route descriptor. `url` is the MPD URL to pull.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        DashPullRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
            allow_cross_origin: false,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }

    /// Lets the MPD name `BaseURL`, segment and `UTCTiming` URLs on another
    /// origin than its own. Off by default; see the module documentation.
    #[must_use]
    pub fn with_cross_origin_urls(mut self, allow: bool) -> Self {
        self.allow_cross_origin = allow;
        self
    }
}

impl Source for DashPullRoute {
    fn stream_name(&self) -> &str {
        &self.name
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

/// Parse an ISO-8601 UTC timestamp (`YYYY-MM-DDThh:mm:ss[.fff]Z`, the form
/// `MPD@availabilityStartTime` uses, §5.3.1.2) into Unix seconds. `None` for
/// any other shape.
///
/// Parsed by `jiff::Timestamp` (SP5); a lexical guard requires the UTC `Z`
/// suffix, so an offset form (`+01:00`) is rejected rather than silently
/// converted. Whole-second and fractional forms both round-trip.
fn parse_iso8601_utc(s: &str) -> Option<u64> {
    let s = s.trim();
    if !s.ends_with('Z') {
        return None;
    }
    let ts: jiff::Timestamp = s.parse().ok()?;
    u64::try_from(ts.as_second()).ok()
}

/// `#[doc(hidden)]` test seam over [`parse_iso8601_utc`].
#[doc(hidden)]
pub fn parse_iso8601_utc_for_test(s: &str) -> Option<u64> {
    parse_iso8601_utc(s)
}

fn build_client(route: &DashPullRoute) -> Result<(HttpClient, Url, Option<ScopedCredentials>)> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad DASH-pull URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let credentials = scope_credentials(credentials, &clean_url);
    let http = HttpClient::builder()
        .redirect(crate::source::redirect_policy(route.allow_cross_origin))
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
