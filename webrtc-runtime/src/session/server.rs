//! Generic WHIP/WHEP server state machine for a single session.

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

use headers::HeaderMapExt;
use http::{HeaderMap, StatusCode};

use super::{Profile, Whep, Whip, content_type};
use crate::Error;
use crate::http_util::{self, Precondition};

pub use crate::http_util::HttpResponse;

/// State of a server session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// Awaiting POST with SDP offer.
    AwaitingOffer,
    /// WHEP only: counter-offer sent (406), awaiting the player's SDP answer
    /// via PATCH.
    CounterOffered {
        /// The resource's `ETag`, if one was already assigned.
        etag: Option<String>,
    },
    /// Session established — media flowing.
    Established {
        /// The resource's current `ETag`.
        etag: String,
    },
    /// Session terminated.
    Closed,
}

/// Events emitted by a server to the caller.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// SDP offer received — generate an answer.
    SdpOffer(Vec<u8>),
    /// WHEP only: the player's SDP answer to our counter-offer.
    SdpAnswer(Vec<u8>),
    /// Trickle ICE candidates received from the client.
    TrickleIce {
        /// SDP fragment body carrying the client's new ICE candidates.
        sdp_fragment: Vec<u8>,
        /// The `If-Match` `ETag` the client supplied, if any.
        if_match: Option<String>,
    },
    /// ICE restart requested by the client.
    IceRestart {
        /// SDP fragment body carrying the restarted ICE credentials/candidates.
        sdp_fragment: Vec<u8>,
    },
    /// Client terminated the session.
    Terminated,
}

/// Sans-IO server state machine for a single session, generic over the
/// direction [`Profile`]; use the [`WhipSession`] and [`WhepSession`] aliases.
pub struct Session<P: Profile> {
    session_url: String,
    state: State,
    profile: PhantomData<P>,
}

/// Sans-IO WHIP server session (media server endpoint).
pub type WhipSession = Session<Whip>;

/// Sans-IO WHEP server session (media server / origin endpoint).
pub type WhepSession = Session<Whep>;

impl<P: Profile> core::fmt::Debug for Session<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct(P::SERVER_NAME)
            .field("session_url", &self.session_url)
            .field("state", &self.state)
            .finish()
    }
}

impl<P: Profile> Session<P> {
    /// Create a new server session that will answer at `session_url`.
    pub fn new(session_url: String) -> Self {
        Self {
            session_url,
            state: State::AwaitingOffer,
            profile: PhantomData,
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
            .with_content_type(content_type::SDP)
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

    /// Process an incoming PATCH (trickle ICE or ICE restart; for WHEP also the
    /// player's answer to a counter-offer).
    ///
    /// The body's media type comes from the typed `Content-Type` in `headers`
    /// (parameters such as `charset` are ignored, RFC 9110 §8.3.1), and
    /// `If-Match` is read as a typed RFC 9110 entity-tag list: `*` is an ICE
    /// restart, the session's current strong tag is a trickle update. An absent
    /// header is `428` for WHIP (RFC 9725 §4.3.1: this session always generates
    /// an ETag, and §4.3.2 makes entity-tags mandatory for Trickle ICE) and a
    /// plain trickle update for WHEP.
    ///
    /// # Errors
    /// [`Error::InvalidSdpFragment`] when `Content-Type` is not
    /// `application/trickle-ice-sdpfrag`;
    /// [`Error::InvalidHeader`] for a malformed `If-Match`;
    /// [`Error::PreconditionRequired`] when `If-Match` is absent (WHIP only);
    /// [`Error::ETagMismatch`] if it does not strongly match the session's tag;
    /// [`Error::WrongState`] outside [`State::Established`] (and, for WHEP,
    /// [`State::CounterOffered`]).
    pub fn on_patch(&mut self, body: Vec<u8>, headers: &HeaderMap) -> Result<Event, Error> {
        match &self.state {
            State::CounterOffered { .. }
                if http_util::content_type_is(headers, content_type::SDP) =>
            {
                Ok(Event::SdpAnswer(body))
            }
            State::Established { etag } => {
                if !http_util::content_type_is(headers, content_type::TRICKLE_ICE) {
                    return Err(Error::InvalidSdpFragment {
                        reason: alloc::format!(
                            "unexpected content-type: {}",
                            http_util::content_type_text(headers)
                        ),
                    });
                }
                match http_util::check_if_match(headers, etag)? {
                    Precondition::Any => Ok(Event::IceRestart { sdp_fragment: body }),
                    Precondition::Passes => Ok(Event::TrickleIce {
                        sdp_fragment: body,
                        if_match: Some(etag.clone()),
                    }),
                    Precondition::Absent if P::EGRESS => Ok(Event::TrickleIce {
                        sdp_fragment: body,
                        if_match: None,
                    }),
                    Precondition::Absent => Err(Error::PreconditionRequired { operation: "PATCH" }),
                }
            }
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
            .with_content_type(content_type::TRICKLE_ICE)
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

    /// Build the 200 OK response for DELETE.
    pub fn ack_delete(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::OK)
    }
}

impl Session<Whep> {
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
            Some(ref date) => {
                alloc::format!("{}; valid-until=\"{date}\"", content_type::SDP)
            }
            None => content_type::SDP.into(),
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

    /// Build 204 response after accepting player's SDP answer to counter-offer.
    pub fn ack_answer(&mut self, etag: String) -> HttpResponse {
        self.state = State::Established { etag };
        HttpResponse::new(StatusCode::NO_CONTENT)
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
}

fn server_state_name(s: &State) -> &'static str {
    match s {
        State::AwaitingOffer => "awaiting-offer",
        State::CounterOffered { .. } => "counter-offered",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
