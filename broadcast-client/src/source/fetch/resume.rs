//! Resuming a media request closed for inactivity with an HTTP range request (RFC 9110 14.2).
//!
//! A response is resumable when it is a `200` carrying `Accept-Ranges: bytes` (14.3) or a `206`,
//! has no `Content-Encoding` (range offsets would no longer be body offsets), and names a strong
//! validator: a strong `ETag` (8.8.3) or a `Last-Modified` at least one second older than the
//! response's `Date` (8.8.2.2). The retry sends `Range` for the bytes not yet received with
//! `If-Range` (13.1.5) naming that validator. Its answer is taken as follows: a `206` whose
//! `Content-Range` (14.4) starts exactly after the received prefix extends it; a `200` (the
//! resource changed or ranges are ignored) replaces it; anything else drops it and asks for the
//! whole resource again.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use dash_runtime::timing::datetime::parse_http_date;
use headers::{
    AcceptRanges, ContentEncoding, ContentLength, ContentRange, Date, ETag, HeaderMapExt, IfRange,
    LastModified, Range,
};
use http::header::{DATE, LAST_MODIFIED};
use http::{HeaderMap, HeaderName, HeaderValue};

use super::body::BodyAcc;

const STATUS_OK: u16 = 200;
const STATUS_PARTIAL_CONTENT: u16 = 206;
const MIN_STRONG_LAST_MODIFIED_AGE_US: i64 = 1_000_000;
const MAX_PARKED: usize = 8;

fn map_of(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.append(name, value);
        }
    }
    map
}

#[derive(Debug, Clone, PartialEq)]
enum Validator {
    ETag(ETag),
    LastModified(LastModified),
}

impl Validator {
    fn of(map: &HeaderMap) -> Option<Self> {
        if let Some(etag) = map.typed_get::<ETag>()
            && !etag.is_weak()
        {
            return Some(Self::ETag(etag));
        }
        let modified = map.typed_get::<LastModified>()?;
        map.typed_get::<Date>()?;
        let instant = |name| parse_http_date(map.get(name)?.to_str().ok()?).ok();
        let age_us = instant(DATE)?.checked_sub(instant(LAST_MODIFIED)?)?;
        (age_us >= MIN_STRONG_LAST_MODIFIED_AGE_US).then_some(Self::LastModified(modified))
    }

    fn if_range(&self) -> IfRange {
        match self {
            Self::ETag(etag) => IfRange::etag(etag.clone()),
            Self::LastModified(at) => IfRange::date((*at).into()),
        }
    }

    fn contradicted_by(&self, map: &HeaderMap) -> bool {
        match self {
            Self::ETag(etag) => map.typed_get::<ETag>().is_some_and(|got| got != *etag),
            Self::LastModified(at) => map
                .typed_get::<LastModified>()
                .is_some_and(|got| got != *at),
        }
    }
}

/// What a resumable response said about the representation: the validator, the offset of the
/// first body byte in the whole representation, and the offset of its last byte when known.
#[derive(Debug, Clone)]
pub(crate) struct Basis {
    validator: Validator,
    start: u64,
    end: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resumed {
    Continue,
    Restart,
    Mismatch,
}

impl Basis {
    pub(crate) fn of(status: u16, headers: &[(String, String)]) -> Option<Self> {
        let map = map_of(headers);
        if map.typed_get::<ContentEncoding>().is_some() {
            return None;
        }
        let validator = Validator::of(&map)?;
        match status {
            STATUS_OK => {
                if !map
                    .typed_get::<AcceptRanges>()
                    .is_some_and(|ranges| ranges.is_bytes())
                {
                    return None;
                }
                let end = map
                    .typed_get::<ContentLength>()
                    .and_then(|len| len.0.checked_sub(1));
                Some(Self {
                    validator,
                    start: 0,
                    end,
                })
            }
            STATUS_PARTIAL_CONTENT => {
                let (start, end) = map.typed_get::<ContentRange>()?.bytes_range()?;
                Some(Self {
                    validator,
                    start,
                    end: Some(end),
                })
            }
            _ => None,
        }
    }

    /// The `Range` and `If-Range` headers asking for everything after the first `received` body
    /// bytes; `None` when nothing was received or nothing is left to ask for.
    pub(crate) fn request_headers(&self, received: u64) -> Option<Vec<(String, String)>> {
        if received == 0 {
            return None;
        }
        let first = self.start.checked_add(received)?;
        if self.end.is_some_and(|end| first > end) {
            return None;
        }
        let range = match self.end {
            Some(end) => Range::bytes(first..=end),
            None => Range::bytes(first..),
        }
        .ok()?;
        let mut map = HeaderMap::new();
        map.typed_insert(range);
        map.typed_insert(self.validator.if_range());
        map.iter()
            .map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
            .collect()
    }

    fn accept(&mut self, status: u16, headers: &[(String, String)], received: u64) -> Resumed {
        match status {
            STATUS_OK => Resumed::Restart,
            STATUS_PARTIAL_CONTENT => {
                let map = map_of(headers);
                if map.typed_get::<ContentEncoding>().is_some()
                    || self.validator.contradicted_by(&map)
                {
                    return Resumed::Mismatch;
                }
                let Some(range) = map.typed_get::<ContentRange>() else {
                    return Resumed::Mismatch;
                };
                let Some((first, last)) = range.bytes_range() else {
                    return Resumed::Mismatch;
                };
                let wanted = self.start.checked_add(received);
                if wanted != Some(first)
                    || last < first
                    || self.end.is_some_and(|end| end != last)
                    || range.bytes_len().is_some_and(|total| last >= total)
                {
                    return Resumed::Mismatch;
                }
                self.end = Some(last);
                Resumed::Continue
            }
            _ => Resumed::Mismatch,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HeadOutcome {
    Accepted,
    Refused,
    Fallback,
}

/// Takes the 2xx response head of a request: a fresh request or the resumed one when
/// `resume_from` names the bytes already kept in `body`. `Refused` is a ranged request the
/// answer does not honour; `Fallback` is a resumed request the answer does not continue, with
/// `body` emptied so the caller asks for the whole resource again.
pub(crate) fn accept_head(
    body: &mut BodyAcc,
    basis: &mut Option<Basis>,
    resume_from: &mut Option<u64>,
    status: u16,
    headers: &[(String, String)],
    byte_range: Option<(u64, u64)>,
) -> HeadOutcome {
    if let Some(received) = resume_from.take() {
        let outcome = basis
            .as_mut()
            .map_or(Resumed::Mismatch, |b| b.accept(status, headers, received));
        match outcome {
            Resumed::Continue => {
                let suffix = map_of(headers).typed_get::<ContentLength>().map(|l| l.0);
                body.expect_suffix(suffix);
                return HeadOutcome::Accepted;
            }
            Resumed::Mismatch => {
                *basis = None;
                body.reset();
                return HeadOutcome::Fallback;
            }
            Resumed::Restart => {
                *basis = None;
                body.reset();
            }
        }
    }
    if body.accept(status, headers, byte_range) {
        *basis = Basis::of(status, headers);
        HeadOutcome::Accepted
    } else {
        *basis = None;
        HeadOutcome::Refused
    }
}

/// A closed request's received prefix, kept until the same resource is requested again.
#[derive(Debug)]
pub(crate) struct Parked {
    pub(crate) url: String,
    pub(crate) byte_range: Option<(u64, u64)>,
    pub(crate) body: BodyAcc,
    pub(crate) basis: Basis,
}

#[derive(Debug)]
pub(crate) struct Stash<K> {
    items: Vec<(K, Parked)>,
}

impl<K: PartialEq> Stash<K> {
    pub(crate) fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub(crate) fn put(&mut self, key: K, parked: Parked) {
        self.items.retain(|(k, _)| *k != key);
        if self.items.len() >= MAX_PARKED {
            self.items.remove(0);
        }
        self.items.push((key, parked));
    }

    /// The prefix kept for `key`, if it was fetched from this `url` and `byte_range`.
    pub(crate) fn take(
        &mut self,
        key: &K,
        url: &str,
        byte_range: Option<(u64, u64)>,
    ) -> Option<Parked> {
        let at = self.items.iter().position(|(k, _)| k == key)?;
        let (_, parked) = self.items.remove(at);
        (parked.url == url && parked.byte_range == byte_range).then_some(parked)
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn etag_200() -> Vec<(String, String)> {
        h(&[
            ("ETag", "\"v1\""),
            ("Accept-Ranges", "bytes"),
            ("Content-Length", "100"),
        ])
    }

    fn range_of(basis: &Basis, received: u64) -> Vec<(String, String)> {
        basis.request_headers(received).unwrap()
    }

    #[test]
    fn a_200_with_ranges_and_a_strong_etag_resumes_from_the_received_offset() {
        let basis = Basis::of(200, &etag_200()).unwrap();
        assert_eq!(
            range_of(&basis, 40),
            h(&[("range", "bytes=40-99"), ("if-range", "\"v1\"")])
        );
    }

    #[test]
    fn a_200_of_unknown_length_asks_open_ended() {
        let basis = Basis::of(200, &h(&[("ETag", "\"v1\""), ("Accept-Ranges", "bytes")])).unwrap();
        assert_eq!(range_of(&basis, 7)[0], ("range".into(), "bytes=7-".into()));
    }

    #[test]
    fn a_206_resumes_inside_its_own_range() {
        let basis = Basis::of(
            206,
            &h(&[("ETag", "\"v1\""), ("Content-Range", "bytes 500-1499/9000")]),
        )
        .unwrap();
        assert_eq!(
            range_of(&basis, 100)[0],
            ("range".into(), "bytes=600-1499".into())
        );
    }

    #[test]
    fn no_resume_without_a_strong_validator_or_range_support_or_with_a_content_coding() {
        let none = |status, pairs: &[(&str, &str)]| Basis::of(status, &h(pairs)).is_none();
        assert!(none(200, &[("Accept-Ranges", "bytes")]));
        assert!(none(
            200,
            &[("ETag", "W/\"v1\""), ("Accept-Ranges", "bytes")]
        ));
        assert!(none(200, &[("ETag", "\"v1\"")]));
        assert!(none(
            200,
            &[
                ("ETag", "\"v1\""),
                ("Accept-Ranges", "bytes"),
                ("Content-Encoding", "gzip")
            ]
        ));
        assert!(none(200, &[("ETag", "\"v1\""), ("Accept-Ranges", "none")]));
        assert!(none(404, &[("ETag", "\"v1\""), ("Accept-Ranges", "bytes")]));
    }

    #[test]
    fn last_modified_is_a_validator_only_a_second_older_than_date() {
        let at = |date| {
            Basis::of(
                200,
                &h(&[
                    ("Last-Modified", "Tue, 15 Nov 1994 12:45:26 GMT"),
                    ("Date", date),
                    ("Accept-Ranges", "bytes"),
                ]),
            )
        };
        assert!(at("Tue, 15 Nov 1994 12:45:27 GMT").is_some());
        assert!(at("Tue, 15 Nov 1994 12:45:26 GMT").is_none());
        let no_date = Basis::of(
            200,
            &h(&[
                ("Last-Modified", "Tue, 15 Nov 1994 12:45:26 GMT"),
                ("Accept-Ranges", "bytes"),
            ]),
        );
        assert!(no_date.is_none());
        let basis = at("Tue, 15 Nov 1994 12:46:00 GMT").unwrap();
        assert_eq!(
            range_of(&basis, 1)[1],
            ("if-range".into(), "Tue, 15 Nov 1994 12:45:26 GMT".into())
        );
    }

    #[test]
    fn a_weak_etag_falls_back_to_a_strong_last_modified() {
        let basis = Basis::of(
            200,
            &h(&[
                ("ETag", "W/\"v1\""),
                ("Last-Modified", "Tue, 15 Nov 1994 12:45:26 GMT"),
                ("Date", "Tue, 15 Nov 1994 13:00:00 GMT"),
                ("Accept-Ranges", "bytes"),
            ]),
        )
        .unwrap();
        assert_eq!(range_of(&basis, 1)[1].0, "if-range");
    }

    #[test]
    fn nothing_received_or_nothing_left_has_no_request() {
        let basis = Basis::of(200, &etag_200()).unwrap();
        assert!(basis.request_headers(0).is_none());
        assert!(basis.request_headers(100).is_none());
        assert!(basis.request_headers(99).is_some());
        assert!(basis.request_headers(u64::MAX).is_none());
    }

    fn resumed(headers: &[(&str, &str)], status: u16, received: u64) -> (HeadOutcome, BodyAcc) {
        let mut basis = Basis::of(200, &etag_200());
        let mut body = BodyAcc::new(1024);
        body.push(&vec![0u8; usize::try_from(received).unwrap()])
            .unwrap();
        let mut from = Some(received);
        let head = accept_head(&mut body, &mut basis, &mut from, status, &h(headers), None);
        (head, body)
    }

    #[test]
    fn a_206_starting_after_the_prefix_extends_it() {
        let (head, body) = resumed(
            &[
                ("Content-Range", "bytes 40-99/100"),
                ("Content-Length", "60"),
            ],
            206,
            40,
        );
        assert_eq!(head, HeadOutcome::Accepted);
        assert_eq!(body.len(), 40);
        let mut body = body;
        body.push(&[1u8; 59]).unwrap();
        assert!(body.is_truncated(), "one byte short of 40 + 60");
        body.push(&[1u8]).unwrap();
        assert!(!body.is_truncated());
    }

    #[test]
    fn a_206_that_does_not_continue_the_prefix_falls_back_with_an_empty_body() {
        for range in [
            "bytes 41-99/100",
            "bytes 39-99/100",
            "bytes 40-98/100",
            "bytes 40-120/100",
            "bytes */100",
        ] {
            let (head, body) = resumed(&[("Content-Range", range)], 206, 40);
            assert_eq!(head, HeadOutcome::Fallback, "{range}");
            assert_eq!(body.len(), 0, "{range}");
        }
        let (head, _) = resumed(&[], 206, 40);
        assert_eq!(head, HeadOutcome::Fallback, "no Content-Range");
    }

    #[test]
    fn a_206_naming_another_etag_falls_back() {
        let (head, _) = resumed(
            &[("Content-Range", "bytes 40-99/100"), ("ETag", "\"v2\"")],
            206,
            40,
        );
        assert_eq!(head, HeadOutcome::Fallback);
        let (head, _) = resumed(
            &[("Content-Range", "bytes 40-99/100"), ("ETag", "\"v1\"")],
            206,
            40,
        );
        assert_eq!(head, HeadOutcome::Accepted);
    }

    #[test]
    fn a_200_to_a_resume_replaces_the_prefix() {
        let (head, body) = resumed(&[("Content-Length", "100")], 200, 40);
        assert_eq!(head, HeadOutcome::Accepted);
        assert_eq!(body.len(), 0);
    }

    #[test]
    fn any_other_status_to_a_resume_falls_back() {
        for status in [201, 204, 205] {
            let (head, _) = resumed(&[], status, 40);
            assert_eq!(head, HeadOutcome::Fallback, "{status}");
        }
    }

    fn parked(url: &str) -> Parked {
        Parked {
            url: url.into(),
            byte_range: None,
            body: BodyAcc::new(8),
            basis: Basis::of(200, &etag_200()).unwrap(),
        }
    }

    #[test]
    fn a_parked_prefix_is_taken_once_and_only_for_the_same_request() {
        let mut stash = Stash::new();
        stash.put(1u8, parked("http://a/x"));
        assert!(stash.take(&1, "http://a/y", None).is_none());
        stash.put(1u8, parked("http://a/x"));
        assert!(stash.take(&1, "http://a/x", Some((0, 1))).is_none());
        stash.put(1u8, parked("http://a/x"));
        assert!(stash.take(&1, "http://a/x", None).is_some());
        assert!(stash.take(&1, "http://a/x", None).is_none());
    }

    #[test]
    fn the_stash_keeps_at_most_a_bounded_number_of_prefixes() {
        let mut stash = Stash::new();
        for key in 0..u32::try_from(MAX_PARKED + 5).unwrap() {
            stash.put(key, parked("http://a/x"));
        }
        assert_eq!(stash.len(), MAX_PARKED);
        assert!(
            stash.take(&0, "http://a/x", None).is_none(),
            "oldest evicted"
        );
    }
}
