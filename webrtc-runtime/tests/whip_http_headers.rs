//! WHIP/WHEP typed-header strictness (W1-R-low-b, review focus 4).
//!
//! `If-Match`, `Content-Type`, `Location`, `ETag` and `Retry-After` are read and
//! written through the `headers` crate's typed headers, so malformed values are
//! rejected the way RFC 9110 says, instead of by hand-rolled string compares.

use headers::HeaderMapExt;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::Error;
use webrtc_runtime::whep::server::WhepSession;
use webrtc_runtime::whip::server::{Event, WhipSession};

const SESSION: &str = "https://o.example/s/1";

fn established_whip() -> WhipSession {
    let mut s = WhipSession::new(SESSION.into());
    let _ = s.accept(b"a".to_vec(), "etag1".into());
    s
}

fn established_whep() -> WhepSession {
    let mut s = WhepSession::new(SESSION.into());
    let _ = s.accept(b"a".to_vec(), "etag1".into());
    s
}

fn h(name: header::HeaderName, v: &str) -> HeaderMap {
    let mut m = HeaderMap::new();
    m.insert(name, HeaderValue::from_str(v).unwrap());
    m
}

fn trickle_headers(if_match: Option<&str>) -> HeaderMap {
    let mut m = h(header::CONTENT_TYPE, "application/trickle-ice-sdpfrag");
    if let Some(v) = if_match {
        m.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
    }
    m
}

#[test]
fn if_match_star_means_ice_restart() {
    let mut s = established_whip();
    let ev = s
        .on_patch(b"f".to_vec(), &h(header::IF_MATCH, "*"))
        .unwrap();
    assert!(matches!(ev, Event::IceRestart { .. }));
}

#[test]
fn if_match_with_the_current_strong_etag_is_trickle_ice() {
    let mut s = established_whip();
    let ev = s
        .on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"etag1\""))
        .unwrap();
    assert!(matches!(ev, Event::TrickleIce { if_match: Some(ref e), .. } if e == "etag1"));
}

#[test]
fn if_match_with_a_stale_etag_is_a_mismatch_carrying_both_values() {
    let mut s = established_whip();
    let err = s
        .on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"old\""))
        .unwrap_err();
    assert!(
        matches!(err, Error::ETagMismatch { ref expected, ref got } if expected == "etag1" && got == "\"old\""),
        "{err:?}"
    );
}

/// UPSTREAM-BEHAVIOUR PIN: the strong comparison is `headers::IfMatch::
/// precondition_passes`, not code in this crate, so this test cannot fail on
/// our code; it only guards against a future swap to a weak/custom comparison.
#[test]
fn a_weak_etag_never_satisfies_if_match_strong_comparison() {
    let mut s = established_whip();
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "W/\"etag1\"")),
        Err(Error::ETagMismatch { .. })
    ));
}

/// RFC 9110 `If-Match` members are quoted entity-tags (or the bare `*`). An
/// unquoted token and a quoted `"*"` are malformed and never match. (`headers`
/// 0.4.2's `IfMatch` decoder does not reject a malformed list member: the
/// observable result is a precondition failure, `ETagMismatch`, not
/// `InvalidHeader`.)
#[test]
fn an_unquoted_etag_and_a_quoted_star_are_malformed() {
    let mut s = established_whip();
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "etag1")),
        Err(Error::ETagMismatch { ref got, .. }) if got == "etag1"
    ));
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"*\"")),
        Err(Error::ETagMismatch { .. })
    ));
}

#[test]
fn a_patch_without_if_match_is_a_plain_trickle() {
    let mut s = established_whip();
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &HeaderMap::new()).unwrap(),
        Event::TrickleIce { if_match: None, .. }
    ));
}

/// An `If-Match` listing several tags passes when any of them is the current
/// strong tag, including when the header is sent as two separate field lines.
#[test]
fn if_match_lists_and_repeated_field_lines_are_honoured() {
    let mut s = established_whip();
    let ev = s
        .on_patch(
            b"f".to_vec(),
            &h(header::IF_MATCH, "\"old\", \"etag1\", \"older\""),
        )
        .unwrap();
    assert!(matches!(
        ev,
        Event::TrickleIce {
            if_match: Some(_),
            ..
        }
    ));

    let mut twice = HeaderMap::new();
    twice.append(header::IF_MATCH, HeaderValue::from_static("\"old\""));
    twice.append(header::IF_MATCH, HeaderValue::from_static("\"etag1\""));
    let ev = s.on_patch(b"f".to_vec(), &twice).unwrap();
    assert!(matches!(
        ev,
        Event::TrickleIce {
            if_match: Some(_),
            ..
        }
    ));
}

/// A header value that is not UTF-8 is malformed, never a panic and never a match.
#[test]
fn a_non_utf8_if_match_is_rejected() {
    let mut s = established_whip();
    let mut m = HeaderMap::new();
    m.insert(
        header::IF_MATCH,
        HeaderValue::from_bytes(b"\"et\xFFag1\"").unwrap(),
    );
    let err = s.on_patch(b"f".to_vec(), &m).unwrap_err();
    assert!(
        matches!(
            err,
            Error::InvalidHeader { .. } | Error::ETagMismatch { .. }
        ),
        "{err:?}"
    );
}

#[test]
fn whep_counter_offer_answer_accepts_media_type_parameters_and_rejects_others() {
    let mut s = WhepSession::new(SESSION.into());
    let _ = s.counter_offer(b"o".to_vec(), None);
    let ok = s.on_patch(
        b"ans".to_vec(),
        &h(header::CONTENT_TYPE, "application/sdp; charset=utf-8"),
    );
    assert!(ok.is_ok(), "{ok:?}");
    let mut s = WhepSession::new(SESSION.into());
    let _ = s.counter_offer(b"o".to_vec(), None);
    let bad = s.on_patch(b"x".to_vec(), &h(header::CONTENT_TYPE, "text/plain"));
    assert!(bad.is_err());
}

/// A `Content-Type` sent twice is ambiguous: it is rejected, not resolved by
/// picking one of the two.
#[test]
fn a_duplicated_content_type_is_rejected() {
    let mut s = established_whep();
    let mut m = HeaderMap::new();
    m.append(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/trickle-ice-sdpfrag"),
    );
    m.append(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    assert!(s.on_patch(b"f".to_vec(), &m).is_err());
}

#[test]
fn whep_established_patch_uses_the_same_typed_if_match() {
    let mut s = established_whep();
    let ev = s
        .on_patch(b"f".to_vec(), &trickle_headers(Some("\"etag1\"")))
        .unwrap();
    assert!(matches!(
        ev,
        webrtc_runtime::whep::server::Event::TrickleIce {
            if_match: Some(_),
            ..
        }
    ));
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &trickle_headers(Some("etag1"))),
        Err(Error::ETagMismatch { .. })
    ));
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &trickle_headers(Some("W/\"etag1\""))),
        Err(Error::ETagMismatch { .. })
    ));
}

#[test]
fn responses_carry_typed_location_etag_and_content_type() {
    let mut s = WhipSession::new(SESSION.into());
    let r = s.accept(b"a".to_vec(), "etag1".into());
    assert_eq!(r.status, StatusCode::CREATED);
    assert_eq!(r.headers.get(header::LOCATION).unwrap(), SESSION);
    assert_eq!(r.headers.get(header::ETAG).unwrap(), "\"etag1\"");
    assert_eq!(
        r.headers
            .typed_get::<headers::ContentType>()
            .unwrap()
            .to_string(),
        "application/sdp"
    );
}

#[test]
fn no_publisher_sets_retry_after_in_seconds() {
    let r = WhepSession::no_publisher(Some(std::time::Duration::from_secs(5)));
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.headers.get(header::RETRY_AFTER).unwrap(), "5");
    assert!(
        WhepSession::no_publisher(None)
            .headers
            .get(header::RETRY_AFTER)
            .is_none()
    );
}

#[test]
fn a_non_ascii_location_is_an_invalid_header_not_a_panic() {
    let r = webrtc_runtime::whip::client::HttpResponse::new(StatusCode::CREATED)
        .with_location("https://o.example/\u{1F600}");
    assert!(matches!(
        r,
        Err(Error::InvalidHeader { header: "Location" })
    ));
}

/// A session URL or ETag that cannot be sent as a header makes `accept` answer
/// `500` with no `Location`, and leaves the session where it was.
#[test]
fn accept_with_an_unsendable_url_or_etag_is_a_500_not_a_panic() {
    let mut s = WhipSession::new("https://o/\u{1F600}".into());
    let r = s.accept(b"a".to_vec(), "etag1".into());
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(r.headers.get(header::LOCATION).is_none());
    assert!(matches!(
        s.state(),
        webrtc_runtime::whip::server::State::AwaitingOffer
    ));

    let mut s = WhipSession::new(SESSION.into());
    let r = s.accept(b"a".to_vec(), "bad\"etag".into());
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);

    let mut s = WhepSession::new("https://o/\u{1F600}".into());
    assert_eq!(
        s.accept(b"a".to_vec(), "e".into()).status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
