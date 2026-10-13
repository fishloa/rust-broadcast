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

    /// Allocate a `Vec` and serialize into it, returning the serializer's error instead of
    /// panicking. This is the only
    /// allocate-and-serialize entry point: there is deliberately no panicking
    /// `to_bytes` (hand-built values, or values assembled from damaged wire data,
    /// can violate a wire constraint, so the error must reach the caller).
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
    fn try_to_bytes_happy_path_fills_buffer() {
        let ok = Flagged {
            fail: false,
            written: 4,
        };
        assert_eq!(ok.try_to_bytes().unwrap(), vec![1, 2, 3, 4]);
    }
}
