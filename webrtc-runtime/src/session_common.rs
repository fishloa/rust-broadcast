//! Request building and established-session response classification shared by
//! the WHIP client and the WHEP player (RFC 9725 §4.3, draft-ietf-wish-whep):
//! both PATCH trickle-ICE fragments and ICE restarts with `If-Match`, DELETE
//! to terminate, and answer a response by the request actually in flight.

use alloc::string::String;
use alloc::vec::Vec;

use headers::{HeaderMapExt, IfMatch};
use http::{HeaderMap, Method, StatusCode};

use crate::Error;
use crate::http_util::{self, HttpRequest, HttpResponse};

/// The kind of request awaiting a response while a session is established
/// (audit run-09 W21): status/`ETag` alone cannot tell a trickle-ICE ack apart
/// from an ICE-restart answer (both are `PATCH`, both can legally come back
/// `200` with an `ETag`) or a `DELETE` ack from a trickle ack (both can
/// legally come back with no body and no `ETag`). Tracking which request is
/// actually in flight removes the ambiguity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingRequest {
    Trickle,
    IceRestart,
    Delete,
}

/// What an established-session response means.
pub(crate) enum Established {
    Terminated,
    IceRestart {
        sdp_fragment: Vec<u8>,
        new_etag: String,
    },
    Acknowledged,
}

/// The endpoint a session was opened against and the bearer token bound to it.
pub(crate) struct Origin<'a> {
    pub(crate) endpoint_url: &'a str,
    pub(crate) bearer_token: Option<&'a str>,
}

impl Origin<'_> {
    /// The token is bound to the endpoint it was configured for: a session URL
    /// (`Location`) on another origin never receives it.
    pub(crate) fn request(
        &self,
        method: Method,
        url: String,
        content_type: Option<&'static str>,
        body: Vec<u8>,
    ) -> Result<HttpRequest, Error> {
        let mut headers = HeaderMap::new();
        if let Some(token) = self.bearer_token
            && http_util::same_origin(self.endpoint_url, &url)
        {
            headers.typed_insert(http_util::bearer(token)?);
        }
        if let Some(ct) = content_type {
            headers.typed_insert(http_util::content_type(ct)?);
        }
        Ok(HttpRequest {
            method,
            url,
            headers,
            body,
        })
    }

    /// Trickle-ICE `PATCH`, conditional on the resource's current `ETag` when
    /// it has one.
    pub(crate) fn trickle(
        &self,
        session_url: String,
        etag: Option<String>,
        fragment: Vec<u8>,
    ) -> Result<HttpRequest, Error> {
        let mut req = self.request(
            Method::PATCH,
            session_url,
            Some(crate::whep::content_type::TRICKLE_ICE),
            fragment,
        )?;
        if let Some(etag) = etag {
            req.headers
                .typed_insert(IfMatch::from(http_util::etag(&etag)?));
        }
        Ok(req)
    }

    /// ICE-restart `PATCH` (`If-Match: *`).
    pub(crate) fn ice_restart(
        &self,
        session_url: String,
        fragment: Vec<u8>,
    ) -> Result<HttpRequest, Error> {
        let mut req = self.request(
            Method::PATCH,
            session_url,
            Some(crate::whep::content_type::TRICKLE_ICE),
            fragment,
        )?;
        req.headers.typed_insert(IfMatch::any());
        Ok(req)
    }

    pub(crate) fn delete(&self, session_url: String) -> Result<HttpRequest, Error> {
        self.request(Method::DELETE, session_url, None, Vec::new())
    }
}

/// Classify `resp` by the request that was in flight, refreshing `etag` when
/// the server supplied a new one.
pub(crate) fn classify_established(
    pending: Option<PendingRequest>,
    resp: HttpResponse,
    etag: &mut Option<String>,
) -> Result<Established, Error> {
    match pending {
        Some(PendingRequest::Delete) => match resp.status {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(Established::Terminated),
            _ => Err(http_error(&resp)),
        },
        Some(PendingRequest::IceRestart) => {
            match (resp.status, http_util::etag_of(&resp.headers)) {
                (StatusCode::OK, Some(new_etag)) => {
                    *etag = Some(new_etag.clone());
                    Ok(Established::IceRestart {
                        sdp_fragment: resp.body,
                        new_etag,
                    })
                }
                (StatusCode::OK, None) => Err(Error::MissingHeader { header: "ETag" }),
                _ => Err(http_error(&resp)),
            }
        }
        Some(PendingRequest::Trickle) => match resp.status {
            StatusCode::OK | StatusCode::NO_CONTENT => {
                if let Some(new_etag) = http_util::etag_of(&resp.headers) {
                    *etag = Some(new_etag);
                }
                Ok(Established::Acknowledged)
            }
            _ => Err(http_error(&resp)),
        },
        None => Err(Error::WrongState {
            operation: "on_response",
            state: "established with no request pending",
        }),
    }
}

fn http_error(resp: &HttpResponse) -> Error {
    Error::Http {
        status: resp.status.as_u16(),
    }
}
