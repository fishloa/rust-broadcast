//! Canonical `Parse` and `Serialize` traits for the DVB crate family.
//!
//! Each implementer picks its own error type via `type Error`, so
//! domain-specific error variants stay visible to the caller.

use alloc::vec::Vec;

/// Parse a DVB structure from raw bytes. Borrowing allowed via `<'a>`; the
/// concrete error type is chosen per implementer.
pub trait Parse<'a>: Sized {
    /// The error type this implementer returns. Typically the enclosing
    /// crate's `Error` enum.
    type Error;

    /// Parse `bytes` as `Self`. Returns `Err(Self::Error)` on any protocol
    /// violation or buffer underrun.
    fn parse(bytes: &'a [u8]) -> Result<Self, Self::Error>;
}

/// Serialize a DVB structure back to bytes. Split from [`Parse`] so owned
/// and borrowed variants of the same type can implement `Serialize`
/// without carrying a lifetime.
pub trait Serialize {
    /// The error type this implementer returns (usually the same as the
    /// corresponding [`Parse`] impl, but need not be).
    type Error;

    /// Number of bytes `serialize_into` will write.
    fn serialized_len(&self) -> usize;

    /// Write the serialised form into `buf`. Returns the number of bytes
    /// written (always equal to `serialized_len()`).
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Convenience: allocate a `Vec` and serialise into it.
    ///
    /// # Panics
    /// Panics if `serialize_into` returns an error on a buffer of exactly
    /// `serialized_len()` bytes. For values obtained by parsing real wire data
    /// this never happens; it can only occur for a **hand-constructed** value
    /// that violates a wire constraint (e.g. a section whose body exceeds the
    /// 12-bit `section_length`). When building values by hand and that's a
    /// possibility, call [`serialize_into`](Self::serialize_into) and handle the
    /// error instead.
    fn to_bytes(&self) -> Vec<u8>
    where
        Self::Error: core::fmt::Debug,
    {
        let mut v = alloc::vec![0u8; self.serialized_len()];
        self.serialize_into(&mut v)
            .expect("serialize_into must succeed when buffer is exactly serialized_len()");
        v
    }

    /// Allocate a `Vec` and serialize into it, returning the serializer's error instead of
    /// panicking. Prefer this over [`to_bytes`](Self::to_bytes) for any value that was not
    /// obtained by parsing (hand-built values can violate a wire constraint).
    fn try_to_bytes(&self) -> Result<Vec<u8>, Self::Error> {
        let mut v = alloc::vec![0u8; self.serialized_len()];
        let written = self.serialize_into(&mut v)?;
        // A serializer that writes fewer bytes than it promised must not
        // leave trailing zeros in the result.
        if written != v.len() {
            v.truncate(written);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt;

    #[derive(Debug, PartialEq)]
    struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("test serializer error")
        }
    }

    /// Minimal `Serialize` impl whose `serialize_into` fails when `fail` is set.
    struct Flagged {
        fail: bool,
        written: usize,
    }

    impl Serialize for Flagged {
        type Error = TestError;

        fn serialized_len(&self) -> usize {
            4
        }

        fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if self.fail {
                return Err(TestError);
            }
            let n = self.written.min(buf.len());
            buf[..n].copy_from_slice(&[1, 2, 3, 4][..n]);
            Ok(n)
        }
    }

    #[test]
    fn try_to_bytes_returns_serializer_error() {
        assert_eq!(
            Flagged {
                fail: true,
                written: 0
            }
            .try_to_bytes(),
            Err(TestError)
        );
    }

    #[test]
    #[should_panic]
    fn to_bytes_panics_where_try_to_bytes_errors() {
        let _ = Flagged {
            fail: true,
            written: 0,
        }
        .to_bytes();
    }

    #[test]
    fn try_to_bytes_truncates_when_serializer_writes_less_than_promised() {
        // serialized_len is 4 but only 3 bytes are written: no trailing zero.
        let v = Flagged {
            fail: false,
            written: 3,
        }
        .try_to_bytes()
        .unwrap();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn try_to_bytes_happy_path_matches_to_bytes() {
        let ok = Flagged {
            fail: false,
            written: 4,
        };
        assert_eq!(ok.try_to_bytes().unwrap(), ok.to_bytes());
    }
}
