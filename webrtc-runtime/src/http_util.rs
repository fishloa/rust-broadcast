//! Shared HTTP request/response types and typed-header helpers for the WHIP
//! (RFC 9725) and WHEP state machines.
//!
//! The four state machines (`whip::{client,server}`, `whep::{player,server}`)
//! all speak [`http`] types: [`http::Method`], [`http::StatusCode`] and
//! [`http::HeaderMap`], with the `headers` crate's typed headers
//! (`ETag`/`If-Match`/`Content-Type`/`Location`/`Retry-After`/`Authorization`)
//! instead of hand-formatted header strings. This module is crate-private; the
//! types are re-exported from each state-machine module.

use std::time::Duration;

use headers::{
    Authorization, ContentType, ETag, Header, HeaderMapExt, IfMatch, Location, RetryAfter,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode};

use crate::Error;

/// An HTTP request the caller must send on behalf of a state machine.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method to use.
    pub method: Method,
    /// Absolute or resource-relative URL to send the request to.
    pub url: String,
    /// Headers to send (`Content-Type`, `Authorization`, `If-Match`, ...).
    pub headers: HeaderMap,
    /// Request body bytes.
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// The typed `Content-Type` header, if the request carries one.
    pub fn content_type(&self) -> Option<ContentType> {
        self.headers.typed_get()
    }

    /// The typed `If-Match` header, if the request carries one.
    pub fn if_match(&self) -> Option<IfMatch> {
        self.headers.typed_get()
    }
}

/// An HTTP response: the one a server-side state machine asks the caller to
/// send, or the one a client-side state machine is fed from the peer.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: StatusCode,
    /// Response headers (`Content-Type`, `Location`, `ETag`, `Retry-After`, ...).
    pub headers: HeaderMap,
    /// Response body bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// A response with `status`, no headers and no body.
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    /// Set the response body.
    #[must_use]
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// Set the typed `Content-Type` header from a media-type string
    /// (parameters allowed, e.g. `application/sdp; charset=utf-8`).
    ///
    /// # Errors
    /// [`Error::InvalidHeader`] if `mime` is not a valid media type.
    pub fn with_content_type(mut self, mime: &str) -> Result<Self, Error> {
        self.headers.typed_insert(content_type(mime)?);
        Ok(self)
    }

    /// Set the typed `Location` header.
    ///
    /// # Errors
    /// [`Error::InvalidHeader`] if `url` is not a valid header value (for
    /// example it contains non-ASCII characters).
    pub fn with_location(mut self, url: &str) -> Result<Self, Error> {
        self.headers.typed_insert(location(url)?);
        Ok(self)
    }

    /// Set the typed `ETag` header to the strong entity-tag `"opaque"`.
    ///
    /// # Errors
    /// [`Error::InvalidHeader`] if `opaque` cannot be an entity-tag (for
    /// example it contains a double quote or non-ASCII characters).
    pub fn with_etag(mut self, opaque: &str) -> Result<Self, Error> {
        self.headers.typed_insert(etag(opaque)?);
        Ok(self)
    }
}

/// `"opaque"` as a strong typed `ETag`.
pub(crate) fn etag(opaque: &str) -> Result<ETag, Error> {
    format!("\"{opaque}\"")
        .parse()
        .map_err(|_| Error::InvalidHeader { header: "ETag" })
}

/// The opaque tag of a typed `ETag` (quotes and any `W/` prefix removed).
pub(crate) fn etag_opaque(e: &ETag) -> String {
    let mut v: Vec<HeaderValue> = Vec::new();
    e.encode(&mut v);
    v.first()
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim_start_matches("W/").trim_matches('"').to_string())
        .unwrap_or_default()
}

/// A typed `Content-Type` from a media-type string.
pub(crate) fn content_type(mime: &str) -> Result<ContentType, Error> {
    mime.parse().map_err(|_| Error::InvalidHeader {
        header: "Content-Type",
    })
}

/// A typed `Location` from a URL string.
pub(crate) fn location(url: &str) -> Result<Location, Error> {
    let invalid = || Error::InvalidHeader { header: "Location" };
    // A URI reference is ASCII (RFC 3986 §2); `HeaderValue` alone would let raw
    // UTF-8 bytes through as obs-text.
    if !url.is_ascii() {
        return Err(invalid());
    }
    let v = HeaderValue::from_str(url).map_err(|_| invalid())?;
    Location::decode(&mut std::iter::once(&v)).map_err(|_| invalid())
}

/// The `Location` header of `h`, as text.
pub(crate) fn location_of(h: &HeaderMap) -> Option<String> {
    let l = h.typed_get::<Location>()?;
    let mut v: Vec<HeaderValue> = Vec::new();
    l.encode(&mut v);
    v.first().and_then(|h| h.to_str().ok()).map(str::to_string)
}

/// The opaque tag of the `ETag` header of `h`.
///
/// A weak entity-tag yields `None`: it can never satisfy `If-Match` (strong
/// comparison), and re-quoting it as a strong tag would misrepresent it.
pub(crate) fn etag_of(h: &HeaderMap) -> Option<String> {
    h.typed_get::<ETag>()
        .filter(|e| !e.is_weak())
        .map(|e| etag_opaque(&e))
}

/// An `Authorization: Bearer <token>` typed header.
pub(crate) fn bearer(token: &str) -> Result<Authorization<headers::authorization::Bearer>, Error> {
    Authorization::bearer(token).map_err(|_| Error::InvalidHeader {
        header: "Authorization",
    })
}

/// A `Retry-After: <seconds>` typed header.
pub(crate) fn retry_after(delay: Duration) -> RetryAfter {
    RetryAfter::delay(delay)
}

/// True if the request's `Content-Type` is `expected` by essence (parameters
/// ignored, RFC 9110 §8.3.1) and sent exactly once.
pub(crate) fn content_type_is(h: &HeaderMap, expected: &str) -> bool {
    h.get_all(http::header::CONTENT_TYPE).iter().count() == 1
        && h.typed_get::<ContentType>()
            .map(headers::Mime::from)
            .is_some_and(|m| m.essence_str().eq_ignore_ascii_case(expected))
}

/// The `Content-Type` of `h` as text, for error messages.
pub(crate) fn content_type_text(h: &HeaderMap) -> String {
    h.get(http::header::CONTENT_TYPE)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .unwrap_or_default()
}

/// Outcome of evaluating an `If-Match` precondition against the resource's
/// current entity-tag.
pub(crate) enum Precondition {
    /// No `If-Match` header.
    Absent,
    /// `If-Match: *`.
    Any,
    /// `If-Match` lists the current tag (strong comparison).
    Passes,
}

/// Shared `If-Match` evaluation (RFC 9110 §13.1.1) for the two servers.
///
/// # Errors
/// [`Error::InvalidHeader`] if the typed decoder rejects the header;
/// [`Error::ETagMismatch`] if no listed tag strongly matches `current` — a weak
/// tag never does, and neither does an unquoted token such as `etag1`
/// (`headers` 0.4's `If-Match` decoder is lenient about list members: it does
/// not reject a malformed one, it simply never matches it).
/// Consequently a list with one valid current tag plus garbage still passes, and
/// `*` only counts when it is the whole header. `ETagMismatch::got` carries the
/// raw header text (visible ASCII/obs-text by construction of `HeaderValue`);
/// escape it before logging it as untrusted input.
pub(crate) fn check_if_match(h: &HeaderMap, current: &str) -> Result<Precondition, Error> {
    let Some(raw) = h.get(http::header::IF_MATCH) else {
        return Ok(Precondition::Absent);
    };
    let invalid = || Error::InvalidHeader { header: "If-Match" };
    let parsed = h
        .typed_try_get::<IfMatch>()
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    if parsed.is_any() {
        return Ok(Precondition::Any);
    }
    if parsed.precondition_passes(&etag(current)?) {
        Ok(Precondition::Passes)
    } else {
        Err(Error::ETagMismatch {
            expected: current.to_string(),
            got: raw.to_str().unwrap_or("").to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weak_server_etag_is_not_reused_as_a_strong_if_match_tag() {
        let mut h = HeaderMap::new();
        h.insert(http::header::ETAG, HeaderValue::from_static("W/\"v1\""));
        assert_eq!(etag_of(&h), None);
        h.insert(http::header::ETAG, HeaderValue::from_static("\"v1\""));
        assert_eq!(etag_of(&h).as_deref(), Some("v1"));
    }
}
