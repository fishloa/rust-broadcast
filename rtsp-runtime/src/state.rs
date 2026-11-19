//! RTSP session state machine — RFC 2326 Appendix A.
//!
//! Implements the client (§A.1) and server (§A.2) transition tables exactly as
//! transcribed in `private/specs-verified/rtp/rtsp-runtime/state-machines.md`. State
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
pub(crate) fn is_state_neutral(method: &Method) -> bool {
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
pub(crate) fn client_next_state(current: SessionState, method: &Method) -> Result<SessionState> {
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
pub(crate) fn server_next_state(current: SessionState, method: &Method) -> Result<SessionState> {
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
        assert!(matches!(
            client_next_state(SessionState::Init, &Method::Play),
            Err(crate::error::Error::MethodNotValidInState { .. })
        ));
    }

    #[test]
    fn client_pause_from_ready_bites() {
        assert!(matches!(
            client_next_state(SessionState::Ready, &Method::Pause),
            Err(crate::error::Error::MethodNotValidInState { .. })
        ));
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
        assert!(matches!(
            server_next_state(SessionState::Init, &Method::Play),
            Err(crate::error::Error::MethodNotValidInState { .. })
        ));
    }

    /// RFC 2326 §A.1/§A.2: the next state for every state-affecting method in
    /// every state. `None` is a combination the tables do not list.
    const A1_A2: [(SessionState, Method, Option<SessionState>); 20] = [
        (SessionState::Init, Method::Setup, Some(SessionState::Ready)),
        (SessionState::Init, Method::Play, None),
        (SessionState::Init, Method::Pause, None),
        (SessionState::Init, Method::Record, None),
        (SessionState::Init, Method::Teardown, Some(SessionState::Init)),
        (SessionState::Ready, Method::Setup, Some(SessionState::Ready)),
        (SessionState::Ready, Method::Play, Some(SessionState::Playing)),
        (SessionState::Ready, Method::Pause, None),
        (
            SessionState::Ready,
            Method::Record,
            Some(SessionState::Recording),
        ),
        (SessionState::Ready, Method::Teardown, Some(SessionState::Init)),
        (
            SessionState::Playing,
            Method::Setup,
            Some(SessionState::Playing),
        ),
        (
            SessionState::Playing,
            Method::Play,
            Some(SessionState::Playing),
        ),
        (
            SessionState::Playing,
            Method::Pause,
            Some(SessionState::Ready),
        ),
        (SessionState::Playing, Method::Record, None),
        (
            SessionState::Playing,
            Method::Teardown,
            Some(SessionState::Init),
        ),
        (
            SessionState::Recording,
            Method::Setup,
            Some(SessionState::Recording),
        ),
        (SessionState::Recording, Method::Play, None),
        (
            SessionState::Recording,
            Method::Pause,
            Some(SessionState::Ready),
        ),
        (
            SessionState::Recording,
            Method::Record,
            Some(SessionState::Recording),
        ),
        (
            SessionState::Recording,
            Method::Teardown,
            Some(SessionState::Init),
        ),
    ];

    #[test]
    fn the_client_state_table_is_covered_transition_by_transition() {
        for (state, method, expected) in &A1_A2 {
            match expected {
                Some(next) => assert_eq!(
                    client_next_state(*state, method).expect("a listed transition"),
                    *next,
                    "client {state} + {method:?}"
                ),
                None => assert!(
                    matches!(
                        client_next_state(*state, method),
                        Err(crate::error::Error::MethodNotValidInState { .. })
                    ),
                    "client {state} + {method:?} must be refused"
                ),
            }
        }
    }

    #[test]
    fn the_server_state_table_is_covered_transition_by_transition() {
        for (state, method, expected) in &A1_A2 {
            match expected {
                Some(next) => assert_eq!(
                    server_next_state(*state, method).expect("a listed transition"),
                    *next,
                    "server {state} + {method:?}"
                ),
                None => assert!(
                    matches!(
                        server_next_state(*state, method),
                        Err(crate::error::Error::MethodNotValidInState { .. })
                    ),
                    "server {state} + {method:?} must be refused"
                ),
            }
        }
    }

    #[test]
    fn every_state_names_and_displays_itself() {
        for (state, name) in [
            (SessionState::Init, "Init"),
            (SessionState::Ready, "Ready"),
            (SessionState::Playing, "Playing"),
            (SessionState::Recording, "Recording"),
        ] {
            assert_eq!(state.name(), name);
            assert_eq!(state.to_string(), name);
        }
    }
}
