//! WHIP client (encoder/ingester) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use http::StatusCode;

use crate::Error;
use crate::http_util;
use crate::session_common::{Established, SessionCore};

/// State of a WHIP client session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// Initial state — ready to send SDP offer.
    Idle,
    /// SDP offer sent, awaiting 201 Created.
    OfferSent,
    /// Session established — ICE/DTLS in progress or connected.
    Established {
        /// Resource URL returned in the `Location` header of the 201 response.
        session_url: String,
        /// Current resource `ETag`, if the server supplied one.
        etag: Option<String>,
    },
    /// Session terminated.
    Closed,
}

pub use crate::http_util::{HttpRequest, HttpResponse};
pub use http::Method;

/// Events emitted by the WHIP client to the caller.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// SDP answer received — pass to WebRTC stack.
    SdpAnswer(Vec<u8>),
    /// Server's ICE candidates from restart response.
    IceRestart {
        /// SDP fragment body carrying the server's new ICE candidates.
        sdp_fragment: Vec<u8>,
        /// The resource's new `ETag` after the restart.
        new_etag: String,
    },
    /// Session terminated by server (DELETE acknowledged).
    Terminated,
}

/// Sans-IO WHIP client state machine.
///
/// Caller drives it by:
/// 1. Calling methods to get `HttpRequest`s to send
/// 2. Feeding HTTP responses back via `on_response`
/// 3. Handling emitted `Event`s
pub struct WhipClient {
    core: SessionCore,
    state: State,
}

/// Manual `Debug`: the bearer token never renders (issue #1142).
impl core::fmt::Debug for WhipClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.core.debug(f, "WhipClient", &self.state)
    }
}

impl WhipClient {
    /// Create a new client targeting `endpoint_url`, optionally authenticating
    /// every request with `bearer_token`.
    pub fn new(endpoint_url: String, bearer_token: Option<String>) -> Self {
        Self {
            core: SessionCore::new(endpoint_url, bearer_token),
            state: State::Idle,
        }
    }

    /// The client's current session state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Generate the HTTP POST request carrying the SDP offer.
    pub fn offer(&mut self, sdp_offer: Vec<u8>) -> Result<HttpRequest, Error> {
        if self.state != State::Idle {
            return Err(Error::WrongState {
                operation: "offer",
                state: state_name(&self.state),
            });
        }
        let req = self.core.post_offer(sdp_offer)?;
        self.state = State::OfferSent;
        Ok(req)
    }

    /// Generate aggregated Trickle ICE PATCH for the caller's already-aggregated
    /// candidate fragment.
    pub fn flush_candidates(&mut self, aggregated_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, etag) = self.established_fields()?;
        self.core.trickle(session_url, etag, aggregated_fragment)
    }

    /// Generate ICE restart PATCH request.
    pub fn ice_restart(&mut self, sdp_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        self.core.ice_restart(session_url, sdp_fragment)
    }

    /// Generate DELETE request to terminate the session.
    pub fn terminate(&mut self) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        self.core.delete(session_url)
    }

    /// Feed an HTTP response back into the state machine.
    pub fn on_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        match &self.state {
            State::OfferSent => self.handle_offer_response(resp),
            State::Established { .. } => self.handle_established_response(resp),
            _ => Err(Error::WrongState {
                operation: "on_response",
                state: state_name(&self.state),
            }),
        }
    }

    fn handle_offer_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        if resp.status == StatusCode::CREATED {
            let session_url = http_util::location_of(&resp.headers)
                .ok_or(Error::MissingHeader { header: "Location" })?;
            self.state = State::Established {
                session_url,
                etag: http_util::etag_of(&resp.headers),
            };
            Ok(Some(Event::SdpAnswer(resp.body)))
        } else {
            Err(Error::Http {
                status: resp.status.as_u16(),
            })
        }
    }

    /// Dispatches on which request is actually in flight (audit run-09
    /// W21), rather than guessing from status/`ETag` alone: those are
    /// ambiguous between a trickle-ICE ack, an ICE-restart answer, and a
    /// `DELETE` ack (see [`PendingRequest`]).
    fn handle_established_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        let State::Established { etag, .. } = &mut self.state else {
            return Err(Error::WrongState {
                operation: "on_response",
                state: state_name(&self.state),
            });
        };
        match self.core.classify(resp, etag)? {
            Established::Terminated => {
                self.state = State::Closed;
                Ok(Some(Event::Terminated))
            }
            Established::IceRestart {
                sdp_fragment,
                new_etag,
            } => Ok(Some(Event::IceRestart {
                sdp_fragment,
                new_etag,
            })),
            Established::Acknowledged => Ok(None),
        }
    }

    fn established_fields(&self) -> Result<(String, Option<String>), Error> {
        match &self.state {
            State::Established { session_url, etag } => Ok((session_url.clone(), etag.clone())),
            _ => Err(Error::WrongState {
                operation: "requires established session",
                state: state_name(&self.state),
            }),
        }
    }
}

fn state_name(s: &State) -> &'static str {
    match s {
        State::Idle => "idle",
        State::OfferSent => "offer-sent",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
