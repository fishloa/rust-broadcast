//! WHEP server (media server / origin endpoint) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

/// State of a WHEP server session.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// An HTTP response the caller must send.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code to send.
    pub status: u16,
    /// `Content-Type` header value, if the response carries a body.
    pub content_type: Option<&'static str>,
    /// Additional headers to send (e.g. `Location`, `ETag`, `Retry-After`).
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Events emitted by the WHEP server.
#[derive(Debug, Clone)]
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
    pub fn accept(&mut self, sdp_answer: Vec<u8>, etag: String) -> HttpResponse {
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

    /// Build the 406 Not Acceptable counter-offer response.
    pub fn counter_offer(
        &mut self,
        sdp_offer: Vec<u8>,
        valid_until: Option<String>,
    ) -> HttpResponse {
        self.state = State::CounterOffered { etag: None };
        let ct = match valid_until {
            Some(ref date) => alloc::format!("application/sdp; valid-until=\"{date}\""),
            None => "application/sdp".into(),
        };
        HttpResponse {
            status: super::status::NOT_ACCEPTABLE,
            content_type: None,
            headers: alloc::vec![
                ("Location".into(), self.session_url.clone()),
                ("Content-Type".into(), ct),
            ],
            body: sdp_offer,
        }
    }

    /// Process an incoming PATCH.
    pub fn on_patch(
        &mut self,
        content_type: &str,
        sdp_body: Vec<u8>,
        if_match: Option<&str>,
    ) -> Result<Event, Error> {
        match &self.state {
            State::CounterOffered { .. } if content_type == super::content_type::SDP => {
                Ok(Event::SdpAnswer(sdp_body))
            }
            State::Established { etag } => {
                if content_type == super::content_type::TRICKLE_ICE {
                    if matches!(if_match, Some("*") | Some("\"*\"")) {
                        Ok(Event::IceRestart {
                            sdp_fragment: sdp_body,
                        })
                    } else if let Some(client_etag) = if_match {
                        let client_etag = client_etag.trim_matches('"');
                        if client_etag != etag {
                            return Err(Error::ETagMismatch {
                                expected: etag.clone(),
                                got: client_etag.into(),
                            });
                        }
                        Ok(Event::TrickleIce {
                            sdp_fragment: sdp_body,
                            if_match: Some(client_etag.into()),
                        })
                    } else {
                        Ok(Event::TrickleIce {
                            sdp_fragment: sdp_body,
                            if_match: None,
                        })
                    }
                } else {
                    Err(Error::InvalidSdpFragment {
                        reason: alloc::format!("unexpected content-type: {content_type}"),
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
        HttpResponse {
            status: super::status::NO_CONTENT,
            content_type: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Build 204 response for trickle ICE.
    pub fn ack_trickle(&self) -> HttpResponse {
        HttpResponse {
            status: super::status::NO_CONTENT,
            content_type: None,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Build 200 OK response for ICE restart.
    pub fn ack_restart(&mut self, sdp_fragment: Vec<u8>, new_etag: String) -> HttpResponse {
        self.state = State::Established {
            etag: new_etag.clone(),
        };
        HttpResponse {
            status: 200,
            content_type: Some(super::content_type::TRICKLE_ICE),
            headers: alloc::vec![("ETag".into(), alloc::format!("\"{new_etag}\"")),],
            body: sdp_fragment,
        }
    }

    /// Build 409 Conflict response (no active publisher).
    pub fn no_publisher(retry_after_secs: Option<u32>) -> HttpResponse {
        let mut headers = Vec::new();
        if let Some(secs) = retry_after_secs {
            headers.push(("Retry-After".into(), alloc::format!("{secs}")));
        }
        HttpResponse {
            status: super::status::CONFLICT,
            content_type: None,
            headers,
            body: Vec::new(),
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

    /// Build 200 OK response for DELETE.
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
        State::CounterOffered { .. } => "counter-offered",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}
