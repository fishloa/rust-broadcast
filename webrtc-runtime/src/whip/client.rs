//! WHIP client (encoder/ingester) state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

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

/// An HTTP request the caller must send on behalf of the state machine.
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
    /// `POST` — used to create a session with the initial SDP offer.
    Post,
    /// `PATCH` — used to send Trickle ICE fragments / ICE restarts.
    Patch,
    /// `DELETE` — used to terminate a session.
    Delete,
}

impl Method {
    /// The HTTP method token as it appears on the request line.
    pub fn name(&self) -> &'static str {
        match self {
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
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

/// The kind of request currently awaiting a response while [`State::Established`]
/// (audit run-09 W21): `handle_established_response` used to classify a
/// response by its status code and `ETag` alone, which cannot tell a
/// trickle-ICE ack apart from an ICE-restart answer (both are PATCH,
/// both can legally come back `200` with an `ETag`) or a `DELETE` ack from a
/// trickle ack (both can legally come back with no body and no `ETag`).
/// Tracking which request is actually in flight removes the ambiguity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRequest {
    /// A trickle-ICE (aggregated candidate) `PATCH`.
    Trickle,
    /// An ICE-restart `PATCH` (`If-Match: *`).
    IceRestart,
    /// A `DELETE`.
    Delete,
}

/// Sans-IO WHIP client state machine.
///
/// Caller drives it by:
/// 1. Calling methods to get `HttpRequest`s to send
/// 2. Feeding HTTP responses back via `on_response`
/// 3. Handling emitted `Event`s
#[derive(Debug)]
pub struct WhipClient {
    /// WHIP endpoint URL the initial offer is POSTed to.
    endpoint_url: String,
    /// Bearer token sent as `Authorization` on every request, if configured.
    bearer_token: Option<String>,
    /// Current client-side session state.
    state: State,
    /// The request awaiting a response while [`State::Established`], if any
    /// — see [`PendingRequest`].
    pending: Option<PendingRequest>,
}

impl WhipClient {
    /// Create a new client targeting `endpoint_url`, optionally authenticating
    /// every request with `bearer_token`.
    pub fn new(endpoint_url: String, bearer_token: Option<String>) -> Self {
        Self {
            endpoint_url,
            bearer_token,
            state: State::Idle,
            pending: None,
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
        self.state = State::OfferSent;
        Ok(self.build_request(
            Method::Post,
            self.endpoint_url.clone(),
            Some(super::content_type::SDP),
            sdp_offer,
        ))
    }

    /// Generate aggregated Trickle ICE PATCH for the caller's already-aggregated
    /// candidate fragment.
    pub fn flush_candidates(&mut self, aggregated_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, etag) = self.established_fields()?;
        let mut req = self.build_request(
            Method::Patch,
            session_url,
            Some(super::content_type::TRICKLE_ICE),
            aggregated_fragment,
        );
        if let Some(etag) = etag {
            req.headers
                .push(("If-Match".into(), alloc::format!("\"{etag}\"")));
        }
        self.pending = Some(PendingRequest::Trickle);
        Ok(req)
    }

    /// Generate ICE restart PATCH request.
    pub fn ice_restart(&mut self, sdp_fragment: Vec<u8>) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        let mut req = self.build_request(
            Method::Patch,
            session_url,
            Some(super::content_type::TRICKLE_ICE),
            sdp_fragment,
        );
        req.headers.push(("If-Match".into(), "*".into()));
        self.pending = Some(PendingRequest::IceRestart);
        Ok(req)
    }

    /// Generate DELETE request to terminate the session.
    pub fn terminate(&mut self) -> Result<HttpRequest, Error> {
        let (session_url, _) = self.established_fields()?;
        self.pending = Some(PendingRequest::Delete);
        Ok(self.build_request(Method::Delete, session_url, None, Vec::new()))
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
        if resp.status == super::status::CREATED {
            let session_url = resp
                .location
                .ok_or(Error::MissingHeader { header: "Location" })?;
            self.state = State::Established {
                session_url,
                etag: resp.etag,
            };
            Ok(Some(Event::SdpAnswer(resp.body)))
        } else {
            Err(Error::Http {
                status: resp.status,
            })
        }
    }

    /// Dispatches on which request is actually in flight (audit run-09
    /// W21), rather than guessing from status/`ETag` alone: those are
    /// ambiguous between a trickle-ICE ack, an ICE-restart answer, and a
    /// `DELETE` ack (see [`PendingRequest`]).
    fn handle_established_response(&mut self, resp: HttpResponse) -> Result<Option<Event>, Error> {
        match self.pending.take() {
            Some(PendingRequest::Delete) => match resp.status {
                200 | super::status::NO_CONTENT => {
                    self.state = State::Closed;
                    Ok(Some(Event::Terminated))
                }
                _ => Err(Error::Http {
                    status: resp.status,
                }),
            },
            Some(PendingRequest::IceRestart) => match (resp.status, resp.etag) {
                (200, Some(new_etag)) => {
                    if let State::Established { etag, .. } = &mut self.state {
                        *etag = Some(new_etag.clone());
                    }
                    Ok(Some(Event::IceRestart {
                        sdp_fragment: resp.body,
                        new_etag,
                    }))
                }
                (200, None) => Err(Error::MissingHeader { header: "ETag" }),
                _ => Err(Error::Http {
                    status: resp.status,
                }),
            },
            Some(PendingRequest::Trickle) => match resp.status {
                200 | super::status::NO_CONTENT => {
                    if let Some(new_etag) = resp.etag
                        && let State::Established { etag, .. } = &mut self.state
                    {
                        *etag = Some(new_etag);
                    }
                    Ok(None)
                }
                _ => Err(Error::Http {
                    status: resp.status,
                }),
            },
            None => Err(Error::WrongState {
                operation: "on_response",
                state: "established with no request pending",
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
        State::Established { .. } => "established",
        State::Closed => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn method_display_matches_http_request_line_token() {
        assert_eq!(Method::Post.to_string(), "POST");
        assert_eq!(Method::Patch.to_string(), "PATCH");
        assert_eq!(Method::Delete.to_string(), "DELETE");
    }
}
