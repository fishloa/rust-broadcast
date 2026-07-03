//! RTSP session state machine — RFC 2326 Appendix A.
//!
//! Implements the client (§A.1) and server (§A.2) transition tables exactly as
//! transcribed in [`docs/state-machines.md`](../docs/state-machines.md). State
//! is tracked per session object (stream URL + session id); this module models
//! a single object's state.
//!
//! The state-neutral methods `OPTIONS`, `DESCRIBE`, `ANNOUNCE`, `GET_PARAMETER`,
//! and `SET_PARAMETER` are permitted in every state and never change it
//! (RFC 2326 Appendix A intro). The state-affecting methods are `SETUP`,
//! `PLAY`, `PAUSE`, `TEARDOWN`, `RECORD`, and `REDIRECT`.

use crate::Method;
use crate::error::{Error, Result};

/// The lifecycle state of an RTSP session object (RFC 2326 §A.1 / §A.2).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SessionState {
    /// Initial state; no successful SETUP has been completed. A client in `Init`
    /// that has sent a SETUP is still tracked as `Init` until the 2xx arrives.
    #[default]
    Init,
    /// A SETUP succeeded (or a PAUSE returned from Playing/Recording). Ready to
    /// PLAY or RECORD.
    Ready,
    /// A PLAY succeeded; media is being delivered.
    Playing,
    /// A RECORD succeeded; media is being recorded.
    Recording,
}

impl SessionState {
    /// The RFC 2326 label for this state.
    pub fn name(&self) -> &'static str {
        match self {
            SessionState::Init => "Init",
            SessionState::Ready => "Ready",
            SessionState::Playing => "Playing",
            SessionState::Recording => "Recording",
        }
    }
}

impl core::fmt::Display for SessionState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Returns `true` for the methods that do not affect session state and are
/// therefore permitted in every state (RFC 2326 Appendix A intro).
pub fn is_state_neutral(method: &Method) -> bool {
    matches!(
        method,
        Method::Options
            | Method::Describe
            | Method::Announce
            | Method::GetParameter
            | Method::SetParameter
    )
}

/// Computes the next state for the **client** after receiving a `2xx` success
/// response to `method` sent in `current` state, per the RFC 2326 §A.1 client
/// state table.
///
/// Returns `Err(MethodNotValidInState)` if the method is not listed for the
/// current state (a client MUST NOT issue such a request). State-neutral methods
/// return the unchanged current state.
// NOTE: client_next_state + server_next_state are two hand-written matches
// mirroring RFC 2326 Appendix A.1/A.2 verbatim. A new SessionState variant
// (the enum is #[non_exhaustive]) must update BOTH; the wildcard arm returns
// MethodNotValidInState, so an unhandled combination fails safe, not silently.
pub fn client_next_state(current: SessionState, method: &Method) -> Result<SessionState> {
    if is_state_neutral(method) {
        return Ok(current);
    }
    let next = match (current, method) {
        // Init
        (SessionState::Init, Method::Setup) => SessionState::Ready,
        (SessionState::Init, Method::Teardown) => SessionState::Init,
        // Ready
        (SessionState::Ready, Method::Play) => SessionState::Playing,
        (SessionState::Ready, Method::Record) => SessionState::Recording,
        (SessionState::Ready, Method::Teardown) => SessionState::Init,
        (SessionState::Ready, Method::Setup) => SessionState::Ready,
        // Playing
        (SessionState::Playing, Method::Pause) => SessionState::Ready,
        (SessionState::Playing, Method::Teardown) => SessionState::Init,
        (SessionState::Playing, Method::Play) => SessionState::Playing,
        (SessionState::Playing, Method::Setup) => SessionState::Playing, // changed transport
        // Recording
        (SessionState::Recording, Method::Pause) => SessionState::Ready,
        (SessionState::Recording, Method::Teardown) => SessionState::Init,
        (SessionState::Recording, Method::Record) => SessionState::Recording,
        (SessionState::Recording, Method::Setup) => SessionState::Recording, // changed transport
        _ => {
            return Err(Error::MethodNotValidInState {
                method: method.clone(),
                state: current,
            });
        }
    };
    Ok(next)
}

/// Computes the next state for the **server** after sending a `2xx` success
/// response to a received `method` in `current` state, per the RFC 2326 §A.2
/// server state table.
///
/// Returns `Err(MethodNotValidInState)` if the method is not listed for the
/// current state; the server maps that error to a `455` response.
/// State-neutral methods return the unchanged current state.
pub fn server_next_state(current: SessionState, method: &Method) -> Result<SessionState> {
    if is_state_neutral(method) {
        return Ok(current);
    }
    let next = match (current, method) {
        // Init
        (SessionState::Init, Method::Setup) => SessionState::Ready,
        (SessionState::Init, Method::Teardown) => SessionState::Init,
        // Ready
        (SessionState::Ready, Method::Play) => SessionState::Playing,
        (SessionState::Ready, Method::Setup) => SessionState::Ready,
        (SessionState::Ready, Method::Teardown) => SessionState::Init,
        (SessionState::Ready, Method::Record) => SessionState::Recording,
        // Playing
        (SessionState::Playing, Method::Play) => SessionState::Playing,
        (SessionState::Playing, Method::Pause) => SessionState::Ready,
        (SessionState::Playing, Method::Teardown) => SessionState::Init,
        (SessionState::Playing, Method::Setup) => SessionState::Playing,
        // Recording
        (SessionState::Recording, Method::Record) => SessionState::Recording,
        (SessionState::Recording, Method::Pause) => SessionState::Ready,
        (SessionState::Recording, Method::Teardown) => SessionState::Init,
        (SessionState::Recording, Method::Setup) => SessionState::Recording,
        _ => {
            return Err(Error::MethodNotValidInState {
                method: method.clone(),
                state: current,
            });
        }
    };
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_setup_from_init_is_ready() {
        assert_eq!(
            client_next_state(SessionState::Init, &Method::Setup).unwrap(),
            SessionState::Ready
        );
    }

    #[test]
    fn client_play_from_init_bites() {
        assert!(client_next_state(SessionState::Init, &Method::Play).is_err());
    }

    #[test]
    fn client_pause_from_ready_bites() {
        assert!(client_next_state(SessionState::Ready, &Method::Pause).is_err());
    }

    #[test]
    fn client_teardown_any_to_init() {
        for s in [
            SessionState::Ready,
            SessionState::Playing,
            SessionState::Recording,
        ] {
            assert_eq!(
                client_next_state(s, &Method::Teardown).unwrap(),
                SessionState::Init
            );
        }
    }

    #[test]
    fn state_neutral_methods_never_change_state() {
        for m in [
            Method::Options,
            Method::Describe,
            Method::Announce,
            Method::GetParameter,
            Method::SetParameter,
        ] {
            for s in [
                SessionState::Init,
                SessionState::Ready,
                SessionState::Playing,
            ] {
                assert_eq!(client_next_state(s, &m).unwrap(), s);
                assert_eq!(server_next_state(s, &m).unwrap(), s);
            }
        }
    }

    #[test]
    fn server_play_from_init_bites() {
        assert!(server_next_state(SessionState::Init, &Method::Play).is_err());
    }
}
