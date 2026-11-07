//! The target every pull route shares: a name for logging/metrics, the source
//! URL (which may carry `user:pass@` userinfo), the ingest timeouts and the
//! config-supplied credentials (which take precedence over URL userinfo, see
//! `crate::source::http_auth::resolve_credentials`).

use broadcast_auth::Credentials;

use crate::source::IngestTimeouts;

#[derive(Clone)]
pub(crate) struct PullTarget {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) timeouts: IngestTimeouts,
    pub(crate) auth: Option<Credentials>,
}

impl PullTarget {
    pub(crate) fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    pub(crate) fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
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
