//! Bounded header-framing check shared by the client core and the server codec.
//!
//! This is NOT protocol parsing: it only finds where a header block ends (the
//! blank line) so that (a) the 64 KiB header cap can be enforced on the bytes
//! buffered *before* the terminator, whatever shape they have, and (b)
//! `rtsp_types::Message::parse` runs once per message instead of once per
//! read. All header and body interpretation stays in `rtsp_types`.

use memchr::memmem;

use crate::error::{Error, MessageKind, Result};
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
    let crlf = memmem::find(window, b"\r\n\r\n").map(|i| from + i + b"\r\n\r\n".len());
    let lf = memmem::find(window, b"\n\n").map(|i| from + i + b"\n\n".len());
    let end = match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let head_len = end.unwrap_or(buf.len());
    if head_len > MAX_HEAD_BYTES {
        return Err(Error::MessageParse {
            what: MessageKind::Framing,
            source: crate::error::MessageParseError::HeaderUnterminated {
                at_least: head_len,
                limit: MAX_HEAD_BYTES,
            },
        });
    }
    Ok(end)
}

/// The version token every RTSP start line carries (`RTSP/x.y`, RFC 2326
/// §6.1).
const VERSION_PREFIX: &[u8] = b"RTSP/";

/// Longest start-line prefix this check reads before calling the bytes junk:
/// longer than any `METHOD` (RFC 2326 §10).
const MAX_TOKEN: usize = 32;

/// The end of the first line of `buf`, if it holds one.
fn line_end(buf: &[u8]) -> Option<usize> {
    let crlf = memmem::find(buf, b"\r\n");
    let lf = memchr::memchr(b'\n', buf);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Whether the first line names the RTSP version as one of its tokens.
fn line_names_version(line: &[u8]) -> bool {
    line.split(|b| *b == b' ' || *b == b'\t')
        .any(|token| token.starts_with(VERSION_PREFIX))
}

/// Whether `buf` begins with something that can be an RTSP start line
/// (`METHOD SP ... RTSP/x.y` or `RTSP/x.y SP code`).
///
/// `Some(false)` is the resynchronisation signal: a `$`-frame stream may carry
/// junk where a message head was expected, and a player scans forward to the
/// next frame instead of giving up. `None` means "not decided yet" — the
/// buffer ends inside the start line, so the caller waits for more bytes.
pub(crate) fn start_line_is_message(buf: &[u8]) -> Option<bool> {
    if let Some(end) = line_end(buf) {
        return Some(line_names_version(&buf[..end]));
    }
    for byte in buf.iter().take(MAX_TOKEN) {
        match *byte {
            b' ' | b'\t' => return Some(true),
            b'\r' | b'\n' => return Some(false),
            _ if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'/' | b'.') => {}
            _ => return Some(false),
        }
    }
    // A start line this long with neither a space nor a line end cannot be
    // one: every method is followed by a space well inside this bound.
    if buf.len() < MAX_TOKEN {
        None
    } else {
        Some(false)
    }
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
        assert!(matches!(
            head_end(&a, 0),
            Err(crate::error::Error::MessageParse { .. })
        ));
        let lines = b"X-A: b\r\n".repeat(9000);
        assert!(matches!(
            head_end(&lines, 0),
            Err(crate::error::Error::MessageParse { .. })
        ));
        assert!(head_end(&a[..1000], 0).unwrap().is_none());
    }

    #[test]
    fn a_chunked_head_is_found_whatever_the_chunk_boundary() {
        let buf = b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\nbody";
        let end = buf.len() - 4;
        for chunk in 1..=buf.len() {
            let mut scanned = 0;
            let mut found = None;
            let mut upto = 0;
            while upto < buf.len() {
                upto = (upto + chunk).min(buf.len());
                match head_end(&buf[..upto], scanned) {
                    Ok(Some(end)) => {
                        found = Some(end);
                        break;
                    }
                    Ok(None) => scanned = upto,
                    Err(e) => panic!("chunk {chunk} at {upto}: {e:?}"),
                }
            }
            assert_eq!(found, Some(end), "chunk size {chunk}");
        }
    }

    #[test]
    fn a_start_line_is_recognised_and_junk_is_not() {
        assert_eq!(
            start_line_is_message(b"OPTIONS rtsp://h/s RTSP/1.0\r\n"),
            Some(true)
        );
        assert_eq!(start_line_is_message(b"RTSP/1.0 200 OK\r\n"), Some(true));
        assert_eq!(start_line_is_message(b"garbage!$"), Some(false));
        assert_eq!(start_line_is_message(b"garbage$"), Some(false));
        assert_eq!(start_line_is_message(b"$\x00\x00\x01"), Some(false));
        assert_eq!(
            start_line_is_message(b"GARBAGE\r\n\r\n"),
            Some(false),
            "a token line without the version is junk, not a request"
        );
        assert_eq!(
            start_line_is_message(&vec![b'A'; 70 * 1024]),
            Some(false),
            "a long unterminated run with no separator is junk"
        );
        assert_eq!(start_line_is_message(b"OPT"), None);
        assert_eq!(start_line_is_message(b"RTS"), None);
        assert_eq!(start_line_is_message(b""), None);
    }

    #[test]
    fn the_head_cap_is_64_kib_including_the_terminator() {
        let mut head = Vec::new();
        head.extend_from_slice(b"OPTIONS rtsp://h/s RTSP/1.0\r\nX-Pad: ");
        head.resize(65_536 - 4, b'a');
        head.extend_from_slice(b"\r\n\r\n");
        assert_eq!(head.len(), 65_536);
        assert_eq!(head_end(&head, 0).unwrap(), Some(65_536));

        head.insert(65_536 - 4, b'a');
        assert_eq!(head.len(), 65_537);
        match head_end(&head, 0) {
            Err(crate::error::Error::MessageParse {
                source: crate::error::MessageParseError::HeaderUnterminated { at_least, limit },
                ..
            }) => {
                assert_eq!(at_least, 65_537);
                assert_eq!(limit, 65_536);
            }
            other => panic!("expected HeaderUnterminated, got {other:?}"),
        }
    }
}
