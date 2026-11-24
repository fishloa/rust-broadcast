//! fMP4 period recovery: walk a period media file's top-level boxes and
//! report the init and every `moof`+`mdat` fragment span.
//!
//! The walk is incremental ([`Fmp4Scan`]): it asks the caller for one box
//! header at a time and seeks over every payload, so recovering a period
//! reads a bounded number of bytes however large the file is.
//!
//! This is only for fMP4. A TS period has no recoverable fragment boundary:
//! concatenated 188-byte packets do not name where a segment starts, so a TS
//! period whose playlist is lost stays unusable (documented, not worked
//! around).

use std::num::NonZeroU64;

use broadcast_common::Parse;
use broadcast_common::range::ByteSpan;
use isobmff::box_types::{BOX_HEADER_MIN_SIZE, BoxHeader, LARGESIZE_SIZE, SIZE_TO_EOF};
use isobmff::emsg::EMSG_BOX_TYPE as EMSG;
use isobmff::init_segment::MOOV;
use isobmff::sample_groups::PRFT;
use isobmff::segments::{FTYP, MDAT, MOOF, STYP};

/// Whether a top-level box belongs to the segment a following `moof` opens
/// rather than to the init.
fn leads_a_fragment(kind: &[u8; isobmff::box_types::BOX_TYPE_SIZE]) -> bool {
    kind == &STYP || kind == &EMSG || kind == &PRFT
}

/// One recovered span within the period's media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecoveredBox {
    /// The period's single init, from byte 0.
    Init(ByteSpan),
    /// One segment: the `moof` plus the `mdat` that immediately follows (or
    /// the `moof` alone when no `mdat` does) and any `styp`/`emsg`/`prft` run
    /// directly before it.
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
    /// [`Fmp4Scan::supply`] was called without a pending need, or with a
    /// different offset or length than the one it asked for.
    #[error("the supplied bytes do not answer the pending need")]
    UnexpectedSupply,
    /// The file declares more fragments than the scan accepts, which a real
    /// period never does: a corrupt or crafted file is refused instead of
    /// collected.
    #[error("the period file holds more than {limit} fragments")]
    TooManyFragments {
        /// Most fragments the scan accepts.
        limit: usize,
    },
}

/// Bytes of a box header: 32-bit size, type, 64-bit largesize.
pub const BOX_HEADER_MAX: usize = BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE;

/// Most fragments a scan reports unless [`Fmp4Scan::with_max_fragments`] says
/// otherwise.
///
/// A period holds a bounded number of segments, hours of a few seconds each,
/// so a million is far above any real period. Without the cap a corrupt file
/// made of tiny `moof` boxes could grow the result list until the file's own
/// size allowed: a 512 MiB file holds 64 million 8-byte boxes.
pub const DEFAULT_MAX_FRAGMENTS: usize = 1 << 20;

/// The span at `offset` for `len` bytes, or `None` for an empty one.
fn span(offset: u64, len: u64) -> Option<ByteSpan> {
    NonZeroU64::new(len).map(|len| ByteSpan::new(offset, len))
}

/// One round of [`Fmp4Scan`]: read one box header, or report the spans.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanStep {
    /// Supply `len` bytes at `offset` (always a box header) and call
    /// [`Fmp4Scan::supply`].
    Need {
        /// Byte offset of the header within the period media file.
        offset: u64,
        /// How many bytes to read there, never more than
        /// [`BOX_HEADER_MAX`].
        len: usize,
    },
    /// Every span the file holds, init first.
    ///
    /// The list is handed over: a later [`Fmp4Scan::step`] reports `Done`
    /// with an empty one.
    Done(Vec<RecoveredBox>),
}

/// An incremental scan of a period media file's top-level boxes.
///
/// The scan reads headers only; the caller seeks over the payloads between
/// them. [`step`](Self::step) drives it: it either names the next header to
/// read or, once the file ends, reports the recovered spans.
///
/// A truncated tail (a power loss or a full disk mid-write) ends the scan at
/// the last complete box: every span found before it is reported and the
/// scan does not fail. A `moof` with no following `mdat` is reported as a
/// fragment by itself.
///
/// A fragment begins at the `styp`/`emsg`/`prft` run directly before its
/// `moof`, so a CMAF segment's span is recovered whole. A segment an LL-HLS
/// packager split into several `moof` runs is still reported as one fragment
/// per `moof`.
///
/// The scan reports at most [`DEFAULT_MAX_FRAGMENTS`] fragments, or the cap
/// [`with_max_fragments`](Self::with_max_fragments) sets, and refuses a file
/// that holds more with [`RecoverError::TooManyFragments`].
pub struct Fmp4Scan {
    file_len: u64,
    next: u64,
    pending: Option<(u64, usize)>,
    first: bool,
    saw_init: bool,
    init_end: Option<u64>,
    lead: Option<u64>,
    trailing: Option<(u64, u64)>,
    fragments: Vec<RecoveredBox>,
    max_fragments: usize,
    done: bool,
}

impl Fmp4Scan {
    /// Start scanning a period media file that is `file_len` bytes long.
    #[must_use]
    pub fn new(file_len: u64) -> Self {
        Self {
            file_len,
            next: 0,
            pending: None,
            first: true,
            saw_init: false,
            init_end: None,
            lead: None,
            trailing: None,
            fragments: Vec::new(),
            max_fragments: DEFAULT_MAX_FRAGMENTS,
            done: false,
        }
    }

    /// Report at most `max` fragments, then fail with
    /// [`RecoverError::TooManyFragments`].
    #[must_use]
    pub fn with_max_fragments(mut self, max: usize) -> Self {
        self.max_fragments = max;
        self
    }

    /// The next header to read, or the recovered spans.
    ///
    /// [`ScanStep::Done`] hands the list over, so every later call reports
    /// `Done` with an empty list. The same holds after an error the scan
    /// cannot resume from, which ends it.
    ///
    /// # Errors
    ///
    /// [`RecoverError::NotIsoBmff`] when the file's first box is not a
    /// complete ISO BMFF header, which also covers a file shorter than one
    /// header; [`RecoverError::MissingInit`] when the scan ends without an
    /// `ftyp`/`moov` before any fragment.
    pub fn step(&mut self) -> Result<ScanStep, RecoverError> {
        if self.done {
            return Ok(ScanStep::Done(Vec::new()));
        }
        let remaining = self.file_len.saturating_sub(self.next);
        if remaining < BOX_HEADER_MIN_SIZE as u64 {
            if core::mem::take(&mut self.first) {
                return Err(RecoverError::NotIsoBmff);
            }
            return self.finish();
        }
        let len = usize::try_from(remaining.min(BOX_HEADER_MAX as u64)).unwrap_or(BOX_HEADER_MAX);
        self.pending = Some((self.next, len));
        Ok(ScanStep::Need {
            offset: self.next,
            len,
        })
    }

    /// Answer the pending [`ScanStep::Need`] with the bytes it asked for.
    ///
    /// # Errors
    ///
    /// [`RecoverError::UnexpectedSupply`] when `offset` or `bytes` do not
    /// answer the pending need; [`RecoverError::NotIsoBmff`] when the file's
    /// first box is not a complete ISO BMFF header.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<(), RecoverError> {
        match self.pending {
            Some((asked_offset, len)) if asked_offset == offset && len == bytes.len() => {}
            _ => return Err(RecoverError::UnexpectedSupply),
        }
        self.pending = None;
        let first = core::mem::take(&mut self.first);
        let Ok(header) = BoxHeader::parse(bytes) else {
            if first {
                return Err(RecoverError::NotIsoBmff);
            }
            self.next = self.file_len;
            return Ok(());
        };
        if let Some((start, moof_len)) = self.trailing.take() {
            if header.box_type.is(&MDAT)
                && let Some(total) = self.extent(offset, &header)
            {
                self.push_fragment(start, moof_len.saturating_add(total))?;
                self.next = offset.saturating_add(total);
                return Ok(());
            }
            self.push_fragment(start, moof_len)?;
        }
        self.apply(offset, &header, first)
    }

    fn apply(&mut self, offset: u64, header: &BoxHeader, first: bool) -> Result<(), RecoverError> {
        let Some(total) = self.extent(offset, header) else {
            if first {
                return Err(RecoverError::NotIsoBmff);
            }
            self.next = self.file_len;
            return Ok(());
        };
        let end = offset.saturating_add(total);
        let kind = &header.box_type;
        if kind.is(&MOOF) {
            let start = self.lead.take().unwrap_or(offset);
            self.init_end.get_or_insert(start);
            if end < self.file_len {
                self.trailing = Some((start, end.saturating_sub(start)));
                self.next = end;
            } else {
                self.push_fragment(start, end.saturating_sub(start))?;
                self.next = self.file_len;
            }
            return Ok(());
        }
        if leads_a_fragment(&kind.0) {
            self.lead.get_or_insert(offset);
        } else {
            self.lead = None;
            if self.init_end.is_none() && (kind.is(&FTYP) || kind.is(&MOOV)) {
                self.saw_init = true;
            }
        }
        self.next = end;
        Ok(())
    }

    /// How many bytes the box at `offset` declares, `None` when that runs
    /// past the end of the file (a torn tail).
    fn extent(&self, offset: u64, header: &BoxHeader) -> Option<u64> {
        let remaining = self.file_len.saturating_sub(offset);
        let total = if header.size == u64::from(SIZE_TO_EOF) {
            remaining
        } else {
            header.size
        };
        (total <= remaining).then_some(total)
    }

    fn push_fragment(&mut self, start: u64, len: u64) -> Result<(), RecoverError> {
        let Some(span) = span(start, len) else {
            return Ok(());
        };
        if self.fragments.len() >= self.max_fragments {
            self.done = true;
            return Err(RecoverError::TooManyFragments {
                limit: self.max_fragments,
            });
        }
        self.fragments.push(RecoveredBox::Fragment(span));
        Ok(())
    }

    fn finish(&mut self) -> Result<ScanStep, RecoverError> {
        if let Some((start, len)) = self.trailing.take() {
            self.push_fragment(start, len)?;
        }
        self.done = true;
        if !self.saw_init {
            return Err(RecoverError::MissingInit);
        }
        let end = self.init_end.unwrap_or(self.file_len);
        let Some(init) = span(0, end) else {
            return Err(RecoverError::MissingInit);
        };
        let mut boxes = Vec::with_capacity(self.fragments.len() + 1);
        boxes.push(RecoveredBox::Init(init));
        boxes.append(&mut self.fragments);
        Ok(ScanStep::Done(boxes))
    }
}
