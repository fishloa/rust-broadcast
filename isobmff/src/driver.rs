//! Blocking drivers over the sans-IO [`Reader`]: one for a source that can
//! seek, one for a source that can only move forward.
//!
//! [`Reader`] asks for structure byte ranges and yields descriptors; these
//! drivers answer each demand themselves and hand the caller only the material
//! steps. Neither holds a whole input: [`FileReader`] seeks to each demand and
//! [`ForwardReader`] discards what lies between demands through a fixed
//! buffer, so the reader's own [`Limits`] continue to bound every allocation.
//!
//! A progressive file whose `moov` follows its `mdat` is reached by
//! [`FileReader`] through header skips alone. [`ForwardReader`] cannot skip
//! media it has not read, so that file is the reader's typed
//! [`Error::MoovAfterMdatForwardOnly`].
//!
//! Both drivers report the movie's and each track's typed metadata as
//! [`DriverStep::Metadata`], and [`ForwardReader::read_sample`] reads the
//! samples of a fragmented file in file order, which is what makes a pipe or a
//! socket source work.
//!
//! See the streaming-first design §4.2 and ISO/IEC 14496-12:2026 §4.2, §8.2
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use std::io::{Read, Seek, SeekFrom};

use crate::error::Error;
use crate::limits::Limits;
use crate::reader::{
    ByteRange, Reader, ReaderConfig, SampleOrder, Source, Step, TrackInfo, Warning,
};
use crate::sample_table::SampleRef;

/// Bytes discarded per read when a forward-only source must skip to the next
/// structure demand. One read allocation, reused for the whole run.
pub const DISCARD_CHUNK_BYTES: usize = 8192;

/// Bytes of the previous read a forward-only source retains, so a header it
/// must re-read with a `largesize` or `uuid` extension (ISO/IEC 14496-12:2026
/// §4.2) is answered without a backward seek. A box header occupies at most
/// 32 bytes (`size`, `type`, 64-bit `largesize`, 16-byte `usertype`).
pub const MAX_HEADER_REWIND_BYTES: usize = 32;

/// The largest structure range the reader can demand under `limits`: the
/// larger of a whole `moov`/index box and a whole fragment. A driver's
/// reusable buffer is bounded by it.
#[must_use]
pub fn max_need_bytes(limits: &Limits) -> u64 {
    limits.max_need_bytes()
}

/// Resolve a structure [`Need`](Step::Need) to the number of bytes a driver
/// should read: clamped to the known input length and refused above
/// [`max_need_bytes`] so the driver's buffer stays bounded by the caps.
pub fn need_read_len(
    range: ByteRange,
    total_len: Option<u64>,
    limits: &Limits,
) -> core::result::Result<usize, DriverError> {
    let available = match total_len {
        Some(total) => total.saturating_sub(range.offset).min(range.len),
        None => range.len,
    };
    let cap = max_need_bytes(limits);
    if available > cap {
        return Err(DriverError::Format(Error::LimitExceeded {
            what: "structure need",
            limit: cap,
        }));
    }
    usize::try_from(available).map_err(|_| {
        DriverError::Format(Error::InvalidInput(
            "structure need does not fit a buffer length",
        ))
    })
}

/// Convert a [`SampleRef`] size into a checked buffer length under the
/// reader's sample-size cap.
pub fn sample_read_len(
    sample: &SampleRef,
    limits: &Limits,
) -> core::result::Result<usize, DriverError> {
    if sample.size > limits.max_sample_bytes() {
        return Err(DriverError::Format(Error::LimitExceeded {
            what: "sample size",
            limit: u64::from(limits.max_sample_bytes()),
        }));
    }
    usize::try_from(sample.size).map_err(|_| {
        DriverError::Format(Error::InvalidInput(
            "sample size does not fit a buffer length",
        ))
    })
}

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
    /// A forward-only source cannot reach a sample whose data lies behind the
    /// bytes already consumed.
    #[error("sample at offset {offset} is behind the forward-only position {position}")]
    SampleBehindPosition {
        /// Absolute file offset of the sample's data.
        offset: u64,
        /// The byte offset the forward-only source has reached.
        position: u64,
    },
}

/// One owned classification of a reader [`Step`].
///
/// [`advance`] turns a borrow of the reader into this owned value, so a driver
/// can answer a [`Advance::Need`] and then return a track table it owns. A new
/// reader step kind is one arm here, shared by every driver.
#[derive(Debug)]
pub enum Advance {
    /// The reader needs this structure range.
    Need(ByteRange),
    /// The movie's track table.
    Tracks(Vec<TrackInfo>),
    /// A fragment's samples follow.
    Fragment(crate::reader::FragmentInfo),
    /// One sample descriptor.
    Sample(SampleRef),
    /// The movie's and tracks' metadata.
    Metadata(crate::metadata::MovieMetadata),
    /// A tolerated fault.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// Classify one reader [`Step`] into the owned [`Advance`] a driver acts on.
pub fn advance(step: Step<'_>) -> core::result::Result<Advance, DriverError> {
    match step {
        Step::Need(range) => Ok(Advance::Need(range)),
        Step::Tracks(tracks) => Ok(Advance::Tracks(tracks.to_vec())),
        Step::Fragment(info) => Ok(Advance::Fragment(info)),
        Step::Sample(sample) => Ok(Advance::Sample(sample)),
        Step::Metadata(metadata) => Ok(Advance::Metadata(metadata.clone())),
        Step::Warning(warning) => Ok(Advance::Warning(warning)),
        Step::Done => Ok(Advance::Done),
    }
}

/// A material reader step: [`Step`] without the [`Step::Need`] the drivers
/// answer themselves.
///
/// The track table is owned by the driver (cloned once, when the reader
/// surfaces it), so a returned step never keeps the reader borrowed.
#[derive(Debug)]
#[non_exhaustive]
pub enum DriverStep<'a> {
    /// Every track of the movie, in `moov` order. Emitted once.
    Tracks(&'a [TrackInfo]),
    /// A fragment's samples follow.
    Fragment(crate::reader::FragmentInfo),
    /// One sample descriptor, in the configured order.
    Sample(SampleRef),
    /// The movie's and each track's typed metadata. Emitted once, after
    /// [`DriverStep::Tracks`], only when the file carries `udta` or `meta`.
    Metadata(&'a crate::metadata::MovieMetadata),
    /// A fault the reader's permissive mode tolerated.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// The reader, plus a seekable source that answers its demands.
#[derive(Debug)]
pub struct FileReader<R> {
    inner: R,
    reader: Reader,
    config: ReaderConfig,
    tracks: Vec<TrackInfo>,
    metadata: Option<crate::metadata::MovieMetadata>,
    buffer: Vec<u8>,
}

impl<R: Read + Seek> FileReader<R> {
    /// Wrap `inner`. When `config` names no total length, the source's own
    /// length fills it, so a box of size 0 can run to the end of the file.
    pub fn new(mut inner: R, config: ReaderConfig) -> std::io::Result<Self> {
        let config = match config.total_len() {
            Some(_) => config,
            None => {
                let total = inner.seek(SeekFrom::End(0))?;
                config.with_total_len(Some(total))
            }
        };
        Ok(Self {
            inner,
            reader: Reader::new(config.clone()),
            config,
            tracks: Vec::new(),
            metadata: None,
            buffer: Vec::new(),
        })
    }

    /// The next material step, answering every structure demand from the
    /// source.
    pub fn step(&mut self) -> core::result::Result<DriverStep<'_>, DriverError> {
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => self.serve(range)?,
                Advance::Tracks(tracks) => {
                    self.tracks = tracks;
                    return Ok(DriverStep::Tracks(&self.tracks));
                }
                Advance::Fragment(info) => return Ok(DriverStep::Fragment(info)),
                Advance::Sample(sample) => return Ok(DriverStep::Sample(sample)),
                Advance::Metadata(metadata) => {
                    return Ok(DriverStep::Metadata(self.metadata.insert(metadata)));
                }
                Advance::Warning(warning) => return Ok(DriverStep::Warning(warning)),
                Advance::Done => return Ok(DriverStep::Done),
            }
        }
    }

    /// Read one sample's data into `out`, replacing its contents.
    pub fn read_sample(
        &mut self,
        sample: &SampleRef,
        out: &mut Vec<u8>,
    ) -> core::result::Result<(), DriverError> {
        let len = sample_read_len(sample, self.config.limits())?;
        out.clear();
        out.resize(len, 0);
        self.inner.seek(SeekFrom::Start(sample.offset))?;
        self.inner.read_exact(out)?;
        Ok(())
    }

    /// Seek to the last sync sample at or before `decode_time`, in `track`'s
    /// timescale.
    pub fn seek_to(
        &mut self,
        track: crate::TrackId,
        decode_time: u64,
    ) -> core::result::Result<(), DriverError> {
        self.reader.seek_to(track, decode_time)?;
        Ok(())
    }

    /// The sans-IO [`Reader`](crate::Reader) this driver supplies bytes to.
    ///
    /// For the facts a [`DriverStep`] does not carry: the file's extended
    /// types, its identified media data, and the `mehd` duration.
    #[must_use]
    pub fn reader(&self) -> &crate::Reader {
        &self.reader
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<crate::plan::SamplePlan<'_>> {
        self.reader.plan()
    }

    /// The steps' sample order.
    #[must_use]
    pub fn order(&self) -> SampleOrder {
        self.config.order()
    }

    /// Recover the wrapped source.
    pub fn into_inner(self) -> R {
        self.inner
    }

    fn serve(&mut self, range: ByteRange) -> core::result::Result<(), DriverError> {
        let len = need_read_len(range, self.config.total_len(), self.config.limits())?;
        self.buffer.clear();
        self.buffer.resize(len, 0);
        if !self.buffer.is_empty() {
            self.inner.seek(SeekFrom::Start(range.offset))?;
            self.inner.read_exact(&mut self.buffer)?;
        }
        self.reader.supply(range.offset, &self.buffer)?;
        Ok(())
    }
}

/// The reader, plus a source that can only move forward.
///
/// A demand at the current position is read directly; a demand ahead of it is
/// reached by reading and discarding through one fixed [`DISCARD_CHUNK_BYTES`]
/// buffer. A demand behind it is the reader's typed
/// [`Error::MoovAfterMdatForwardOnly`].
///
/// Samples are handed out in file order whatever `config` asked for: the reader
/// then emits each sample's data at or after the bytes already consumed, which
/// is what makes [`ForwardReader::read_sample`] work over a pipe on a
/// fragmented input. A `SampleOrder::DecodeTime` order across the tracks of one
/// fragment is not file order, so this driver replaces it.
#[derive(Debug)]
pub struct ForwardReader<R> {
    inner: R,
    reader: Reader,
    config: ReaderConfig,
    tracks: Vec<TrackInfo>,
    metadata: Option<crate::metadata::MovieMetadata>,
    pos: u64,
    buffer: Vec<u8>,
    scratch: Vec<u8>,
    tail: Vec<u8>,
}

impl<R: Read> ForwardReader<R> {
    /// Wrap `inner`, forcing the reader to a forward-only source and to
    /// file-order samples.
    #[must_use]
    pub fn new(inner: R, config: ReaderConfig) -> Self {
        let config = config
            .with_source(Source::ForwardOnly)
            .with_order(SampleOrder::File);
        Self {
            inner,
            reader: Reader::new(config.clone()),
            config,
            tracks: Vec::new(),
            metadata: None,
            pos: 0,
            buffer: Vec::new(),
            scratch: vec![0u8; DISCARD_CHUNK_BYTES],
            tail: Vec::new(),
        }
    }

    /// The next material step, answering every structure demand from the
    /// source.
    pub fn step(&mut self) -> core::result::Result<DriverStep<'_>, DriverError> {
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => self.serve(range)?,
                Advance::Tracks(tracks) => {
                    self.tracks = tracks;
                    return Ok(DriverStep::Tracks(&self.tracks));
                }
                Advance::Fragment(info) => return Ok(DriverStep::Fragment(info)),
                Advance::Sample(sample) => return Ok(DriverStep::Sample(sample)),
                Advance::Metadata(metadata) => {
                    return Ok(DriverStep::Metadata(self.metadata.insert(metadata)));
                }
                Advance::Warning(warning) => return Ok(DriverStep::Warning(warning)),
                Advance::Done => return Ok(DriverStep::Done),
            }
        }
    }

    /// Read one sample's data into `out`, replacing its contents. The sample
    /// must lie at or ahead of the bytes already consumed.
    pub fn read_sample(
        &mut self,
        sample: &SampleRef,
        out: &mut Vec<u8>,
    ) -> core::result::Result<(), DriverError> {
        if sample.offset < self.pos {
            return Err(DriverError::SampleBehindPosition {
                offset: sample.offset,
                position: self.pos,
            });
        }
        let len = sample_read_len(sample, self.config.limits())?;
        self.skip_to(sample.offset)?;
        out.clear();
        out.resize(len, 0);
        self.inner.read_exact(out)?;
        self.pos = sample.offset.saturating_add(u64::from(sample.size));
        self.tail.clear();
        Ok(())
    }

    /// Seek to the last sync sample at or before `decode_time`. A seek that
    /// would move the walk backwards is the reader's typed error.
    pub fn seek_to(
        &mut self,
        track: crate::TrackId,
        decode_time: u64,
    ) -> core::result::Result<(), DriverError> {
        self.reader.seek_to(track, decode_time)?;
        Ok(())
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<crate::plan::SamplePlan<'_>> {
        self.reader.plan()
    }

    /// The steps' sample order.
    #[must_use]
    pub fn order(&self) -> SampleOrder {
        self.config.order()
    }

    /// Recover the wrapped source.
    pub fn into_inner(self) -> R {
        self.inner
    }

    fn serve(&mut self, range: ByteRange) -> core::result::Result<(), DriverError> {
        let total = need_read_len(range, self.config.total_len(), self.config.limits())?;
        self.buffer.clear();
        let mut physical = range.offset;
        if range.offset < self.pos {
            // A header the reader must re-read with its `largesize`/`uuid`
            // extension starts where the previous header did; borrow those
            // bytes from the retained tail instead of seeking backwards.
            let back = usize::try_from(self.pos - range.offset).unwrap_or(usize::MAX);
            if back > self.tail.len() {
                return Err(DriverError::Format(Error::InvalidInput(
                    "a backward range needs a seekable source",
                )));
            }
            let start = self.tail.len() - back;
            let take = core::cmp::min(total, back);
            self.buffer
                .extend_from_slice(&self.tail[start..start + take]);
            physical = self.pos;
        } else {
            self.skip_to(range.offset)?;
        }
        let prefix = self.buffer.len();
        if prefix < total {
            let mut filled = prefix;
            self.buffer.resize(total, 0);
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
        self.remember_tail();
        self.reader.supply(range.offset, &self.buffer)?;
        Ok(())
    }

    fn remember_tail(&mut self) {
        let keep = core::cmp::min(self.buffer.len(), MAX_HEADER_REWIND_BYTES);
        self.tail.clear();
        if keep > 0 {
            let start = self.buffer.len() - keep;
            self.tail.extend_from_slice(&self.buffer[start..]);
        }
    }

    fn skip_to(&mut self, offset: u64) -> core::result::Result<(), DriverError> {
        let mut remaining = offset.saturating_sub(self.pos);
        while remaining > 0 {
            let chunk = core::cmp::min(
                usize::try_from(remaining).unwrap_or(usize::MAX),
                self.scratch.len(),
            );
            let read = self.inner.read(&mut self.scratch[..chunk])?;
            if read == 0 {
                return Err(DriverError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "input ended before the requested offset",
                )));
            }
            remaining = remaining.saturating_sub(u64::try_from(read).unwrap_or(u64::MAX));
            self.pos = self
                .pos
                .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        }
        Ok(())
    }
}
