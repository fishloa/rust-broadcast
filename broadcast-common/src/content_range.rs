//! `Content-Range` of a `206 Partial Content` answer (RFC 9110 §14.4).
//!
//! `bytes <first>-<last>/<complete-length>` where `complete-length` is a number
//! or `*` (unknown). The unit is a case-insensitive token (§14.1); the grammar
//! allows exactly one space after it and none around `-` or `/`. A range is
//! satisfiable only when `first <= last`, and when the length is known `last`
//! must be below it (§14.4: "a recipient of a Content-Range header field whose
//! last-pos is greater than or equal to complete-length … MUST consider that
//! invalid").
//!
//! The one parser every HTTP client in the workspace uses, `no_std` so the
//! sans-IO policy cores can share it.

/// A parsed `bytes first-last/complete-length` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentRange {
    /// First byte position of the range (inclusive).
    pub first: u64,
    /// Last byte position of the range (inclusive).
    pub last: u64,
    /// Complete length of the representation; `None` for `*` (unknown).
    pub complete_length: Option<u64>,
}

impl ContentRange {
    /// Parse the field value of a satisfied byte range. `None` for any other
    /// unit, a malformed value, `first > last`, a known complete length that
    /// `last` reaches, and the unsatisfied form `bytes */length` (which only
    /// accompanies a `416`).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim_matches(is_ows);
        let (unit, rest) = value.split_once(' ')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }
        let (range, length) = rest.split_once('/')?;
        let (first, last) = range.split_once('-')?;
        let first = digits(first)?;
        let last = digits(last)?;
        let complete_length = match length {
            "*" => None,
            n => Some(digits(n)?),
        };
        if first > last || complete_length.is_some_and(|length| last >= length) {
            return None;
        }
        Some(ContentRange {
            first,
            last,
            complete_length,
        })
    }

    /// The number of bytes in the range.
    #[must_use]
    pub fn len(&self) -> Option<u64> {
        (self.last - self.first).checked_add(1)
    }

    /// Always `false`: a satisfied range holds at least one byte.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }
}

fn is_ows(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_length_range() {
        let r = ContentRange::parse("bytes 0-9/10").unwrap();
        assert_eq!((r.first, r.last, r.complete_length), (0, 9, Some(10)));
        assert_eq!(r.len(), Some(10));
    }

    /// RFC 9110 §14.4: `*` is a valid complete-length.
    #[test]
    fn an_unknown_length_is_valid() {
        let r = ContentRange::parse("bytes 0-9/*").unwrap();
        assert_eq!((r.first, r.last, r.complete_length), (0, 9, None));
    }

    /// RFC 9110 §14.4: last-pos >= complete-length is invalid.
    #[test]
    fn a_last_position_at_or_past_the_length_is_invalid() {
        assert!(ContentRange::parse("bytes 5-20/10").is_none());
        assert!(ContentRange::parse("bytes 0-10/10").is_none());
        assert!(ContentRange::parse("bytes 0-9/10").is_some());
    }

    #[test]
    fn a_reversed_range_the_unsatisfied_form_and_other_units_are_none() {
        for bad in [
            "bytes 9-0/10",
            "bytes */10",
            "items 0-9/10",
            "bytes 0-9",
            "bytes -9/10",
            "bytes 0-/10",
            "bytes 0-9/",
            "bytes +0-9/10",
        ] {
            assert!(ContentRange::parse(bad).is_none(), "{bad}");
        }
    }

    /// The grammar has one space after the unit and none around `-` or `/`.
    #[test]
    fn stray_whitespace_inside_the_value_is_invalid() {
        assert!(ContentRange::parse("bytes  0-9/10").is_none());
        assert!(ContentRange::parse("bytes 0-9 / 10").is_none());
        assert!(ContentRange::parse("bytes 0 - 9/10").is_none());
        assert!(ContentRange::parse(" bytes 0-9/10 ").is_some());
    }

    #[test]
    fn the_unit_is_case_insensitive_and_overflow_is_none() {
        assert!(ContentRange::parse("BYTES 0-9/10").is_some());
        assert!(ContentRange::parse("bytes 0-18446744073709551616/*").is_none());
        assert_eq!(
            ContentRange::parse("bytes 0-18446744073709551615/*")
                .unwrap()
                .len(),
            None
        );
    }
}
