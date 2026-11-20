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

use broadcast_common::http_date::parse_http_date;
use headers::{
    AcceptRanges, ContentEncoding, ContentLength, Date, ETag, HeaderMapExt, IfRange, LastModified,
    Range,
};
use http::header::{DATE, LAST_MODIFIED};
use http::{HeaderMap, StatusCode};

use super::body::BodyAcc;

const MIN_STRONG_LAST_MODIFIED_AGE_US: i64 = 1_000_000;
const MAX_PARKED: usize = 8;
/// Parked prefixes kept per [`crate::Config::max_segment_bytes`]: two stalled segment bodies.
const PARKED_SEGMENTS: usize = 2;

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
        let age_us =
            i128::from(instant(DATE)?.as_microsecond() - instant(LAST_MODIFIED)?.as_microsecond());
        (age_us >= i128::from(MIN_STRONG_LAST_MODIFIED_AGE_US))
            .then_some(Self::LastModified(modified))
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
    pub(crate) fn of(status: StatusCode, map: &HeaderMap) -> Option<Self> {
        if map.typed_get::<ContentEncoding>().is_some() {
            return None;
        }
        let validator = Validator::of(map)?;
        match status {
            StatusCode::OK => {
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
            StatusCode::PARTIAL_CONTENT => {
                let range = super::body::content_range(map)?;
                Some(Self {
                    validator,
                    start: range.first,
                    end: Some(range.last),
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

    fn remaining(&self, received: u64) -> Option<u64> {
        let first = self.start.checked_add(received)?;
        self.end?.checked_sub(first)?.checked_add(1)
    }

    fn accept(&mut self, status: StatusCode, map: &HeaderMap, received: u64) -> Resumed {
        match status {
            StatusCode::OK => Resumed::Restart,
            StatusCode::PARTIAL_CONTENT => {
                if map.typed_get::<ContentEncoding>().is_some()
                    || self.validator.contradicted_by(map)
                {
                    return Resumed::Mismatch;
                }
                let Some(range) = super::body::content_range(map) else {
                    return Resumed::Mismatch;
                };
                let wanted = self.start.checked_add(received);
                if wanted != Some(range.first) || self.end.is_some_and(|end| end != range.last) {
                    return Resumed::Mismatch;
                }
                self.end = Some(range.last);
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
    Oversize,
}

/// Takes the 2xx response head of a request: a fresh request or the resumed one when
/// `resume_from` names the bytes already kept in `body`. `Refused` is a ranged request the
/// answer does not honour; `Fallback` is a resumed request the answer does not continue, with
/// `body` emptied so the caller asks for the whole resource again; `Oversize` is an answer whose
/// declared length is already past the cap.
pub(crate) fn accept_head(
    body: &mut BodyAcc,
    basis: &mut Option<Basis>,
    resume_from: &mut Option<u64>,
    status: StatusCode,
    headers: &HeaderMap,
    byte_range: Option<broadcast_common::range::ByteSpan>,
) -> HeadOutcome {
    if let Some(received) = resume_from.take() {
        let outcome = basis
            .as_mut()
            .map_or(Resumed::Mismatch, |b| b.accept(status, headers, received));
        match outcome {
            Resumed::Continue => {
                let suffix = headers
                    .typed_get::<ContentLength>()
                    .map(|l| l.0)
                    .or_else(|| basis.as_ref().and_then(|b| b.remaining(received)));
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
    match body.accept(status, headers, byte_range) {
        super::body::HeadVerdict::Accepted => {
            *basis = Basis::of(status, headers);
            HeadOutcome::Accepted
        }
        super::body::HeadVerdict::Refused => {
            *basis = None;
            HeadOutcome::Refused
        }
        super::body::HeadVerdict::Oversize => {
            *basis = None;
            HeadOutcome::Oversize
        }
    }
}

/// A closed request's received prefix, kept until the same resource is requested again.
#[derive(Debug)]
pub(crate) struct Parked {
    pub(crate) url: String,
    pub(crate) byte_range: Option<broadcast_common::range::ByteSpan>,
    pub(crate) body: BodyAcc,
    pub(crate) basis: Basis,
}

/// Parked prefixes kept for later resumes, bounded by count and by the bytes they hold together:
/// a host that stalls every request in flight would otherwise keep [`MAX_PARKED`] full segment
/// bodies alive next to the ones being downloaded.
#[derive(Debug)]
pub(crate) struct Stash<K> {
    items: Vec<(K, Parked)>,
    bytes: usize,
    budget: usize,
}

impl<K: PartialEq> Stash<K> {
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            items: Vec::new(),
            bytes: 0,
            budget,
        }
    }

    /// A stash whose parked prefixes may hold [`PARKED_SEGMENTS`] bodies of at most
    /// `max_segment_bytes` together.
    pub(crate) fn with_segment_budget(max_segment_bytes: usize) -> Self {
        Self::new(max_segment_bytes.saturating_mul(PARKED_SEGMENTS))
    }

    pub(crate) fn put(&mut self, key: K, parked: Parked) {
        if let Some(at) = self.items.iter().position(|(k, _)| *k == key) {
            self.take_at(at);
        }
        let len = parked.body.len();
        if len > self.budget {
            return;
        }
        while !self.items.is_empty()
            && (self.items.len() >= MAX_PARKED || self.bytes.saturating_add(len) > self.budget)
        {
            self.take_at(0);
        }
        self.bytes = self.bytes.saturating_add(len);
        self.items.push((key, parked));
    }

    /// The prefix kept for `key`, if it was fetched from this `url` and `byte_range`.
    pub(crate) fn take(
        &mut self,
        key: &K,
        url: &str,
        byte_range: Option<broadcast_common::range::ByteSpan>,
    ) -> Option<Parked> {
        let at = self.items.iter().position(|(k, _)| k == key)?;
        let parked = self.take_at(at);
        (parked.url == url && parked.byte_range == byte_range).then_some(parked)
    }

    pub(crate) fn discard(&mut self, key: &K) {
        let mut kept = Vec::new();
        for (k, parked) in self.items.drain(..) {
            if &k == key {
                self.bytes = self.bytes.saturating_sub(parked.body.len());
            } else {
                kept.push((k, parked));
            }
        }
        self.items = kept;
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
    }

    fn take_at(&mut self, at: usize) -> Parked {
        let (_, parked) = self.items.remove(at);
        self.bytes = self.bytes.saturating_sub(parked.body.len());
        parked
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderName, HeaderValue};

    fn h(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            let name = HeaderName::from_bytes(k.as_bytes()).unwrap();
            map.append(name, HeaderValue::from_str(v).unwrap());
        }
        map
    }

    fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn etag_200() -> HeaderMap {
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
        let basis = Basis::of(StatusCode::OK, &etag_200()).unwrap();
        assert_eq!(
            range_of(&basis, 40),
            pairs(&[("range", "bytes=40-99"), ("if-range", "\"v1\"")])
        );
    }

    #[test]
    fn a_200_of_unknown_length_asks_open_ended() {
        let basis = Basis::of(
            StatusCode::OK,
            &h(&[("ETag", "\"v1\""), ("Accept-Ranges", "bytes")]),
        )
        .unwrap();
        assert_eq!(range_of(&basis, 7)[0], ("range".into(), "bytes=7-".into()));
    }

    #[test]
    fn a_206_resumes_inside_its_own_range() {
        let basis = Basis::of(
            StatusCode::PARTIAL_CONTENT,
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
        let none = |status: u16, pairs: &[(&str, &str)]| {
            Basis::of(StatusCode::from_u16(status).unwrap(), &h(pairs)).is_none()
        };
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
                StatusCode::OK,
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
            StatusCode::OK,
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
            StatusCode::OK,
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
        let basis = Basis::of(StatusCode::OK, &etag_200()).unwrap();
        assert!(basis.request_headers(0).is_none());
        assert!(basis.request_headers(100).is_none());
        assert!(basis.request_headers(99).is_some());
        assert!(basis.request_headers(u64::MAX).is_none());
    }

    fn resumed(headers: &[(&str, &str)], status: u16, received: u64) -> (HeadOutcome, BodyAcc) {
        let mut basis = Basis::of(StatusCode::OK, &etag_200());
        let mut body = BodyAcc::new(1024);
        body.push(&vec![0u8; usize::try_from(received).unwrap()])
            .unwrap();
        let mut from = Some(received);
        let head = accept_head(
            &mut body,
            &mut basis,
            &mut from,
            StatusCode::from_u16(status).unwrap(),
            &h(headers),
            None,
        );
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
    fn a_resumed_206_without_content_length_is_truncated_by_its_content_range() {
        let (head, mut body) = resumed(&[("Content-Range", "bytes 40-99/100")], 206, 40);
        assert_eq!(head, HeadOutcome::Accepted);
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
            basis: Basis::of(StatusCode::OK, &etag_200()).unwrap(),
        }
    }

    #[test]
    fn a_parked_prefix_is_taken_once_and_only_for_the_same_request() {
        let mut stash = Stash::new(1024);
        stash.put(1u8, parked("http://a/x"));
        assert!(stash.take(&1, "http://a/y", None).is_none());
        stash.put(1u8, parked("http://a/x"));
        assert!(
            stash
                .take(
                    &1,
                    "http://a/x",
                    Some(broadcast_common::range::ByteSpan::new(
                        0,
                        core::num::NonZeroU64::new(1).expect("non-zero"),
                    )),
                )
                .is_none()
        );
        stash.put(1u8, parked("http://a/x"));
        assert!(stash.take(&1, "http://a/x", None).is_some());
        assert!(stash.take(&1, "http://a/x", None).is_none());
    }

    #[test]
    fn a_discarded_prefix_is_gone_and_others_stay() {
        let mut stash = Stash::new(1024);
        stash.put(1u8, parked("http://a/x"));
        stash.put(2u8, parked("http://a/y"));
        stash.discard(&1);
        assert_eq!(stash.len(), 1);
        assert!(stash.take(&1, "http://a/x", None).is_none());
        assert!(stash.take(&2, "http://a/y", None).is_some());
    }

    #[test]
    fn the_stash_keeps_at_most_a_bounded_number_of_prefixes() {
        let mut stash = Stash::new(1024);
        for key in 0..u32::try_from(MAX_PARKED + 5).unwrap() {
            stash.put(key, parked("http://a/x"));
        }
        assert_eq!(stash.len(), MAX_PARKED);
        assert!(
            stash.take(&0, "http://a/x", None).is_none(),
            "oldest evicted"
        );
    }

    fn parked_holding(url: &str, bytes: usize) -> Parked {
        let mut body = BodyAcc::new(bytes);
        body.push(&vec![0u8; bytes])
            .expect("fills the body exactly");
        Parked {
            url: url.into(),
            byte_range: None,
            body,
            basis: Basis::of(StatusCode::OK, &etag_200()).unwrap(),
        }
    }

    #[test]
    fn the_stash_evicts_the_oldest_prefix_that_would_outgrow_its_byte_budget() {
        let mut stash = Stash::new(20);
        stash.put(1u8, parked_holding("http://a/x", 12));
        stash.put(2u8, parked_holding("http://a/y", 12));
        assert_eq!(stash.len(), 1, "the oldest made room for the newest");
        assert!(stash.take(&1, "http://a/x", None).is_none());
        assert!(stash.take(&2, "http://a/y", None).is_some());

        let mut stash = Stash::new(10);
        stash.put(3u8, parked_holding("http://a/x", 11));
        assert_eq!(
            stash.len(),
            0,
            "one prefix past the whole budget is never parked"
        );
    }

    #[test]
    fn a_prefix_exactly_the_budget_is_parked() {
        let mut stash = Stash::new(10);
        stash.put(1u8, parked_holding("http://a/x", 10));
        assert_eq!(stash.len(), 1, "the budget is a ceiling, not a limit");
        assert!(stash.take(&1, "http://a/x", None).is_some());
    }

    #[test]
    fn prefixes_that_together_fill_the_budget_exactly_are_both_parked() {
        let mut stash = Stash::new(10);
        stash.put(1u8, parked_holding("http://a/x", 5));
        stash.put(2u8, parked_holding("http://a/y", 5));
        assert_eq!(stash.len(), 2);
        assert!(stash.take(&1, "http://a/x", None).is_some());
        assert!(stash.take(&2, "http://a/y", None).is_some());
    }

    #[test]
    fn a_prefix_that_would_overrun_the_budget_evicts_the_oldest() {
        let mut stash = Stash::new(10);
        stash.put(1u8, parked_holding("http://a/x", 5));
        stash.put(2u8, parked_holding("http://a/y", 5));
        stash.put(3u8, parked_holding("http://a/z", 6));
        assert_eq!(stash.len(), 1, "only the newest fits");
        assert!(stash.take(&1, "http://a/x", None).is_none());
        assert!(stash.take(&2, "http://a/y", None).is_none());
        assert!(stash.take(&3, "http://a/z", None).is_some());
    }

    #[test]
    fn clear_drops_every_parked_prefix() {
        let mut stash = Stash::new(10);
        stash.put(1u8, parked_holding("http://a/x", 5));
        stash.clear();
        assert_eq!(stash.len(), 0);
        assert!(stash.take(&1, "http://a/x", None).is_none());
    }

    fn modified_basis() -> Basis {
        Basis::of(
            StatusCode::OK,
            &h(&[
                ("Last-Modified", "Tue, 15 Nov 1994 12:45:26 GMT"),
                ("Date", "Tue, 15 Nov 1994 12:46:00 GMT"),
                ("Accept-Ranges", "bytes"),
                ("Content-Length", "100"),
            ]),
        )
        .unwrap()
    }

    fn resumed_partial(
        basis: Basis,
        headers: &[(&str, &str)],
        received: u64,
    ) -> (HeadOutcome, BodyAcc) {
        let mut basis = Some(basis);
        let mut body = BodyAcc::new(1024);
        body.push(&vec![0u8; usize::try_from(received).unwrap()])
            .unwrap();
        let mut from = Some(received);
        let head = accept_head(
            &mut body,
            &mut basis,
            &mut from,
            StatusCode::PARTIAL_CONTENT,
            &h(headers),
            None,
        );
        (head, body)
    }

    #[test]
    fn a_206_changing_the_last_modified_validator_is_a_mismatch() {
        let (head, _) = resumed_partial(
            modified_basis(),
            &[
                ("Last-Modified", "Tue, 15 Nov 1994 12:45:27 GMT"),
                ("Content-Range", "bytes 40-99/100"),
            ],
            40,
        );
        assert_eq!(head, HeadOutcome::Fallback);
    }

    #[test]
    fn a_206_keeping_the_last_modified_validator_continues() {
        let (head, _) = resumed_partial(
            modified_basis(),
            &[
                ("Last-Modified", "Tue, 15 Nov 1994 12:45:26 GMT"),
                ("Content-Range", "bytes 40-99/100"),
            ],
            40,
        );
        assert_eq!(head, HeadOutcome::Accepted);
    }
}
