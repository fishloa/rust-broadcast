//! Bounded header-framing check shared by the client core and the server codec.
//!
//! This is NOT protocol parsing: it only finds where a header block ends (the
//! blank line) so that (a) the 64 KiB header cap can be enforced on the bytes
//! buffered *before* the terminator, whatever shape they have, and (b)
//! `rtsp_types::Message::parse` runs once per message instead of once per
//! read. All header and body interpretation stays in `rtsp_types`.

use memchr::memmem;

use crate::error::{Error, Result};
use crate::limits::MAX_HEAD_BYTES;

/// Longest terminator (`\r\n\r\n`); a resumed scan reaches back this much minus one.
const TERMINATOR_MAX_LEN: usize = 4;

/// Index just past the blank line ending the header block in `buf`, scanning
/// only bytes not yet scanned (`scanned` = how much of `buf` earlier calls
/// already covered). `Ok(None)` = no terminator yet; the caller stores
/// `buf.len()` as the new `scanned`. Errors when the head exceeds
/// [`MAX_HEAD_BYTES`], terminated or not.
pub(crate) fn head_end(buf: &[u8], scanned: usize) -> Result<Option<usize>> {
    let from = scanned
        .saturating_sub(TERMINATOR_MAX_LEN - 1)
        .min(buf.len());
    let window = &buf[from..];
    let crlf = memmem::find(window, b"\r\n\r\n").map(|i| from + i + 4);
    let lf = memmem::find(window, b"\n\n").map(|i| from + i + 2);
    let end = match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let head_len = end.unwrap_or(buf.len());
    if head_len > MAX_HEAD_BYTES {
        return Err(Error::MessageParse(format!(
            "header block over {MAX_HEAD_BYTES} bytes without a terminator"
        )));
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_crlf_and_lf_terminators_and_resumes_across_a_split() {
        assert_eq!(head_end(b"A: b\r\n\r\nbody", 0).unwrap(), Some(8));
        assert_eq!(head_end(b"A: b\n\nbody", 0).unwrap(), Some(6));
        let buf = b"A: b\r\n\r\n";
        // terminator split across the resume point
        assert_eq!(head_end(&buf[..6], 0).unwrap(), None);
        assert_eq!(head_end(buf, 6).unwrap(), Some(8));
    }

    #[test]
    fn the_cap_applies_to_any_unterminated_shape() {
        let a = vec![b'A'; 70 * 1024];
        assert!(head_end(&a, 0).is_err());
        let lines = b"X-A: b\r\n".repeat(9000);
        assert!(head_end(&lines, 0).is_err());
        assert!(head_end(&a[..1000], 0).unwrap().is_none());
    }
}
