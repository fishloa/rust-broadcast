//! WHEP player (viewer/consumer) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

/// State of a WHEP player session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum State {
    /// Initial state — ready to send SDP offer.
    Idle,
    /// SDP offer sent, awaiting 201 or 406.
    OfferSent,
    /// Server counter-offered (406), awaiting our SDP answer via PATCH.
    CounterOffered {
        /// Resource URL to PATCH the SDP answer to.
        session_url: String,
    },
    /// Session established — receiving media.
    Established {
        /// Resource URL returned in the `Location` header of the response.
        session_url: String,
        /// Current resource `ETag`, if the server supplied one.
        etag: Option<String>,
    },
    /// Session terminated.
    Closed,
}

/// An HTTP request the caller must send.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method to use.
    pub method: Method,
    /// Absolute or resource-relative URL to send the request to.
    pub url: String,
    /// `Content-Type` header value, if the request carries a body.
    pub content_type: Option<&'static str>,
    /// Additional headers to send (e.g. `Authorization`, `If-Match`).
    pub headers: Vec<(String, String)>,
    /// Request body bytes.
    pub body: Vec<u8>,
}

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// `POST` — used to send the initial SDP offer.
    Post,
    /// `PATCH` — used for counter-offer answers, Trickle ICE, and ICE restarts.
    Patch,
    /// `DELETE` — used to terminate a session.
    Delete,
    /// `OPTIONS` — used to discover supported ICE servers.
    Options,
    /// `HEAD` — used to poll publisher availability.
    Head,
}

impl Method {
    /// The HTTP method token as it appears on the request line.
    pub fn name(&self) -> &'static str {
        match self {
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Options => "OPTIONS",
            Method::Head => "HEAD",
        }
    }
}

broadcast_common::impl_spec_display!(Method);

/// Parsed fields from an HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// `Content-Type` header value, if present.
    pub content_type: Option<String>,
    /// `Location` header value, if present.
    pub location: Option<String>,
    /// `ETag` opaque-tag value (without DQUOTE framing), if present.
    ///
    /// Callers extracting this from an HTTP response MUST strip the
    /// surrounding `"` quotes before storing the value here.
    pub etag: Option<String>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Events emitted by the WHEP player.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// SDP answer received (direct accept) — pass to WebRTC stack.
    SdpAnswer(Vec<u8>),
    /// Server counter-offered — contains server's SDP offer.
    /// Caller must generate an SDP answer and call `answer_counter_offer`.
    CounterOffer(Vec<u8>),
    /// ICE restart response.
    IceRestart {
        /// SDP fragment body carrying the server's new ICE candidates.
        sdp_fragment: Vec<u8>,
        /// The resource's new `ETag` after the restart.
        new_etag: String,
    },
    /// Session terminated.
    Terminated,
}

/// Sans-IO WHEP player state machine.
#[derive(Debug)]
pub struct WhepPlayer {
    endpoint_url: String,
    bearer_token: Option<String>,
    state: State,
}

impl WhepPlayer {
    /// Create a new player targeting `endpoint_url`, optionally authenticating
    /// every request with `bearer_token`.
    pub fn new(endpoint_url: String, bearer_token: Option<String>) -> Self {
        Self {
            endpoint_url,
            bearer_token,
            state: State::Idle,
        }
    }

    /// The player's current session state.
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
        self.state = State::OfferSent;
        Ok(self.build_request(
            Method::Post,
            self.endpoint_url.clone(),
            Some(super::content_type::SDP),
            sdp_offer,
        ))
    }

    /// After receiving a CounterOffer event, send our SDP answer via PATCH.
    pub fn answer_counter_offer(&mut self, sdp_answer: Vec<u8>) -> Result<HttpRequest, Error> {
        match &self.state {
            State::CounterOffered { session_url } => {
                let url = session_url.clone();
                Ok(self.build_request(
                    Method::Patch,
                    url,
                    Some(super::content_type::SDP),
                    sdp_answer,
                ))
            }
            _ => Err(Error::WrongState {
                operation: "answer_counter_offer",
                state: state_name(&self.state),
            }),
        }
    }

    /// Generate Trickle ICE PATCH.
    pub fn trickle_ice(&self, sdp_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, etag) = self.established_fields()?;
        let mut req = self.build_request(
            Method::Patch,
            session_url,
            Some(super::content_type::TRICKLE_ICE),
            sdp_fragment,
        );
        if let Some(etag) = etag {
            req.headers
                .push(("If-Match".into(), alloc::format!("\"{etag}\"")));
        }
        Ok(req)
    }

    /// Generate ICE restart PATCH.
    pub fn ice_restart(&self, sdp_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        let mut req = self.build_request(
            Method::Patch,
            session_url,
            Some(super::content_type::TRICKLE_ICE),
            sdp_fragment,
        );
        req.headers.push(("If-Match".into(), "*".into()));
        Ok(req)
    }

    /// Generate DELETE request.
    pub fn terminate(&mut self) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        Ok(self.build_request(Method::Delete, session_url, None, Vec::new()))
    }

    /// Feed an HTTP response back into the state machine.
    pub fn on_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        match &self.state {
            State::OfferSent => self.handle_offer_response(resp),
            State::CounterOffered { .. } => self.handle_counter_offer_response(resp),
            State::Established { .. } => self.handle_established_response(resp),
            _ => Err(Error::WrongState {
                operation: "on_response",
                state: state_name(&self.state),
            }),
        }
    }

    fn handle_offer_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        match resp.status {
            super::status::CREATED => {
                let session_url = resp
                    .location
                    .ok_or(Error::MissingHeader { header: "Location" })?;
                self.state = State::Established {
                    session_url,
                    etag: resp.etag,
                };
                Ok(Some(Event::SdpAnswer(resp.body)))
            }
            super::status::NOT_ACCEPTABLE => {
                let session_url = resp
                    .location
                    .ok_or(Error::MissingHeader { header: "Location" })?;
                self.state = State::CounterOffered { session_url };
                Ok(Some(Event::CounterOffer(resp.body)))
            }
            super::status::CONFLICT => Err(Error::NoPublisher),
            _ => Err(Error::Http {
                status: resp.status,
            }),
        }
    }

    fn handle_counter_offer_response(
        &mut self,
        resp: HttpResponse,
    ) -> Result<Option<Event>, Error> {
        if resp.status == super::status::NO_CONTENT {
            if let State::CounterOffered { session_url } = &self.state {
                let url = session_url.clone();
                self.state = State::Established {
                    session_url: url,
                    etag: resp.etag,
                };
            }
            Ok(None)
        } else {
            Err(Error::Http {
                status: resp.status,
            })
        }
    }

    fn handle_established_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        match resp.status {
            super::status::NO_CONTENT => Ok(None),
            200 => {
                if let Some(new_etag) = resp.etag {
                    if let State::Established { etag, .. } = &mut self.state {
                        *etag = Some(new_etag.clone());
                    }
                    Ok(Some(Event::IceRestart {
                        sdp_fragment: resp.body,
                        new_etag,
                    }))
                } else {
                    self.state = State::Closed;
                    Ok(Some(Event::Terminated))
                }
            }
            _ => Err(Error::Http {
                status: resp.status,
            }),
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

    fn build_request(
        &self,
        method: Method,
        url: String,
        content_type: Option<&'static str>,
        body: Vec<u8>,
    ) -> HttpRequest {
        let mut headers = Vec::new();
        if let Some(token) = &self.bearer_token {
            headers.push(("Authorization".into(), alloc::format!("Bearer {token}")));
        }
        HttpRequest {
            method,
            url,
            content_type,
            headers,
            body,
        }
    }
}

fn state_name(s: &State) -> &'static str {
    match s {
        State::Idle => "idle",
        State::OfferSent => "offer-sent",
        State::CounterOffered { .. } => "counter-offered",
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_display_matches_http_request_line_token() {
        assert_eq!(Method::Post.to_string(), "POST");
        assert_eq!(Method::Patch.to_string(), "PATCH");
        assert_eq!(Method::Delete.to_string(), "DELETE");
        assert_eq!(Method::Options.to_string(), "OPTIONS");
        assert_eq!(Method::Head.to_string(), "HEAD");
    }
}
