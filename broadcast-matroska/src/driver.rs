//! Blocking drivers over the sans-IO [`Reader`]: one for a source that can
//! seek, one for a source that can only move forward.
//!
//! [`Reader`] asks for structure byte ranges and yields descriptors; these
//! drivers answer every demand themselves and hand the caller only the material
//! steps. Neither holds a whole input: [`FileReader`] seeks to each demand and
//! [`ForwardReader`] discards what lies between demands through a fixed buffer,
//! so the reader's own [`Limits`] bound every allocation.
//!
//! Frame bytes are never copied by a driver's walk. A [`DriverStep::Block`]
//! names the frame's range and [`FileReader::read_frame`] /
//! [`ForwardReader::read_frame`] fetches exactly those bytes when the caller
//! asks.
//!
//! See RFC 8794 §6.2 and RFC 9559 §5, §6, §12
//! (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`,
//! `private/specs-verified/ietf/specs/rfc9559_matroska.txt`).

use std::io::{Read, Seek, SeekFrom};

use crate::error::Error;
use crate::limits::Limits;
use crate::reader::{
    BlockStep, ByteRange, CueEntry, ElementRef, Mode, Reader, SeekOutcome, SegmentInfo, Source,
    Step, TrackInfo, Warning,
};

/// Bytes discarded per read when a forward-only source must skip to the next
/// structure demand. One read allocation, reused for the whole run.
pub const DISCARD_CHUNK_BYTES: usize = 8192;

/// Bytes of the previous read a forward-only source retains, so a payload that
/// starts inside a header demand is answered without a backward seek. An EBML
/// element header occupies at most twelve octets (RFC 8794 §5, §6.1).
pub const MAX_HEADER_REWIND_BYTES: usize = crate::reader::MAX_ELEMENT_HEADER_BYTES as usize;

/// Failure of a blocking driver.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DriverError {
    /// The underlying source failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The reader rejected the input.
    #[error(transparent)]
    Format(#[from] Error),
    /// A forward-only source cannot reach a frame whose bytes lie behind it.
    #[error("frame at offset {offset} is behind the forward-only position {position}")]
    FrameBehindPosition {
        /// Absolute offset of the frame's data.
        offset: u64,
        /// The byte offset the forward-only source has reached.
        position: u64,
    },
}

/// One owned classification of a reader [`Step`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Advance {
    /// The reader needs this range.
    Need(ByteRange),
    /// The Segment's `Info`.
    Info {
        /// `TimestampScale` and `Duration`.
        info: SegmentInfo,
        /// The `Info` element, header included.
        element: ByteRange,
    },
    /// The Segment's tracks.
    Tracks {
        /// The track table.
        tracks: std::vec::Vec<TrackInfo>,
        /// The `Tracks` element, header included.
        element: ByteRange,
    },
    /// One Block frame.
    Block(BlockStep),
    /// The Segment's `Cues`.
    Cues(std::vec::Vec<CueEntry>),
    /// A `Chapters` element.
    Chapters(ElementRef),
    /// A `Tags` element.
    Tags(ElementRef),
    /// An `Attachments` element.
    Attachments(ElementRef),
    /// A tolerated fault.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// Classify one reader [`Step`] into the owned [`Advance`] a driver acts on.
pub fn advance(step: Step<'_>) -> Result<Advance, DriverError> {
    Ok(match step {
        Step::Need(range) => Advance::Need(range),
        Step::Info { info, element } => Advance::Info { info, element },
        Step::Tracks { tracks, element } => Advance::Tracks {
            tracks: tracks.to_vec(),
            element,
        },
        Step::Block(block) => Advance::Block(block),
        Step::Cues(cues) => Advance::Cues(cues.to_vec()),
        Step::Chapters(element) => Advance::Chapters(element),
        Step::Tags(element) => Advance::Tags(element),
        Step::Attachments(element) => Advance::Attachments(element),
        Step::Warning(warning) => Advance::Warning(warning),
        Step::Done => Advance::Done,
    })
}

/// A material reader step: [`Step`] without the [`Step::Need`] the drivers
/// answer themselves.
#[derive(Debug)]
#[non_exhaustive]
pub enum DriverStep<'a> {
    /// The Segment's `Info` and its element range.
    Info {
        /// `TimestampScale` and `Duration`.
        info: SegmentInfo,
        /// The `Info` element, header included.
        element: ByteRange,
    },
    /// The Segment's tracks, in `TrackEntry` order.
    Tracks {
        /// The track table.
        tracks: &'a [TrackInfo],
        /// The `Tracks` element, header included.
        element: ByteRange,
    },
    /// One Block frame; fetch its bytes with `read_frame`.
    Block(BlockStep),
    /// The Segment's `Cues`.
    Cues(&'a [CueEntry]),
    /// A `Chapters` element; fetch its bytes with `read_element`.
    Chapters(ElementRef),
    /// A `Tags` element; fetch its bytes with `read_element`.
    Tags(ElementRef),
    /// An `Attachments` element; fetch its bytes with `read_element`.
    Attachments(ElementRef),
    /// A tolerated fault.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// Bytes a driver should read for one `Need`, refused above the reader's
/// master cap so the driver's reusable buffer stays bounded.
fn need_read_len(range: ByteRange, limits: &Limits) -> Result<usize, DriverError> {
    if range.len > limits.max_master_bytes() {
        return Err(DriverError::Format(Error::LimitExceeded {
            limit: "master read",
            value: range.len,
        }));
    }
    usize::try_from(range.len).map_err(|_| {
        DriverError::Format(Error::InvalidField(broadcast_common::error::InvalidField {
            field: "need",
            reason: "length does not fit a buffer",
        }))
    })
}

/// A frame range's read length under the reader's element cap.
fn frame_read_len(range: ByteRange, limits: &Limits) -> Result<usize, DriverError> {
    if range.len > limits.max_element_bytes() {
        return Err(DriverError::Format(Error::LimitExceeded {
            limit: "max_element_bytes",
            value: range.len,
        }));
    }
    usize::try_from(range.len).map_err(|_| {
        DriverError::Format(Error::InvalidField(broadcast_common::error::InvalidField {
            field: "frame",
            reason: "length does not fit a buffer",
        }))
    })
}

fn read_exact_or_short<R: Read>(
    inner: &mut R,
    buffer: &mut std::vec::Vec<u8>,
    len: usize,
) -> std::io::Result<()> {
    buffer.clear();
    buffer.resize(len, 0);
    let mut filled = 0usize;
    while filled < len {
        let read = inner.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled = filled.saturating_add(read);
    }
    buffer.truncate(filled);
    Ok(())
}

/// The reader, plus a seekable source that answers its demands.
#[derive(Debug)]
pub struct FileReader<R> {
    inner: R,
    reader: Reader,
    limits: Limits,
    tracks: std::vec::Vec<TrackInfo>,
    cues: std::vec::Vec<CueEntry>,
    buffer: std::vec::Vec<u8>,
}

impl<R: Read + Seek> FileReader<R> {
    /// Wrap `inner`. Its length is measured once so the reader can clamp every
    /// demand and recognise the end of the input.
    pub fn new(mut inner: R, mode: Mode, limits: Limits) -> std::io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            reader: Reader::new(Source::Seekable { len }, mode, limits),
            limits,
            tracks: std::vec::Vec::new(),
            cues: std::vec::Vec::new(),
            buffer: std::vec::Vec::new(),
        })
    }

    /// The next material step, answering every structure demand from the
    /// source.
    pub fn step(&mut self) -> Result<DriverStep<'_>, DriverError> {
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => self.serve(range)?,
                Advance::Info { info, element } => return Ok(DriverStep::Info { info, element }),
                Advance::Tracks { tracks, element } => {
                    self.tracks = tracks;
                    return Ok(DriverStep::Tracks {
                        tracks: &self.tracks,
                        element,
                    });
                }
                Advance::Block(block) => return Ok(DriverStep::Block(block)),
                Advance::Cues(cues) => {
                    self.cues = cues;
                    return Ok(DriverStep::Cues(&self.cues));
                }
                Advance::Chapters(element) => return Ok(DriverStep::Chapters(element)),
                Advance::Tags(element) => return Ok(DriverStep::Tags(element)),
                Advance::Attachments(element) => return Ok(DriverStep::Attachments(element)),
                Advance::Warning(warning) => return Ok(DriverStep::Warning(warning)),
                Advance::Done => return Ok(DriverStep::Done),
            }
        }
    }

    /// Read one frame's bytes into `out`, replacing its contents.
    pub fn read_frame(
        &mut self,
        block: &BlockStep,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        self.read_range(block.frame, out)
    }

    /// Read one whole element's bytes into `out`, replacing its contents.
    pub fn read_element(
        &mut self,
        element: &ElementRef,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        self.read_range(element.range, out)
    }

    /// Seek to the last cue point or indexed Cluster at or before `timestamp`.
    pub fn seek_to(&mut self, track: u64, timestamp: i64) -> Result<SeekOutcome, DriverError> {
        Ok(self.reader.seek_to(track, timestamp)?)
    }

    /// The reader, for its track and cue tables.
    #[must_use]
    pub const fn reader(&self) -> &Reader {
        &self.reader
    }

    /// Recover the wrapped source.
    pub fn into_inner(self) -> R {
        self.inner
    }

    fn read_range(
        &mut self,
        range: ByteRange,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        let len = frame_read_len(range, &self.limits)?;
        out.clear();
        out.resize(len, 0);
        if len > 0 {
            self.inner.seek(SeekFrom::Start(range.offset))?;
            self.inner.read_exact(out)?;
        }
        Ok(())
    }

    fn serve(&mut self, range: ByteRange) -> Result<(), DriverError> {
        let len = need_read_len(range, &self.limits)?;
        if len > 0 {
            self.inner.seek(SeekFrom::Start(range.offset))?;
        }
        read_exact_or_short(&mut self.inner, &mut self.buffer, len)?;
        let buffer = core::mem::take(&mut self.buffer);
        let supplied = self.reader.supply(range.offset, &buffer);
        self.buffer = buffer;
        supplied?;
        Ok(())
    }
}

/// The reader, plus a source that can only move forward.
///
/// A demand at the current position is read directly; a demand ahead of it is
/// reached by reading and discarding through one fixed [`DISCARD_CHUNK_BYTES`]
/// buffer. A demand behind it is the reader's typed
/// [`Error::BackwardSeek`]. Frame bytes are read by [`ForwardReader::read_frame`]
/// and must lie at or ahead of the bytes already consumed.
#[derive(Debug)]
pub struct ForwardReader<R> {
    inner: R,
    reader: Reader,
    limits: Limits,
    tracks: std::vec::Vec<TrackInfo>,
    cues: std::vec::Vec<CueEntry>,
    pos: u64,
    rewind: usize,
    buffer: std::vec::Vec<u8>,
    scratch: std::vec::Vec<u8>,
    tail: std::vec::Vec<u8>,
}

impl<R: Read> ForwardReader<R> {
    /// Wrap `inner`, forcing the reader to a forward-only source.
    #[must_use]
    pub fn new(inner: R, mode: Mode, limits: Limits) -> Self {
        let rewind = usize::try_from(limits.max_block_header_bytes())
            .unwrap_or(usize::MAX)
            .max(MAX_HEADER_REWIND_BYTES);
        Self {
            inner,
            reader: Reader::new(Source::ForwardOnly, mode, limits),
            limits,
            tracks: std::vec::Vec::new(),
            cues: std::vec::Vec::new(),
            pos: 0,
            rewind,
            buffer: std::vec::Vec::new(),
            scratch: std::vec![0u8; DISCARD_CHUNK_BYTES],
            tail: std::vec::Vec::new(),
        }
    }

    /// The next material step, answering every structure demand from the
    /// source.
    pub fn step(&mut self) -> Result<DriverStep<'_>, DriverError> {
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => self.serve(range)?,
                Advance::Info { info, element } => return Ok(DriverStep::Info { info, element }),
                Advance::Tracks { tracks, element } => {
                    self.tracks = tracks;
                    return Ok(DriverStep::Tracks {
                        tracks: &self.tracks,
                        element,
                    });
                }
                Advance::Block(block) => return Ok(DriverStep::Block(block)),
                Advance::Cues(cues) => {
                    self.cues = cues;
                    return Ok(DriverStep::Cues(&self.cues));
                }
                Advance::Chapters(element) => return Ok(DriverStep::Chapters(element)),
                Advance::Tags(element) => return Ok(DriverStep::Tags(element)),
                Advance::Attachments(element) => return Ok(DriverStep::Attachments(element)),
                Advance::Warning(warning) => return Ok(DriverStep::Warning(warning)),
                Advance::Done => return Ok(DriverStep::Done),
            }
        }
    }

    /// Read one frame's bytes into `out`, replacing its contents. The frame
    /// must lie at or ahead of the bytes already consumed.
    pub fn read_frame(
        &mut self,
        block: &BlockStep,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        self.read_range(block.frame, out)
    }

    /// Read one whole element's bytes into `out`, replacing its contents. The
    /// element must lie at or ahead of the bytes already consumed.
    pub fn read_element(
        &mut self,
        element: &ElementRef,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        self.read_range(element.range, out)
    }

    /// Seek to the last cue point or indexed Cluster at or before `timestamp`.
    /// A seek that would move the walk backwards is [`Error::BackwardSeek`].
    pub fn seek_to(&mut self, track: u64, timestamp: i64) -> Result<SeekOutcome, DriverError> {
        Ok(self.reader.seek_to(track, timestamp)?)
    }

    /// The reader, for its track and cue tables.
    #[must_use]
    pub const fn reader(&self) -> &Reader {
        &self.reader
    }

    /// Recover the wrapped source.
    pub fn into_inner(self) -> R {
        self.inner
    }

    fn read_range(
        &mut self,
        range: ByteRange,
        out: &mut std::vec::Vec<u8>,
    ) -> Result<(), DriverError> {
        let len = frame_read_len(range, &self.limits)?;
        out.clear();
        let mut physical = range.offset;
        if range.offset < self.pos {
            let back = usize::try_from(self.pos - range.offset).unwrap_or(usize::MAX);
            if back > self.tail.len() {
                return Err(DriverError::FrameBehindPosition {
                    offset: range.offset,
                    position: self.pos,
                });
            }
            let start = self.tail.len() - back;
            let take = len.min(back);
            out.extend_from_slice(&self.tail[start..start + take]);
            physical = self.pos;
        } else {
            if range.offset > self.pos {
                self.tail.clear();
            }
            self.skip_to(range.offset)?;
        }
        let prefix = out.len();
        if prefix < len {
            out.resize(len, 0);
            self.inner.read_exact(&mut out[prefix..])?;
            let extra = len - prefix;
            self.pos = physical.saturating_add(u64::try_from(extra).unwrap_or(u64::MAX));
            self.tail.extend_from_slice(&out[prefix..]);
            let excess = self.tail.len().saturating_sub(self.rewind);
            if excess > 0 {
                self.tail.drain(..excess);
            }
        }
        Ok(())
    }

    fn serve(&mut self, range: ByteRange) -> Result<(), DriverError> {
        let total = need_read_len(range, &self.limits)?;
        let mut physical = range.offset;
        self.buffer.clear();
        if range.offset < self.pos {
            let back = usize::try_from(self.pos - range.offset).unwrap_or(usize::MAX);
            if back > self.tail.len() {
                return Err(DriverError::Format(Error::BackwardSeek {
                    offset: range.offset,
                    position: self.pos,
                }));
            }
            let start = self.tail.len() - back;
            let take = total.min(back);
            self.buffer
                .extend_from_slice(&self.tail[start..start + take]);
            physical = self.pos;
        } else {
            if range.offset > self.pos {
                self.tail.clear();
            }
            self.skip_to(range.offset)?;
        }
        let prefix = self.buffer.len();
        if prefix < total {
            self.buffer.resize(total, 0);
            let mut filled = prefix;
            while filled < total {
                let read = self.inner.read(&mut self.buffer[filled..])?;
                if read == 0 {
                    break;
                }
                filled = filled.saturating_add(read);
            }
            self.buffer.truncate(filled);
        }
        let extra = self.buffer.len().saturating_sub(prefix);
        self.pos = physical.saturating_add(u64::try_from(extra).unwrap_or(u64::MAX));
        if extra > 0 {
            self.tail.extend_from_slice(&self.buffer[prefix..]);
            let excess = self.tail.len().saturating_sub(self.rewind);
            if excess > 0 {
                self.tail.drain(..excess);
            }
        }
        let buffer = core::mem::take(&mut self.buffer);
        let supplied = self.reader.supply(range.offset, &buffer);
        self.buffer = buffer;
        supplied?;
        Ok(())
    }

    fn skip_to(&mut self, offset: u64) -> Result<(), DriverError> {
        let mut remaining = offset.saturating_sub(self.pos);
        while remaining > 0 {
            let chunk = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(self.scratch.len());
            let read = self.inner.read(&mut self.scratch[..chunk])?;
            if read == 0 {
                return Err(DriverError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "input ended before the requested offset",
                )));
            }
            let read = u64::try_from(read).unwrap_or(u64::MAX);
            remaining = remaining.saturating_sub(read);
            self.pos = self.pos.saturating_add(read);
        }
        Ok(())
    }
}
