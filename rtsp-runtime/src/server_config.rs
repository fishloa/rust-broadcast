//! What a [`ServerSession`](crate::ServerSession) is configured by: its
//! `Session` id source, the `Server` header it answers with, and the options
//! its replies carry (RFC 2326 §12.37, §12.40).

use broadcast_common::timeouts::Timeouts;

use crate::config::SessionIdSource;

/// The `Server` header a server session answers with when its config names
/// none.
pub const DEFAULT_SERVER_HEADER: &str = "rtsp-runtime";

/// What a [`ServerSession`](crate::ServerSession) is configured by.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RtspServerConfig {
    /// Where the session ids come from (RFC 2326 §3.4).
    pub session_ids: SessionIdSource,
    /// The session's `Server` header (RFC 2326 §12.40).
    pub server_header: String,
    /// The SDP body a `DESCRIBE` is answered with (`Content-Type:
    /// application/sdp`); `None` answers with a bare `200 OK`.
    pub describe_sdp: Option<Vec<u8>>,
    /// The RTP/RTCP port pair a UDP `SETUP` reply advertises as
    /// `server_port` (RFC 2326 §12.39).
    pub server_ports: Option<(u16, u16)>,
    /// The lifetime advertised in the `Session` header on `SETUP`
    /// (RFC 2326 §12.37).
    pub session_timeout: Option<core::time::Duration>,
    /// Bounds on every awaited I/O of a session driven by the `tokio`
    /// adapter; the sans-IO engine reads none of them.
    pub timeouts: Timeouts,
}

impl RtspServerConfig {
    /// Replace [`RtspServerConfig::session_ids`].
    #[must_use]
    pub fn with_session_ids(mut self, session_ids: SessionIdSource) -> Self {
        self.session_ids = session_ids;
        self
    }

    /// Replace [`RtspServerConfig::server_header`].
    #[must_use]
    pub fn with_server_header(mut self, server_header: impl Into<String>) -> Self {
        self.server_header = server_header.into();
        self
    }

    /// Answer `DESCRIBE` with `sdp`.
    #[must_use]
    pub fn with_describe_sdp(mut self, sdp: impl Into<Vec<u8>>) -> Self {
        self.describe_sdp = Some(sdp.into());
        self
    }

    /// Advertise `rtp`/`rtcp` as the `server_port` pair of a UDP `SETUP`
    /// (RFC 2326 §12.39).
    #[must_use]
    pub fn with_server_ports(mut self, rtp: u16, rtcp: u16) -> Self {
        self.server_ports = Some((rtp, rtcp));
        self
    }

    /// Advertise `timeout` in the `Session` header (RFC 2326 §12.37).
    #[must_use]
    pub fn with_session_timeout(mut self, timeout: core::time::Duration) -> Self {
        self.session_timeout = Some(timeout);
        self
    }

    /// Replace [`RtspServerConfig::timeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

impl Default for RtspServerConfig {
    fn default() -> Self {
        RtspServerConfig {
            session_ids: SessionIdSource::default(),
            server_header: DEFAULT_SERVER_HEADER.to_string(),
            describe_sdp: None,
            server_ports: None,
            session_timeout: None,
            timeouts: Timeouts::default(),
        }
    }
}
