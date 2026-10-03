//! WHEP server (media server / origin endpoint) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use headers::HeaderMapExt;
use http::{HeaderMap, StatusCode};

use crate::Error;
use crate::http_util::{self, Precondition};

/// State of a WHEP server session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// Awaiting POST with SDP offer.
    AwaitingOffer,
    /// Counter-offer sent (406), awaiting player's SDP answer via PATCH.
    CounterOffered {
        /// The resource's `ETag`, if one was already assigned.
        etag: Option<String>,
    },
    /// Session established — sending media to player.
    Established {
        /// The resource's current `ETag`.
        etag: String,
    },
    /// Session terminated.
    Closed,
}

pub use crate::http_util::HttpResponse;

/// Events emitted by the WHEP server.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// SDP offer received from player.
    SdpOffer(Vec<u8>),
    /// Player's SDP answer to our counter-offer.
    SdpAnswer(Vec<u8>),
    /// Trickle ICE candidates from player.
    TrickleIce {
        /// SDP fragment body carrying the player's new ICE candidates.
        sdp_fragment: Vec<u8>,
        /// The `If-Match` `ETag` the player supplied, if any.
        if_match: Option<String>,
    },
    /// ICE restart requested by player.
    IceRestart {
        /// SDP fragment body carrying the restarted ICE credentials/candidates.
        sdp_fragment: Vec<u8>,
    },
    /// Player terminated the session.
    Terminated,
}

/// Sans-IO WHEP server state machine for a single session.
#[derive(Debug)]
pub struct WhepSession {
    session_url: String,
    state: State,
}

impl WhepSession {
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

    /// Process an incoming POST (player's SDP offer).
    pub fn on_post(&mut self, sdp_offer: Vec<u8>) -> Result<Event, Error> {
        if self.state != State::AwaitingOffer {
            return Err(Error::WrongState {
                operation: "POST",
                state: server_state_name(&self.state),
            });
        }
        Ok(Event::SdpOffer(sdp_offer))
    }

    /// Build the 201 Created response (direct accept).
    ///
    /// A session URL or `etag` that cannot be sent as a header yields a `500`
    /// response with no `Location` and leaves the session's state unchanged.
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

    /// Build the 406 Not Acceptable counter-offer response.
    ///
    /// A `valid_until` that cannot be sent in a `Content-Type` parameter yields
    /// a `500` response and leaves the session's state unchanged.
    pub fn counter_offer(
        &mut self,
        sdp_offer: Vec<u8>,
        valid_until: Option<String>,
    ) -> HttpResponse {
        let ct = match valid_until {
            Some(ref date) => alloc::format!("application/sdp; valid-until=\"{date}\""),
            None => "application/sdp".into(),
        };
        let built = HttpResponse::new(StatusCode::NOT_ACCEPTABLE)
            .with_location(&self.session_url)
            .and_then(|r| r.with_content_type(&ct));
        match built {
            Ok(resp) => {
                self.state = State::CounterOffered { etag: None };
                resp.with_body(sdp_offer)
            }
            Err(_) => HttpResponse::new(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    /// Process an incoming PATCH.
    ///
    /// The body's media type comes from the typed `Content-Type` in `headers`
    /// (parameters such as `charset` are ignored, RFC 9110 §8.3.1; a duplicated
    /// header is rejected), and `If-Match` is read as a typed RFC 9110
    /// entity-tag list (see [`crate::whip::server::WhipSession::on_patch`]).
    ///
    /// # Errors
    /// [`Error::InvalidSdpFragment`] for an unexpected content type,
    /// [`Error::InvalidHeader`] / [`Error::ETagMismatch`] for a bad `If-Match`,
    /// [`Error::WrongState`] otherwise.
    pub fn on_patch(&mut self, sdp_body: Vec<u8>, headers: &HeaderMap) -> Result<Event, Error> {
        match &self.state {
            State::CounterOffered { .. }
                if http_util::content_type_is(headers, super::content_type::SDP) =>
            {
                Ok(Event::SdpAnswer(sdp_body))
            }
            State::Established { etag } => {
                if http_util::content_type_is(headers, super::content_type::TRICKLE_ICE) {
                    match http_util::check_if_match(headers, etag)? {
                        Precondition::Any => Ok(Event::IceRestart {
                            sdp_fragment: sdp_body,
                        }),
                        Precondition::Passes => Ok(Event::TrickleIce {
                            sdp_fragment: sdp_body,
                            if_match: Some(etag.clone()),
                        }),
                        Precondition::Absent => Ok(Event::TrickleIce {
                            sdp_fragment: sdp_body,
                            if_match: None,
                        }),
                    }
                } else {
                    Err(Error::InvalidSdpFragment {
                        reason: alloc::format!(
                            "unexpected content-type: {}",
                            http_util::content_type_text(headers)
                        ),
                    })
                }
            }
            _ => Err(Error::WrongState {
                operation: "PATCH",
                state: server_state_name(&self.state),
            }),
        }
    }

    /// Build 204 response after accepting player's SDP answer to counter-offer.
    pub fn ack_answer(&mut self, etag: String) -> HttpResponse {
        self.state = State::Established { etag };
        HttpResponse::new(StatusCode::NO_CONTENT)
    }

    /// Build 204 response for trickle ICE.
    pub fn ack_trickle(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::NO_CONTENT)
    }

    /// Build 200 OK response for ICE restart.
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

    /// Build 409 Conflict response (no active publisher), with a typed
    /// `Retry-After` (whole seconds) when `retry_after` is given.
    pub fn no_publisher(retry_after: Option<std::time::Duration>) -> HttpResponse {
        let mut resp = HttpResponse::new(StatusCode::CONFLICT);
        if let Some(delay) = retry_after {
            resp.headers.typed_insert(http_util::retry_after(delay));
        }
        resp
    }

    /// Process an incoming DELETE.
    pub fn on_delete(&mut self) -> Result<Event, Error> {
        match &self.state {
            State::Established { .. } | State::CounterOffered { .. } => {
                self.state = State::Closed;
                Ok(Event::Terminated)
            }
            _ => Err(Error::WrongState {
                operation: "DELETE",
                state: server_state_name(&self.state),
            }),
        }
    }

    /// Build 200 OK response for DELETE.
    pub fn ack_delete(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::OK)
    }
}

fn server_state_name(s: &State) -> &'static str {
    match s {
        State::AwaitingOffer => "awaiting-offer",
        State::CounterOffered { .. } => "counter-offered",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
