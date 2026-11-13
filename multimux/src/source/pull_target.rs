//! The target every pull route shares: a name for logging/metrics, the source
//! URL (which may carry `user:pass@` userinfo), the ingest timeouts and the
//! config-supplied credentials (which take precedence over URL userinfo).

use std::future::Future;
use std::sync::Arc;

use broadcast_auth::Credentials;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::TrunkConfig;
use tokio_util::sync::CancellationToken;

use crate::route::RouteHandle;
use broadcast_common::timeouts::Timeouts;

#[derive(Clone)]
pub(crate) struct PullTarget {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) timeouts: Timeouts,
    pub(crate) auth: Option<Credentials>,
}

impl PullTarget {
    pub(crate) fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            timeouts: Timeouts::default(),
            auth: None,
        }
    }

    pub(crate) fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub(crate) fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }

    /// `Debug` body of a route: `url` may carry a live origin's `user:pass@`
    /// userinfo and `auth` a raw password/token, so neither renders verbatim.
    pub(crate) fn debug(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        route: &str,
        extra: &[(&str, &dyn std::fmt::Debug)],
    ) -> std::fmt::Result {
        let mut d = f.debug_struct(route);
        d.field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"));
        for (name, value) in extra {
            d.field(name, value);
        }
        d.finish()
    }
}

/// Names a kind of pull route and carries its route-specific option.
pub trait PullKind {
    /// The route's name in `Debug` output.
    const ROUTE: &'static str;
    /// A route-specific option set by a builder method on the concrete route.
    type Extra: Clone + Default;
    /// `Debug` fields contributed by [`PullKind::Extra`].
    fn debug_extra(_extra: &Self::Extra) -> Vec<(&'static str, &dyn std::fmt::Debug)> {
        Vec::new()
    }
}

/// A pull route: a name, source URL, timeouts and credentials for one [`PullKind`]. [`RtspRoute`],
/// [`TsHttpRoute`], [`HlsPullRoute`], [`SmoothPullRoute`] and
/// [`DashPullRoute`] are this type under their kind.
///
/// [`RtspRoute`]: crate::source::rtsp::RtspRoute
/// [`TsHttpRoute`]: crate::source::ts_http::TsHttpRoute
/// [`HlsPullRoute`]: crate::source::hls_pull::HlsPullRoute
/// [`SmoothPullRoute`]: crate::source::smooth_pull::SmoothPullRoute
/// [`DashPullRoute`]: crate::source::dash_pull::DashPullRoute
pub struct PullRoute<K: PullKind> {
    pub(crate) target: PullTarget,
    pub(crate) extra: K::Extra,
}

impl<K: PullKind> Clone for PullRoute<K> {
    fn clone(&self) -> Self {
        Self {
            target: self.target.clone(),
            extra: self.extra.clone(),
        }
    }
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl<K: PullKind> std::fmt::Debug for PullRoute<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.target.debug(f, K::ROUTE, &K::debug_extra(&self.extra))
    }
}

impl<K: PullKind> PullRoute<K> {
    /// Build a route descriptor for the source at `url`.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            target: PullTarget::new(name, url),
            extra: K::Extra::default(),
        }
    }

    /// Overrides the default [`Timeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.target = self.target.with_timeouts(timeouts);
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.target = self.target.with_auth(auth);
        self
    }
}

impl<K: PullKind> crate::source::Source for PullRoute<K> {
    fn stream_name(&self) -> &str {
        &self.target.name
    }
}

/// How the supervisor drives one [`PullKind`]: the `Trunk` shape its ingest
/// needs and the one async drive loop. [`spawn_pull`](crate::origin) is
/// generic over this, so the five pull routes share a single spawn path.
pub(crate) trait PullDrive: PullKind + Sized + 'static {
    /// `true` for a segment-pulling kind, whose `Trunk` needs a timed ring
    /// deep enough for one fetched segment.
    const SEGMENTED: bool;

    /// Runs one ingest attempt; resolves only when the attempt ends.
    fn drive(
        route: Arc<PullRoute<Self>>,
        trunk_config: TrunkConfig,
        handshake: HandshakePolicy,
        route_handle: Arc<RouteHandle>,
        cancel: CancellationToken,
    ) -> impl Future<Output = crate::Result<()>> + Send;
}
