//! Sans-IO HTTP range driver for a remote MP4.
//!
//! [`RangeReader`] wraps an [`isobmff::Reader`] and turns each structure demand
//! into one byte-range fetch. It coalesces: a demand smaller than the caller's
//! `min_fetch` is widened to it, and later demands inside the fetched window
//! are answered without a request. The held window is bounded by `min_fetch`
//! plus the reader's largest single structure demand, so a remote `moov` costs
//! a bounded number of requests rather than one per box.
//!
//! The caller performs the HTTP request. It reports the answer with
//! [`RangeReader::on_status`] (so a server that ignores `Range` or refuses the
//! range is a typed error) and then [`RangeReader::on_range`]. No HTTP client
//! is built here, and no whole file is ever requested.

use http::StatusCode;
use isobmff::driver::{Advance, DriverError, DriverStep, advance, max_need_bytes};
use isobmff::{ByteRange, Reader, ReaderConfig, TrackInfo};
use thiserror::Error;

/// Failure of a range-reading session.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RangeError {
    /// The isobmff reader rejected the input or a driver step.
    #[error(transparent)]
    Format(#[from] DriverError),
    /// The server answered `200` with the whole representation, so it is not
    /// honouring `Range` and this driver cannot use it.
    #[error("server ignored the byte range request")]
    RangeNotSupported,
    /// The server answered `416 Range Not Satisfiable`.
    #[error("server rejected the byte range as unsatisfiable")]
    RangeNotSatisfiable,
    /// A response carried a successful status other than `206`.
    #[error("unexpected HTTP status {0} for a byte range request")]
    UnexpectedStatus(StatusCode),
    /// A range answer arrived while no fetch was outstanding.
    #[error("range response with no fetch outstanding")]
    UnexpectedRangeResponse,
    /// The caller polled again while a fetch was still outstanding.
    #[error("a range fetch is already outstanding")]
    FetchOutstanding,
    /// The returned range is not the one requested.
    #[error("range response {got:?} does not match the requested {expected:?}")]
    UnexpectedRange {
        /// The range this driver asked for.
        expected: ByteRange,
        /// The range the server answered with.
        got: ByteRange,
    },
}

impl From<isobmff::Error> for RangeError {
    fn from(error: isobmff::Error) -> Self {
        Self::Format(DriverError::Format(error))
    }
}

/// One action for the caller.
#[derive(Debug)]
#[non_exhaustive]
pub enum RangeAction<'a> {
    /// Perform an HTTP range `GET` for `[offset, offset + len)` and report it
    /// through [`RangeReader::on_range`] (after [`RangeReader::on_status`]).
    Fetch {
        /// First byte of the range.
        offset: u64,
        /// Number of bytes requested.
        len: u64,
    },
    /// A material reader step.
    Step(DriverStep<'a>),
    /// No more steps.
    Done,
}

/// A fetched window kept for reuse.
#[derive(Debug)]
struct Window {
    offset: u64,
    bytes: Vec<u8>,
}

/// The reader, plus the caller-driven HTTP range protocol.
#[derive(Debug)]
pub struct RangeReader {
    reader: Reader,
    config: ReaderConfig,
    min_fetch: u64,
    tracks: Vec<TrackInfo>,
    window: Option<Window>,
    need: Option<ByteRange>,
    pending: Option<ByteRange>,
    total_len: Option<u64>,
}

impl RangeReader {
    /// Wrap `config`. `min_fetch` is the caller's coalescing policy: a demand
    /// shorter than it fetches that many bytes. There is no library default.
    #[must_use]
    pub fn new(config: ReaderConfig, min_fetch: u64) -> Self {
        Self {
            reader: Reader::new(config.clone()),
            config,
            min_fetch,
            tracks: Vec::new(),
            window: None,
            need: None,
            pending: None,
            total_len: None,
        }
    }

    /// The next action: a range to fetch, a material step, or `Done`.
    pub fn poll(&mut self) -> Result<RangeAction<'_>, RangeError> {
        if self.pending.is_some() {
            return Err(RangeError::FetchOutstanding);
        }
        loop {
            match advance(self.reader.step()?)? {
                Advance::Need(range) => {
                    if let Some((start, end)) = self.window_indices(range) {
                        let window = self
                            .window
                            .as_ref()
                            .ok_or(RangeError::UnexpectedRangeResponse)?;
                        self.reader
                            .supply(range.offset, &window.bytes[start..end])?;
                        continue;
                    }
                    let fetch = self.fetch_for(range);
                    self.pending = Some(fetch);
                    self.need = Some(range);
                    return Ok(RangeAction::Fetch {
                        offset: fetch.offset,
                        len: fetch.len,
                    });
                }
                Advance::Tracks(tracks) => {
                    self.tracks = tracks;
                    return Ok(RangeAction::Step(DriverStep::Tracks(&self.tracks)));
                }
                Advance::Fragment(info) => {
                    return Ok(RangeAction::Step(DriverStep::Fragment(info)));
                }
                Advance::Sample(sample) => {
                    return Ok(RangeAction::Step(DriverStep::Sample(sample)));
                }
                Advance::Metadata(_) => {}
                Advance::Done => return Ok(RangeAction::Done),
            }
        }
    }

    /// Report the HTTP status of the outstanding range request.
    ///
    /// `206` proceeds to [`on_range`](Self::on_range); `200` (the whole
    /// representation) is [`RangeError::RangeNotSupported`]; `416` is
    /// [`RangeError::RangeNotSatisfiable`]. Any other success is
    /// [`RangeError::UnexpectedStatus`].
    pub fn on_status(&mut self, status: StatusCode) -> Result<(), RangeError> {
        match status {
            StatusCode::PARTIAL_CONTENT => Ok(()),
            StatusCode::OK => Err(RangeError::RangeNotSupported),
            StatusCode::RANGE_NOT_SATISFIABLE => Err(RangeError::RangeNotSatisfiable),
            other => Err(RangeError::UnexpectedStatus(other)),
        }
    }

    /// Report the body of the outstanding range request and its complete
    /// length, from `Content-Range`. A body shorter than requested means the
    /// input ended there.
    pub fn on_range(
        &mut self,
        offset: u64,
        bytes: &[u8],
        total_len: Option<u64>,
    ) -> Result<(), RangeError> {
        let pending = self
            .pending
            .take()
            .ok_or(RangeError::UnexpectedRangeResponse)?;
        let need = self
            .need
            .take()
            .ok_or(RangeError::UnexpectedRangeResponse)?;
        let got = ByteRange {
            offset,
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        };
        if offset != pending.offset || got.len > pending.len {
            self.pending = Some(pending);
            self.need = Some(need);
            return Err(RangeError::UnexpectedRange {
                expected: pending,
                got,
            });
        }
        if total_len.is_some() {
            self.total_len = total_len;
        }
        let window = Window {
            offset,
            bytes: bytes.to_vec(),
        };
        let start = usize::try_from(need.offset.saturating_sub(window.offset)).map_err(|_| {
            RangeError::UnexpectedRange {
                expected: need,
                got,
            }
        })?;
        let available = window.bytes.len().saturating_sub(start);
        let take = usize::try_from(need.len)
            .unwrap_or(usize::MAX)
            .min(available);
        let supplied = window.bytes.get(start..start + take).unwrap_or(&[]);
        self.reader.supply(need.offset, supplied)?;
        self.window = Some(window);
        Ok(())
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<isobmff::plan::SamplePlan<'_>> {
        self.reader.plan()
    }

    /// The total input length, once a response's `Content-Range` named it.
    #[must_use]
    pub fn total_len(&self) -> Option<u64> {
        self.total_len
    }

    fn window_indices(&self, range: ByteRange) -> Option<(usize, usize)> {
        let window = self.window.as_ref()?;
        let start = range.offset.checked_sub(window.offset)?;
        let end = start.checked_add(range.len)?;
        let start = usize::try_from(start).ok()?;
        let end = usize::try_from(end).ok()?;
        window.bytes.get(start..end)?;
        Some((start, end))
    }

    fn fetch_for(&self, range: ByteRange) -> ByteRange {
        let widened = range.len.max(self.min_fetch);
        let len = match self.total_len {
            Some(total) => widened.min(total.saturating_sub(range.offset)),
            None => widened,
        };
        ByteRange {
            offset: range.offset,
            len,
        }
    }

    /// The largest window this reader can hold: the caller's `min_fetch` plus
    /// the reader's largest structure demand.
    #[must_use]
    pub fn window_bound(&self) -> u64 {
        self.min_fetch
            .saturating_add(max_need_bytes(self.config.limits()))
    }
}
