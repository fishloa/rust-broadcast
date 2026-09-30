//! HLS playlist validator (RFC 8216) — free-function `check_playlist`.
//!
//! A thin, retained entry point over [`crate::check_hls_playlist`]. This
//! module used to carry a near-verbatim second copy of that function's
//! line-based rules — same inputs, same intent, already drifted (the rule
//! messages differed between the two, audit MD-W10). One implementation now
//! serves both, so a rule can only be fixed once.
//!
//! # Rule IDs
//!
//! | ID | Severity | Description | Clause |
//! |---|---|---|---|
//! | `hls-parse-error` | Error | Playlist fails to parse as valid HLS | §4 |
//! | `hls-missing-extm3u` | Error | First non-empty line is not `#EXTM3U` | §4.4.1.1 |
//! | `hls-missing-targetduration` | Error | Media playlist with segments lacks TARGETDURATION | §4.4.3.1 |
//! | `hls-extinf-exceeds-target` | Error | EXTINF duration exceeds TARGETDURATION | §4.4.3.1 |
//! | `hls-part-duration-range` | Error | Part duration above PART-TARGET, or below 85% of it | §4.4.4.9 |
//! | `hls-preload-hint-with-endlist` | Error | PRELOAD-HINT in a playlist with ENDLIST | §4.4.5.3 |
//! | `hls-skip-without-can-skip-until` | Error | EXT-X-SKIP without CAN-SKIP-UNTIL in SERVER-CONTROL | §4.4.5.2, §4.4.3.8 |
//! | `hls-malformed-daterange` | Error | DATERANGE line fails `DateRange::parse_tag_line` | §4.4.5.1 |

use crate::report::Report;

/// Validate an HLS playlist, appending findings for each violation.
///
/// Equivalent to [`crate::check_hls_playlist`] — this name is kept for callers
/// that predate it. Line numbers in [`Location`](crate::Location) are 1-based;
/// `pid` is always 0 (text input).
pub fn check_playlist(text: &str, report: &mut Report) {
    crate::check_hls_playlist(text, report);
}
