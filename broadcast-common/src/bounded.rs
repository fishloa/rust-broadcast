//! Bounded whole-document reads.
//!
//! Every small document the workspace reads whole (a config, a playlist, an
//! MPD, an SDP, a key file, one wire structure in an example) goes through
//! here with an explicit [`Limit`], so a wrong or hostile path fails with a
//! typed [`BoundedReadError::TooLarge`] naming the limit instead of exhausting
//! memory.
//!
//! A media capture never goes through this module: a tool or example that
//! takes one streams it through the matching container stage or scanner in
//! bounded chunks.

use std::io::Read;
use std::path::Path;

use alloc::string::String;
use alloc::vec::Vec;

/// Bytes read past the limit, proving the input does not fit without ever
/// holding more than `limit + 1` bytes.
const PROBE_BYTES: u64 = 1;

/// A named byte ceiling for one whole-document read.
///
/// There is no `Default` and no unlimited constructor: a caller must say what
/// the document is and how large it may be, and the error it can get back
/// names exactly that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    name: &'static str,
    bytes: u64,
}

impl Limit {
    /// A `name`d ceiling of `bytes`.
    pub const fn new(name: &'static str, bytes: u64) -> Self {
        Self { name, bytes }
    }

    /// The name reported in [`BoundedReadError::TooLarge`].
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The ceiling, in bytes.
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Failure of a bounded whole-document read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BoundedReadError {
    /// The document was longer than its [`Limit`].
    #[error("{limit_name} exceeds the configured limit of {limit_bytes} bytes")]
    TooLarge {
        /// Name of the document being read.
        limit_name: &'static str,
        /// The ceiling it exceeded, in bytes.
        limit_bytes: u64,
    },
    /// The underlying reader failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The bytes were not valid UTF-8.
    #[error("the document is not valid UTF-8")]
    NotUtf8,
}

/// Read `reader` to end into one buffer, failing when it holds more than
/// `limit` bytes.
///
/// The reader is wrapped in [`Read::take`] for `limit + 1` bytes, so an
/// endless input stops after one byte past the ceiling rather than growing the
/// buffer without bound. File metadata is never consulted.
pub fn read_bounded<R: Read>(reader: R, limit: Limit) -> Result<Vec<u8>, BoundedReadError> {
    let probe = limit.bytes.saturating_add(PROBE_BYTES);
    let mut out = Vec::new();
    reader.take(probe).read_to_end(&mut out)?;
    let read = u64::try_from(out.len()).unwrap_or(u64::MAX);
    if read > limit.bytes {
        return Err(BoundedReadError::TooLarge {
            limit_name: limit.name,
            limit_bytes: limit.bytes,
        });
    }
    Ok(out)
}

/// Read a whole file into one buffer, failing when it holds more than `limit`
/// bytes.
pub fn read_file_bounded(path: &Path, limit: Limit) -> Result<Vec<u8>, BoundedReadError> {
    read_bounded(std::fs::File::open(path)?, limit)
}

/// Read a whole file into one string, failing when it holds more than `limit`
/// bytes or is not valid UTF-8.
pub fn read_file_bounded_string(path: &Path, limit: Limit) -> Result<String, BoundedReadError> {
    let bytes = read_file_bounded(path, limit)?;
    String::from_utf8(bytes).map_err(|_| BoundedReadError::NotUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const LIMIT: Limit = Limit::new("fixture", 4);

    #[test]
    fn a_short_document_is_returned_unchanged() {
        let mut input = Cursor::new(alloc::vec![1u8, 2, 3]);
        assert_eq!(
            read_bounded(&mut input, LIMIT).ok(),
            Some(alloc::vec![1u8, 2, 3])
        );
    }

    #[test]
    fn exactly_at_the_limit_succeeds() {
        let mut input = Cursor::new(alloc::vec![7u8; 4]);
        assert_eq!(
            read_bounded(&mut input, LIMIT).ok(),
            Some(alloc::vec![7u8; 4])
        );
    }

    #[test]
    fn one_byte_over_the_limit_is_too_large() {
        let mut input = Cursor::new(alloc::vec![7u8; 5]);
        assert!(matches!(
            read_bounded(&mut input, LIMIT),
            Err(BoundedReadError::TooLarge {
                limit_name: "fixture",
                limit_bytes: 4,
            })
        ));
    }

    /// A source whose declared length is enormous but which yields bytes on
    /// demand, so trusting the length instead of reading would allocate it all.
    struct Endless {
        remaining: u64,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let want = u64::try_from(buf.len()).unwrap_or(u64::MAX);
            let count = usize::try_from(self.remaining.min(want)).unwrap_or(usize::MAX);
            buf[..count].fill(0);
            self.remaining = self.remaining.saturating_sub(count as u64);
            Ok(count)
        }
    }

    #[test]
    fn a_lying_length_does_not_grow_the_buffer_past_the_limit() {
        let mut input = Endless {
            remaining: 4 * 1024 * 1024 * 1024,
        };
        assert!(matches!(
            read_bounded(&mut input, LIMIT),
            Err(BoundedReadError::TooLarge {
                limit_name: "fixture",
                limit_bytes: 4,
            })
        ));
    }

    #[test]
    fn invalid_utf8_is_reported() {
        let path = std::env::temp_dir().join(alloc::format!(
            "broadcast-common-bounded-utf8-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, [0xFFu8, 0xFE]).expect("write fixture");
        let result = read_file_bounded_string(&path, LIMIT);
        std::fs::remove_file(&path).expect("remove fixture");
        assert!(matches!(result, Err(BoundedReadError::NotUtf8)));
    }

    #[test]
    fn the_file_variant_uses_the_same_cap() {
        let path = std::env::temp_dir().join(alloc::format!(
            "broadcast-common-bounded-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, [0u8; 8]).expect("write fixture");
        let short = read_file_bounded(&path, Limit::new("file", 16));
        let long = read_file_bounded(&path, Limit::new("file", 4));
        std::fs::remove_file(&path).expect("remove fixture");
        assert_eq!(short.ok().map(|bytes| bytes.len()), Some(8));
        assert!(matches!(
            long,
            Err(BoundedReadError::TooLarge {
                limit_name: "file",
                limit_bytes: 4,
            })
        ));
    }

    #[test]
    fn the_file_string_variant_reads_text() {
        let path = std::env::temp_dir().join(alloc::format!(
            "broadcast-common-bounded-text-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::write(&path, "hello").expect("write fixture");
        let text = read_file_bounded_string(&path, Limit::new("text", 16));
        std::fs::remove_file(&path).expect("remove fixture");
        assert_eq!(text.ok().as_deref(), Some("hello"));
    }
}
