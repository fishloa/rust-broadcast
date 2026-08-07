//! WHIP server (media server endpoint) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

/// State of a WHIP server session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Awaiting POST with SDP offer.
    AwaitingOffer,
    /// Session established — client connected.
    Established { etag: String },
    /// Session terminated.
    Closed,
}

/// An HTTP response the caller must send on behalf of the state machine.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<&'static str>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Events emitted by the WHIP server to the caller.
#[derive(Debug, Clone)]
pub enum Event {
    /// SDP offer received — generate an answer.
    SdpOffer(Vec<u8>),
    /// Trickle ICE candidates received from client.
    TrickleIce {
        sdp_fragment: Vec<u8>,
        if_match: Option<String>,
    },
    /// ICE restart requested by client.
    IceRestart { sdp_fragment: Vec<u8> },
    /// Client terminated the session.
    Terminated,
}

/// Sans-IO WHIP server state machine for a single session.
#[derive(Debug)]
pub struct WhipSession {
    session_url: String,
    state: State,
}

impl WhipSession {
    pub fn new(session_url: String) -> Self {
        Self {
            session_url,
            state: State::AwaitingOffer,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

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
    pub fn accept(
        &mut self,
        sdp_answer: Vec<u8>,
        etag: String,
    ) -> HttpResponse {
        self.state = State::Established { etag: etag.clone() };
        HttpResponse {
            status: super::status::CREATED,
            content_type: Some(super::content_type::SDP),
            headers: alloc::vec![
                ("Location".into(), self.session_url.clone()),
                ("ETag".into(), alloc::format!("\"{etag}\"")),
            ],
            body: sdp_answer,
        }
    }

    /// Process an incoming PATCH (trickle ICE or ICE restart).
    pub fn on_patch(
        &mut self,
        sdp_fragment: Vec<u8>,
        if_match: Option<&str>,
    ) -> Result<Event, Error> {
        match &self.state {
            State::Established { etag } => {
                if if_match == Some("*") {
                    Ok(Event::IceRestart { sdp_fragment })
                } else if let Some(client_etag) = if_match {
                    let client_etag = client_etag.trim_matches('"');
                    if client_etag != etag {
                        return Err(Error::ETagMismatch {
                            expected: etag.clone(),
                            got: client_etag.into(),
                        });
                    }
                    Ok(Event::TrickleIce {
                        sdp_fragment,
                        if_match: Some(client_etag.into()),
                    })
                } else {
                    Ok(Event::TrickleIce {
                        sdp_fragment,
                        if_match: None,
                    })
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
        HttpResponse {
            status: super::status::NO_CONTENT,
            content_type: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Build the 200 OK response for ICE restart.
    pub fn ack_restart(
        &mut self,
        sdp_fragment: Vec<u8>,
        new_etag: String,
    ) -> HttpResponse {
        self.state = State::Established {
            etag: new_etag.clone(),
        };
        HttpResponse {
            status: 200,
            content_type: Some(super::content_type::TRICKLE_ICE),
            headers: alloc::vec![
                ("ETag".into(), alloc::format!("\"{new_etag}\"")),
            ],
            body: sdp_fragment,
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
        HttpResponse {
            status: 200,
            content_type: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

fn server_state_name(s: &State) -> &'static str {
    match s {
        State::AwaitingOffer => "awaiting-offer",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
