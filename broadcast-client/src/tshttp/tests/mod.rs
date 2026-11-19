use core::ops::{Deref, DerefMut};

use crate::tshttp::config::Config;
use crate::tshttp::{client, policy};
use broadcast_common::stage::Timestamp;

mod audit_1485;
mod audit_1492;
mod edge_cases;
mod event_serde;
mod flush;
mod policy_core;
mod policy_tests;
mod real_stream;
mod retry_after;
mod timers;
mod transitions;
mod vod;
mod vod_real;

fn status_of(code: u16) -> http::StatusCode {
    http::StatusCode::from_u16(code).expect("a valid test status code")
}

fn header_map(pairs: &[(String, String)]) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            http::HeaderName::from_bytes(name.as_bytes()).expect("a valid test header name"),
            http::HeaderValue::from_str(value).expect("a valid test header value"),
        );
    }
    map
}

/// The pre-#2685 scripted calls: a bare status code and header pairs, which the
/// policy has since taken as typed `http` values. Tests assert through the real
/// types; only the scripted call shape is adapted.
pub(crate) struct TsHttpClient(client::TsHttpClient);

impl TsHttpClient {
    pub(crate) fn with_seed(
        url: impl Into<String>,
        config: Config,
        now: Timestamp,
        seed: u64,
    ) -> Self {
        Self(client::TsHttpClient::with_seed(url, config, now, seed))
    }

    pub(crate) fn on_response(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        now: Timestamp,
    ) {
        self.0
            .on_response(status_of(status), &header_map(headers), now);
    }
}

impl Deref for TsHttpClient {
    type Target = client::TsHttpClient;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TsHttpClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) struct TsHttpPolicy(policy::TsHttpPolicy);

impl TsHttpPolicy {
    pub(crate) fn new(url: impl Into<String>, config: Config, now: Timestamp) -> Self {
        Self(policy::TsHttpPolicy::new(url, config, now))
    }

    pub(crate) fn on_response(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        now: Timestamp,
    ) {
        self.0
            .on_response(status_of(status), &header_map(headers), now);
    }

    pub(crate) fn poll_timeout(&mut self, now: Timestamp) {
        self.0.poll_timeout_with(now, |_| false);
    }
}

impl Deref for TsHttpPolicy {
    type Target = policy::TsHttpPolicy;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TsHttpPolicy {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
