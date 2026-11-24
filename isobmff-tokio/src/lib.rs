//! Async (tokio) driver over the [`isobmff`] sans-IO MP4 reader.
//!
//! [`AsyncFileReader`] wraps an `AsyncRead + AsyncSeek` source and answers the
//! reader's structure demands itself, exactly like the blocking
//! [`isobmff::driver::FileReader`]. It holds one `moov`/`moof` buffer bounded
//! by the reader's own [`isobmff::Limits`], never a whole input, and reuses
//! the blocking driver's buffer-sizing helpers and error type so the two
//! cannot drift.
//!
//! The one async-only fact is the total input length: when a
//! [`ReaderConfig`] names none, [`AsyncFileReader::new`]
//! seeks to the end to learn it, so a box of size 0 can run to the end of the
//! file.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::io::SeekFrom;

use isobmff::driver::{Advance, DriverError, DriverStep, advance, need_read_len, sample_read_len};
use isobmff::{Reader, ReaderConfig, SampleOrder, SampleRef, TrackInfo};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

/// An [`isobmff::Reader`] plus an async seekable source that answers its
/// demands.
#[derive(Debug)]
pub struct AsyncFileReader<R> {
    inner: R,
    reader: Reader,
    config: ReaderConfig,
    tracks: Vec<TrackInfo>,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + AsyncSeek + Unpin> AsyncFileReader<R> {
    /// Wrap `inner`. When `config` names no total length, the source's own
    /// length fills it.
    pub async fn new(mut inner: R, config: ReaderConfig) -> std::io::Result<Self> {
        let config = match config.total_len() {
            Some(_) => config,
            None => {
                let total = inner.seek(SeekFrom::End(0)).await?;
                config.with_total_len(Some(total))
            }
        };
        Ok(Self {
            inner,
            reader: Reader::new(config.clone()),
            config,
            tracks: Vec::new(),
            buffer: Vec::new(),
        })
    }

    /// The next material step, answering every structure demand from the
    /// source.
    pub async fn step(&mut self) -> Result<DriverStep<'_>, DriverError> {
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => self.serve(range).await?,
                Advance::Tracks(tracks) => {
                    self.tracks = tracks;
                    return Ok(DriverStep::Tracks(&self.tracks));
                }
                Advance::Fragment(info) => return Ok(DriverStep::Fragment(info)),
                Advance::Sample(sample) => return Ok(DriverStep::Sample(sample)),
                Advance::Done => return Ok(DriverStep::Done),
            }
        }
    }

    /// Read one sample's data into `out`, replacing its contents.
    pub async fn read_sample(
        &mut self,
        sample: &SampleRef,
        out: &mut Vec<u8>,
    ) -> Result<(), DriverError> {
        let len = sample_read_len(sample, self.config.limits())?;
        out.clear();
        out.resize(len, 0);
        self.inner.seek(SeekFrom::Start(sample.offset)).await?;
        self.inner.read_exact(out).await?;
        Ok(())
    }

    /// Seek to the last sync sample at or before `decode_time`, in `track`'s
    /// timescale.
    pub async fn seek_to(
        &mut self,
        track: isobmff::TrackId,
        decode_time: u64,
    ) -> Result<(), DriverError> {
        self.reader.seek_to(track, decode_time)?;
        Ok(())
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<isobmff::plan::SamplePlan<'_>> {
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

    async fn serve(&mut self, range: isobmff::ByteRange) -> Result<(), DriverError> {
        let len = need_read_len(range, self.config.total_len(), self.config.limits())?;
        self.buffer.clear();
        self.buffer.resize(len, 0);
        if !self.buffer.is_empty() {
            self.inner.seek(SeekFrom::Start(range.offset)).await?;
            self.inner.read_exact(&mut self.buffer).await?;
        }
        self.reader.supply(range.offset, &self.buffer)?;
        Ok(())
    }
}
