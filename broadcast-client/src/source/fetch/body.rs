//! A response body accumulated under a size cap, with the byte-range rules of a ranged request.
//!
//! A request carrying a byte range must be answered with `206` and a `Content-Range` naming
//! exactly that range, or with a `200` (the server ignored `Range`), from which the range is
//! sliced out once the whole body arrived (the cap applies to the whole file). Any other 2xx
//! answer to a ranged request is a failure.

use alloc::vec::Vec;

use headers::{ContentRange, HeaderMapExt};

use super::resume::map_of;

const STATUS_OK: u16 = 200;
const STATUS_PARTIAL_CONTENT: u16 = 206;
const HEADER_CONTENT_LENGTH: &str = "content-length";

/// The body would exceed its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Oversize;

#[derive(Debug)]
pub(crate) struct BodyAcc {
    limit: usize,
    bytes: Vec<u8>,
    expected_len: Option<u64>,
    slice: Option<(usize, usize)>,
}

impl BodyAcc {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: Vec::new(),
            expected_len: None,
            slice: None,
        }
    }

    /// Forgets everything received so the same request can be sent again.
    pub(crate) fn reset(&mut self) {
        self.bytes.clear();
        self.expected_len = None;
        self.slice = None;
    }

    /// Takes the response head of a 2xx answer: `false` when `byte_range` was requested and the
    /// answer does not honour it. Records the declared `Content-Length` for the truncation check.
    pub(crate) fn accept(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        byte_range: Option<(u64, u64)>,
    ) -> bool {
        if !self.range_honoured(status, headers, byte_range) {
            return false;
        }
        self.expected_len = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(HEADER_CONTENT_LENGTH))
            .and_then(|(_, v)| v.trim().parse::<u64>().ok())
            .or_else(|| {
                (status == STATUS_PARTIAL_CONTENT)
                    .then(|| content_range_len(headers))
                    .flatten()
            });
        true
    }

    fn range_honoured(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        byte_range: Option<(u64, u64)>,
    ) -> bool {
        let Some((start, len)) = byte_range else {
            return true;
        };
        match status {
            STATUS_PARTIAL_CONTENT => content_range_is(headers, start, len),
            STATUS_OK => {
                let bounds = usize::try_from(start)
                    .ok()
                    .zip(usize::try_from(len).ok())
                    .filter(|(s, l)| s.checked_add(*l).is_some());
                self.slice = bounds;
                bounds.is_some()
            }
            _ => false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Declares how many bytes the rest of a resumed transfer carries, so the truncation check
    /// covers the prefix kept from the earlier attempt plus the suffix.
    pub(crate) fn expect_suffix(&mut self, suffix_len: Option<u64>) {
        let kept = u64::try_from(self.bytes.len()).unwrap_or(u64::MAX);
        self.expected_len = suffix_len.map(|n| n.saturating_add(kept));
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), Oversize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(Oversize);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// The transfer ended short of the declared `Content-Length`.
    pub(crate) fn is_truncated(&self) -> bool {
        self.expected_len
            .is_some_and(|n| n != u64::try_from(self.bytes.len()).unwrap_or(u64::MAX))
    }

    /// Cuts the requested range out of a whole-file `200` body: `false` when the body is too short
    /// to hold it. A body that was not ranged is left as it is.
    pub(crate) fn finish(&mut self) -> bool {
        let Some((start, len)) = self.slice else {
            return true;
        };
        let Some(end) = start
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
        else {
            return false;
        };
        self.bytes.truncate(end);
        self.bytes.drain(..start);
        true
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

pub(crate) fn content_range_bytes(headers: &[(String, String)]) -> Option<(u64, u64)> {
    map_of(headers)
        .typed_get::<ContentRange>()?
        .bytes_range()
        .filter(|(first, last)| first <= last)
}

pub(crate) fn content_range_len(headers: &[(String, String)]) -> Option<u64> {
    let (first, last) = content_range_bytes(headers)?;
    (last - first).checked_add(1)
}

pub(crate) fn content_range_is(headers: &[(String, String)], start: u64, len: u64) -> bool {
    let Some(last) = len.checked_sub(1).and_then(|l| start.checked_add(l)) else {
        return false;
    };
    content_range_bytes(headers) == Some((start, last))
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

    #[test]
    fn a_body_may_reach_its_cap_exactly_and_no_further() {
        let mut body = BodyAcc::new(4);
        assert_eq!(body.push(b"ab"), Ok(()));
        assert_eq!(body.push(b"cd"), Ok(()));
        assert_eq!(body.push(b"e"), Err(Oversize));
        assert_eq!(body.as_slice(), b"abcd", "a refused chunk is not kept");
    }

    #[test]
    fn one_chunk_over_the_cap_is_refused_whole() {
        let mut body = BodyAcc::new(3);
        assert_eq!(body.push(b"abcd"), Err(Oversize));
        assert!(body.as_slice().is_empty());
    }

    #[test]
    fn the_cap_check_does_not_overflow() {
        let mut body = BodyAcc::new(usize::MAX);
        assert_eq!(body.push(b"a"), Ok(()));
        assert_eq!(body.push(b"b"), Ok(()));
    }

    #[test]
    fn an_unranged_request_accepts_any_2xx_and_keeps_every_byte() {
        for status in [200, 201, 204, 206] {
            let mut body = BodyAcc::new(16);
            assert!(body.accept(status, &[], None), "{status}");
            body.push(b"abcdef").unwrap();
            assert!(body.finish());
            assert_eq!(body.as_slice(), b"abcdef");
        }
    }

    #[test]
    fn a_206_must_name_exactly_the_requested_range() {
        let range = Some((500, 1000));
        let ok = h(&[("Content-Range", "bytes 500-1499/9000")]);
        assert!(BodyAcc::new(1).accept(206, &ok, range));
        for wrong in [
            "bytes 500-1498/9000",
            "bytes 500-1500/9000",
            "bytes 499-1499/9000",
            "bytes 501-1499/9000",
            "items 500-1499/9000",
            "bytes 500-1499",
            "bytes */9000",
            "",
        ] {
            assert!(
                !BodyAcc::new(1).accept(206, &h(&[("Content-Range", wrong)]), range),
                "{wrong}"
            );
        }
        assert!(
            !BodyAcc::new(1).accept(206, &[], range),
            "no Content-Range at all"
        );
    }

    #[test]
    fn a_206_without_content_length_is_truncated_by_its_content_range() {
        let mut body = BodyAcc::new(2_000);
        assert!(body.accept(
            206,
            &h(&[("Content-Range", "bytes 500-599/9000")]),
            Some((500, 100))
        ));
        body.push(&[0u8; 99]).unwrap();
        assert!(body.is_truncated());
        body.push(&[0u8; 1]).unwrap();
        assert!(!body.is_truncated());
    }

    #[test]
    fn a_ranged_request_rejects_every_other_2xx() {
        let range = Some((0, 10));
        for status in [201, 202, 204, 205] {
            assert!(!BodyAcc::new(1).accept(status, &[], range), "{status}");
        }
    }

    #[test]
    fn a_200_to_a_ranged_request_is_sliced_once_the_whole_file_arrived() {
        let mut body = BodyAcc::new(64);
        assert!(body.accept(200, &[], Some((3, 4))));
        body.push(b"0123").unwrap();
        body.push(b"456789").unwrap();
        assert!(body.finish());
        assert_eq!(body.as_slice(), b"3456");
    }

    #[test]
    fn a_200_body_that_ends_inside_the_range_fails() {
        let mut body = BodyAcc::new(64);
        assert!(body.accept(200, &[], Some((3, 4))));
        body.push(b"012345").unwrap();
        assert!(!body.finish(), "bytes 3..7 do not fit in 6");
    }

    #[test]
    fn a_200_body_that_ends_exactly_at_the_range_end_is_sliced() {
        let mut body = BodyAcc::new(64);
        assert!(body.accept(200, &[], Some((3, 4))));
        body.push(b"0123456").unwrap();
        assert!(body.finish());
        assert_eq!(body.as_slice(), b"3456");
    }

    #[test]
    fn the_cap_applies_to_the_whole_file_not_the_slice() {
        let mut body = BodyAcc::new(5);
        assert!(body.accept(200, &[], Some((0, 2))));
        assert_eq!(body.push(b"0123456"), Err(Oversize));
    }

    #[test]
    fn a_range_that_cannot_be_addressed_is_refused() {
        assert!(!BodyAcc::new(1).accept(200, &[], Some((u64::MAX, 1))));
        assert!(!BodyAcc::new(1).accept(200, &[], Some((u64::MAX - 1, u64::MAX))));
    }

    #[test]
    fn a_zero_length_range_is_never_honoured_by_a_206() {
        let ok = h(&[("Content-Range", "bytes 5-4/9")]);
        assert!(!BodyAcc::new(1).accept(206, &ok, Some((5, 0))));
    }

    #[test]
    fn a_body_shorter_than_its_content_length_is_truncated() {
        let mut body = BodyAcc::new(16);
        assert!(body.accept(200, &h(&[("Content-Length", " 6 ")]), None));
        body.push(b"abcde").unwrap();
        assert!(body.is_truncated());
        body.push(b"f").unwrap();
        assert!(!body.is_truncated());
    }

    #[test]
    fn a_body_longer_than_its_content_length_is_truncated_too() {
        let mut body = BodyAcc::new(16);
        assert!(body.accept(200, &h(&[("content-length", "2")]), None));
        body.push(b"abc").unwrap();
        assert!(body.is_truncated());
    }

    #[test]
    fn no_or_unparseable_content_length_is_never_truncated() {
        for headers in [h(&[]), h(&[("Content-Length", "many")])] {
            let mut body = BodyAcc::new(16);
            assert!(body.accept(200, &headers, None));
            body.push(b"abc").unwrap();
            assert!(!body.is_truncated());
        }
    }

    #[test]
    fn reset_forgets_bytes_length_and_range() {
        let mut body = BodyAcc::new(16);
        assert!(body.accept(200, &h(&[("Content-Length", "9")]), Some((1, 2))));
        body.push(b"abc").unwrap();
        body.reset();
        assert!(body.as_slice().is_empty());
        assert!(!body.is_truncated());
        assert!(body.finish(), "no range remembered");
    }

    #[test]
    fn into_bytes_returns_what_was_accepted() {
        let mut body = BodyAcc::new(16);
        body.push(b"xyz").unwrap();
        assert_eq!(body.into_bytes(), b"xyz");
    }
}
