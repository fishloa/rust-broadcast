//! One byte range the workspace's pull clients carry on their request
//! actions, so HLS, DASH and Smooth describe the same thing in the same shape
//! (`docs/API-CONVENTIONS.md`, "Byte ranges: one `ByteSpan`, never `(u64, u64)`").
//!
//! A [`ByteSpan`] is the inclusive-exclusive half-open range
//! `[offset, offset + len)`, rendered as the `Range: bytes=<first>-<last>`
//! header by [`ByteSpan::range_header`]. The format-specific HLS and DASH
//! `ByteRange` model types convert to and from it at the parser edge only
//! (their `From` impls live in `broadcast-hls` and `broadcast-dash`); the
//! engines themselves never carry a raw `(u64, u64)` tuple with an
//! undocumented convention.

use core::num::NonZeroU64;
use core::fmt;

use alloc::string::String;

/// A byte sub-range of a resource: `offset` for `len` bytes
/// (`[offset, offset + len)`), what a `Range: bytes=first-last` request names.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByteSpan {
    /// First byte of the range, from the start of the resource (inclusive).
    pub offset: u64,
    /// Number of bytes in the range; always at least one, so the range is
    /// never empty.
    pub len: NonZeroU64,
}

impl ByteSpan {
    /// A range of `len` bytes starting at `offset`. `len` must be at least
    /// one.
    #[must_use]
    pub fn new(offset: u64, len: NonZeroU64) -> Self {
        ByteSpan { offset, len }
    }

    /// The inclusive last byte position: `offset + len - 1`.
    #[must_use]
    pub fn last(&self) -> u64 {
        self.offset + self.len.get() - 1
    }

    /// The `Range: bytes=<first>-<last>` field value for an HTTP range
    /// request (RFC 9110 §14.2), first and last inclusive.
    #[must_use]
    pub fn range_header(&self) -> String {
        alloc::format!("bytes={}-{}", self.offset, self.last())
    }
}

impl fmt::Display for ByteSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.offset, self.last())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_header_is_first_last_inclusive() {
        let span = ByteSpan::new(0, NonZeroU64::new(10).expect("nonzero"));
        assert_eq!(span.range_header(), "bytes=0-9");
        assert_eq!(span.to_string(), "0-9");
    }

    #[test]
    fn offset_and_length_survive_unaltered() {
        let span = ByteSpan::new(500, NonZeroU64::new(100).expect("nonzero"));
        assert_eq!(span.offset, 500);
        assert_eq!(span.len.get(), 100);
        assert_eq!(span.last(), 599);
        assert_eq!(span.range_header(), "bytes=500-599");
    }
}
