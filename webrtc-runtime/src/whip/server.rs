//! WHIP server (media server endpoint) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use http::{HeaderMap, StatusCode};

use crate::Error;
use crate::http_util::{self, Precondition};

/// State of a WHIP server session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// Awaiting POST with SDP offer.
    AwaitingOffer,
    /// Session established — client connected.
    Established {
        /// The resource's current `ETag`.
        etag: String,
    },
    /// Session terminated.
    Closed,
}

pub use crate::http_util::HttpResponse;

/// Events emitted by the WHIP server to the caller.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// SDP offer received — generate an answer.
    SdpOffer(Vec<u8>),
    /// Trickle ICE candidates received from client.
    TrickleIce {
        /// SDP fragment body carrying the client's new ICE candidates.
        sdp_fragment: Vec<u8>,
        /// The `If-Match` `ETag` the client supplied, if any.
        if_match: Option<String>,
    },
    /// ICE restart requested by client.
    IceRestart {
        /// SDP fragment body carrying the restarted ICE credentials/candidates.
        sdp_fragment: Vec<u8>,
    },
    /// Client terminated the session.
    Terminated,
}

/// Sans-IO WHIP server state machine for a single session.
#[derive(Debug)]
pub struct WhipSession {
    /// Resource URL returned in the `Location` header of the 201 response.
    session_url: String,
    /// Current server-side session state.
    state: State,
}

impl WhipSession {
    /// Create a new server session that will answer at `session_url`.
    pub fn new(session_url: String) -> Self {
        Self {
            session_url,
            state: State::AwaitingOffer,
        }
    }

    /// The session's current state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The resource URL this session answers at.
    pub fn session_url(&self) -> &str {
        &self.session_url
    }

    /// Process an incoming POST (SDP offer).
    pub fn on_post(&mut self, sdp_offer: Vec<u8>) -> Result<Event, Error> {
        if self.state != State::AwaitingOffer {
            return Err(Error::WrongState {
                operation: "POST",
                state: server_state_name(&self.state),
            });
        }
        Ok(Event::SdpOffer(sdp_offer))
    }

    /// Build the 201 Created response after generating an SDP answer.
    ///
    /// A session URL or `etag` that cannot be sent as a header (non-ASCII, a
    /// double quote in the tag) yields a `500 Internal Server Error` response
    /// with no `Location`, and leaves the session in its current state.
    pub fn accept(&mut self, sdp_answer: Vec<u8>, etag: String) -> HttpResponse {
        let built = HttpResponse::new(StatusCode::CREATED)
            .with_content_type(super::content_type::SDP)
            .and_then(|r| r.with_location(&self.session_url))
            .and_then(|r| r.with_etag(&etag));
        match built {
            Ok(resp) => {
                self.state = State::Established { etag };
                resp.with_body(sdp_answer)
            }
            Err(_) => HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// Process an incoming PATCH (trickle ICE or ICE restart).
    ///
    /// `If-Match` is read as a typed RFC 9110 entity-tag list: `*` is an ICE
    /// restart, the session's current strong tag is a trickle update, and an
    /// absent header is a plain trickle update.
    ///
    /// # Errors
    /// [`Error::InvalidHeader`] for a malformed `If-Match`;
    /// [`Error::ETagMismatch`] if it does not strongly match the session's tag;
    /// [`Error::WrongState`] outside [`State::Established`].
    pub fn on_patch(&mut self, sdp_fragment: Vec<u8>, headers: &HeaderMap) -> Result<Event, Error> {
        match &self.state {
            State::Established { etag } => match http_util::check_if_match(headers, etag)? {
                Precondition::Any => Ok(Event::IceRestart { sdp_fragment }),
                Precondition::Passes => Ok(Event::TrickleIce {
                    sdp_fragment,
                    if_match: Some(etag.clone()),
                }),
                Precondition::Absent => Ok(Event::TrickleIce {
                    sdp_fragment,
                    if_match: None,
                }),
            },
            _ => Err(Error::WrongState {
                operation: "PATCH",
                state: server_state_name(&self.state),
            }),
        }
    }

    /// Build the 204 No Content response for trickle ICE.
    pub fn ack_trickle(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::NO_CONTENT)
    }

    /// Build the 200 OK response for ICE restart.
    ///
    /// A `new_etag` that cannot be sent as a header yields a `500` response and
    /// leaves the session's tag unchanged.
    pub fn ack_restart(&mut self, sdp_fragment: Vec<u8>, new_etag: String) -> HttpResponse {
        let built = HttpResponse::new(StatusCode::OK)
            .with_content_type(super::content_type::TRICKLE_ICE)
            .and_then(|r| r.with_etag(&new_etag));
        match built {
            Ok(resp) => {
                self.state = State::Established { etag: new_etag };
                resp.with_body(sdp_fragment)
            }
            Err(_) => HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// Process an incoming DELETE.
    pub fn on_delete(&mut self) -> Result<Event, Error> {
        match &self.state {
            State::Established { .. } => {
                self.state = State::Closed;
                Ok(Event::Terminated)
            }
            _ => Err(Error::WrongState {
                operation: "DELETE",
                state: server_state_name(&self.state),
            }),
        }
    }

    /// Build the 200 OK response for DELETE.
    pub fn ack_delete(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::OK)
    }
}

fn server_state_name(s: &State) -> &'static str {
    match s {
        State::AwaitingOffer => "awaiting-offer",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
