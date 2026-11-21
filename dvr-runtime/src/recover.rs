//! fMP4 period recovery: walk a period media file's top-level boxes and
//! report the init and every `moof`+`mdat` fragment span.
//!
//! This is only for fMP4. A TS period has no recoverable fragment boundary:
//! concatenated 188-byte packets do not name where a segment starts, so a TS
//! period whose playlist is lost stays unusable (documented, not worked
//! around).

use std::num::NonZeroU64;

use broadcast_common::range::ByteSpan;
use transmux::segments::{FTYP, MDAT, MOOF};

/// One recovered span within the period's media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecoveredBox {
    /// The period's single init, from byte 0.
    Init(ByteSpan),
    /// One fragment: its `moof` plus the `mdat` that immediately follows, or
    /// the `moof` alone when no `mdat` does.
    Fragment(ByteSpan),
}

/// Why a recovery scan failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecoverError {
    /// The data is empty or its first top-level box is not ISO BMFF.
    #[error("not an ISO BMFF period file")]
    NotIsoBmff,
    /// No `ftyp`/`moov` precedes the first `moof`, so the init cannot be
    /// found.
    #[error("the period file has no init before its first fragment")]
    MissingInit,
}

fn span(offset: usize, len: usize) -> Option<ByteSpan> {
    let len = NonZeroU64::new(u64::try_from(len).ok()?)?;
    Some(ByteSpan::new(u64::try_from(offset).ok()?, len))
}

/// Scan a whole period media file (init first, then fragments).
///
/// A truncated tail (a power loss or a full disk mid-write) stops the walk at
/// the last complete box: every span found before it is reported, and the
/// function does not fail. A `moof` with no following `mdat` is reported as a
/// fragment by itself.
///
/// # Errors
///
/// [`RecoverError::NotIsoBmff`] when the data does not start with a complete
/// ISO BMFF box; [`RecoverError::MissingInit`] when fragments precede any
/// init.
pub fn scan_fmp4_data(data: &[u8]) -> Result<Vec<RecoveredBox>, RecoverError> {
    if data.is_empty() {
        return Err(RecoverError::NotIsoBmff);
    }
    let mut offset = 0usize;
    let mut init_end: Option<usize> = None;
    let mut saw_init = false;
    let mut fragments = Vec::new();
    let mut first = true;
    while offset < data.len() {
        let Ok((box_ref, consumed)) = transmux::parse_box(&data[offset..]) else {
            if first {
                return Err(RecoverError::NotIsoBmff);
            }
            break;
        };
        if consumed == 0 {
            break;
        }
        first = false;
        let kind = &box_ref.header.box_type;
        if kind.is(&MOOF) {
            if init_end.is_none() {
                init_end = Some(offset);
            }
            let mut total = consumed;
            if let Some(next) = offset.checked_add(consumed)
                && next < data.len()
                && let Ok((next_box, next_len)) = transmux::parse_box(&data[next..])
                && next_box.header.box_type.is(&MDAT)
            {
                total = total.saturating_add(next_len);
            }
            if let Some(span) = span(offset, total) {
                fragments.push(RecoveredBox::Fragment(span));
            }
            offset = offset.saturating_add(total);
            continue;
        }
        if init_end.is_none() && (kind.is(&FTYP) || kind.is(b"moov")) {
            saw_init = true;
        }
        offset = offset.saturating_add(consumed);
    }
    if !saw_init {
        return Err(RecoverError::MissingInit);
    }
    let end = init_end.unwrap_or(data.len());
    let init = span(0, end).ok_or(RecoverError::MissingInit)?;
    let mut recovered = Vec::with_capacity(fragments.len() + 1);
    recovered.push(RecoveredBox::Init(init));
    recovered.extend(fragments);
    Ok(recovered)
}
