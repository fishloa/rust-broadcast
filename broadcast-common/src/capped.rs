//! Bounded whole-unit reads for the streaming-first drivers
//! (`docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md`
//! §4.5): a download whose length is not known until every byte has arrived
//! must never allocate past the caller's cap. [`read_capped`] reads at most
//! one byte over the limit, so the cap is enforced before the data is held,
//! and reports [`Error::TooLarge`] when the input does not fit.

use std::io::Read;
use std::path::Path;

use alloc::vec::Vec;

/// One extra byte read past the limit, proving the input does not fit without
/// ever holding more than `limit + 1` bytes.
const PROBE_BYTES: u64 = 1;

/// A whole-unit read exceeded the caller's byte cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{what} exceeds the configured limit of {limit} bytes")]
pub struct TooLarge {
    /// Name of the unit being read.
    pub what: &'static str,
    /// The cap it exceeded, in bytes.
    pub limit: u64,
}

/// Failure of a bounded whole-unit read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The underlying reader failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The input was longer than the cap.
    #[error(transparent)]
    TooLarge(#[from] TooLarge),
}

/// Read `reader` to end into one buffer, failing with [`TooLarge`] when it
/// holds more than `limit` bytes.
///
/// The reader is wrapped in [`Read::take`] for `limit + 1` bytes, so the
/// buffer itself is bounded by the cap even for an endless input.
pub fn read_capped<R: Read + ?Sized>(
    reader: &mut R,
    limit: u64,
    what: &'static str,
) -> Result<Vec<u8>, Error> {
    let probe = limit.saturating_add(PROBE_BYTES);
    let mut out = Vec::new();
    reader.take(probe).read_to_end(&mut out)?;
    if u64::try_from(out.len()).unwrap_or(u64::MAX) > limit {
        return Err(Error::TooLarge(TooLarge { what, limit }));
    }
    Ok(out)
}

/// Read a whole file into one buffer, failing with [`TooLarge`] when it holds
/// more than `limit` bytes.
pub fn read_file_capped(path: &Path, limit: u64, what: &'static str) -> Result<Vec<u8>, Error> {
    let mut file = std::fs::File::open(path)?;
    read_capped(&mut file, limit, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn a_short_input_is_returned_unchanged() {
        let mut input = Cursor::new(alloc::vec![1u8, 2, 3]);
        assert_eq!(
            read_capped(&mut input, 8, "fixture").ok(),
            Some(alloc::vec![1u8, 2, 3])
        );
    }

    #[test]
    fn exactly_at_the_limit_succeeds() {
        let mut input = Cursor::new(alloc::vec![7u8; 4]);
        assert_eq!(
            read_capped(&mut input, 4, "fixture").ok(),
            Some(alloc::vec![7u8; 4])
        );
    }

    #[test]
    fn one_byte_over_the_limit_is_too_large() {
        let mut input = Cursor::new(alloc::vec![7u8; 5]);
        assert!(matches!(
            read_capped(&mut input, 4, "fixture"),
            Err(Error::TooLarge(TooLarge {
                what: "fixture",
                limit: 4,
            }))
        ));
    }

    struct Counting {
        inner: Cursor<Vec<u8>>,
        read: usize,
    }

    impl Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read += n;
            Ok(n)
        }
    }

    #[test]
    fn the_reader_stops_one_byte_past_the_limit() {
        let mut counting = Counting {
            inner: Cursor::new(alloc::vec![0u8; 4096]),
            read: 0,
        };
        assert!(read_capped(&mut counting, 16, "fixture").is_err());
        assert_eq!(counting.read, 17);
    }

    #[test]
    fn the_file_variant_uses_the_same_cap() {
        let path = std::env::temp_dir().join(alloc::format!(
            "broadcast-common-capped-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, [0u8; 8]).expect("write fixture");
        let short = read_file_capped(&path, 16, "file");
        let long = read_file_capped(&path, 4, "file");
        std::fs::remove_file(&path).expect("remove fixture");
        assert_eq!(short.ok().map(|bytes| bytes.len()), Some(8));
        assert!(matches!(
            long,
            Err(Error::TooLarge(TooLarge {
                what: "file",
                limit: 4,
            }))
        ));
    }
}
