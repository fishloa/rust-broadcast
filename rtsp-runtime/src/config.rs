//! What each RTSP role is configured by (RFC 2326 §A): one config per role,
//! each built with `Default::default` plus `with_*` setters and handed to that
//! role's [`ClientSession::new`](crate::ClientSession::new) or
//! [`ServerSession::new`](crate::ServerSession::new).

#[cfg(feature = "tokio")]
use crate::client::ClientSession;
use crate::error::{Error, Result};
use crate::session_id::SessionId;

/// The `User-Agent` a client announces when its config names none.
pub const DEFAULT_USER_AGENT: &str = "rtsp-runtime";

/// The `CSeq` a client's first request carries when its config names none.
pub const FIRST_CSEQ: u32 = 1;

/// What a `ClientSession` is configured by: the
/// `User-Agent` it announces and the `CSeq` its first request carries.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RtspClientConfig {
    /// The session's `User-Agent` (RFC 2326 §12.40).
    pub user_agent: String,
    /// The `CSeq` of the first request; every later request advances it
    /// (RFC 2326 §C.1.1).
    pub initial_cseq: u32,
}

impl RtspClientConfig {
    /// Replace [`RtspClientConfig::user_agent`].
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Replace [`RtspClientConfig::initial_cseq`].
    #[must_use]
    pub fn with_initial_cseq(mut self, cseq: u32) -> Self {
        self.initial_cseq = cseq;
        self
    }
}

impl Default for RtspClientConfig {
    fn default() -> Self {
        RtspClientConfig {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            initial_cseq: FIRST_CSEQ,
        }
    }
}

/// Where a [`ServerSession`](crate::ServerSession) draws its
/// [`SessionId`]s from (RFC 2326 §3.4).
///
/// The id is the only thing tying later requests to a session, so its source
/// must be cryptographically secure: an unpredictable id is what stops
/// another client taking a session over. [`SessionIdSource::OsRandom`] is the
/// default; [`SessionIdSource::Counter`] is a deterministic test source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SessionIdSource {
    /// Each id is drawn from the OS cryptographically secure random source.
    /// A failing read surfaces on the `SETUP` that needed the id as
    /// [`Error::RandomSource`] and no session is created. The default.
    #[default]
    OsRandom,
    /// Each id is the seed plus the number of ids already allocated. For
    /// deterministic tests only: the ids are predictable, so a server that
    /// accepts real connections must never use it.
    Counter(u64),
}

impl SessionIdSource {
    /// Hands out `seed`, `seed + 1`, `seed + 2`, … — one per call.
    #[must_use]
    pub fn counter(seed: u64) -> Self {
        SessionIdSource::Counter(seed)
    }

    /// The id a session `allocated` ids in draws next: the seed plus
    /// `allocated` for a counter, the next OS random draw otherwise.
    ///
    /// # Errors
    ///
    /// [`Error::RandomSource`] when [`SessionIdSource::OsRandom`]'s read of
    /// the OS random source fails.
    pub(crate) fn next_id(&self, allocated: u64) -> Result<SessionId> {
        match self {
            SessionIdSource::Counter(seed) => Ok(SessionId::new(seed.wrapping_add(allocated))),
            SessionIdSource::OsRandom => os_random_id(),
        }
    }
}

/// Reads one id from the OS cryptographically secure random source.
fn os_random_id() -> Result<SessionId> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| Error::RandomSource)?;
    Ok(SessionId::new(u64::from_ne_bytes(bytes)))
}

/// What an [`AsyncRtspClient`](crate::io::AsyncRtspClient) is configured by:
/// the session it drives and the [`Timeouts`](broadcast_common::timeouts::Timeouts)
/// on every awaited I/O.
///
/// The `tokio` adapter's config, not the sans-IO
/// [`RtspClientConfig`]: it adds the IO bounds to the protocol session.
#[cfg(feature = "tokio")]
#[derive(Debug)]
#[non_exhaustive]
pub struct RtspClientIoConfig {
    /// The protocol session, e.g. one carrying
    /// [`Credentials`](crate::Credentials).
    pub session: ClientSession,
    /// Bounds on every awaited I/O; `connect` bounds the TCP connect and
    /// `handshake` the TLS handshake.
    pub timeouts: broadcast_common::timeouts::Timeouts,
}

#[cfg(feature = "tokio")]
impl RtspClientIoConfig {
    /// Replace [`RtspClientIoConfig::session`] (a fresh `ClientSession` by
    /// default).
    #[must_use]
    pub fn with_session(mut self, session: ClientSession) -> Self {
        self.session = session;
        self
    }

    /// Replace [`RtspClientIoConfig::timeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: broadcast_common::timeouts::Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

#[cfg(feature = "tokio")]
impl Default for RtspClientIoConfig {
    /// A fresh [`ClientSession`] and the default `Timeouts`.
    fn default() -> Self {
        RtspClientIoConfig {
            session: ClientSession::default(),
            timeouts: broadcast_common::timeouts::Timeouts::default(),
        }
    }
}
