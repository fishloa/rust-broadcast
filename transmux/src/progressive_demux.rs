//! Progressive (single-file, non-fragmented) MP4 demux — ISO/IEC
//! 14496-12:2026 §8.5–§8.7.
//!
//! [`ProgressiveDemux`] parses a non-fragmented `.mp4` (a `moov` carrying full
//! per-track sample tables, no `moof`) into the crate's [`Media`] IR — the
//! demux counterpart to [`crate::progressive::ProgressiveMux`] and the
//! moov-only sibling of [`crate::media::Fmp4Demux`]. It reuses
//! [`crate::media`]'s `moov` → [`crate::pipeline::TrackSpec`] reconstruction
//! verbatim (the `stsd` → [`crate::pipeline::CodecConfig`] path is the single
//! shared implementation for both demuxers), and additionally walks the
//! `stbl` sample tables that a progressive file carries instead of
//! `moof`/`trun` fragments:
//!
//! - `stts` (§8.6.1.2): run-length decode-time deltas, expanded to a
//!   per-sample duration.
//! - `ctts` (§8.6.1.3): run-length composition offsets (v0 unsigned / v1
//!   signed — both stored as a wire `u32` reinterpreted as `i32`, matching
//!   [`isobmff::timing::CompositionOffsetBox`]'s own convention), expanded to a
//!   per-sample composition offset; absent ⇒ every sample's offset is `0`.
//! - `stss` (§8.6.2): the sync-sample index list; absent ⇒ every sample is a
//!   sync sample (§8.6.2).
//! - `stsz` (§8.7.3): per-sample sizes, either a uniform `sample_size` or an
//!   explicit per-sample list.
//! - `stsc` (§8.7.4) + `stco`/`co64` (§8.7.5): expanded into a per-chunk
//!   sample count, then walked chunk-by-chunk (each chunk offset is an
//!   *absolute* file byte offset per §8.7.5) to slice each sample's coded
//!   bytes directly out of the input — no separate `mdat` lookup is needed,
//!   because chunk offsets are already file-absolute.
//!
//! This demuxer never reads the `elst` edit list (ISO/IEC 14496-12:2026
//! §8.6.6): the [`Track`]/[`Sample`] IR the tracks are built into carries only
//! decode-order sample timing, matching every other demuxer in this crate — a
//! presentation-timeline edit remains a mux/consumer-side concern.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use broadcast_common::len::need_in;
use core::marker::PhantomData;

use broadcast_common::{Demand, Parse, Stage, Timestamp, Unpackage};

use crate::error::{Error, Result};
use crate::media::{
    Media, SkippedTrack, Track, find_box, normalise_track_nal_lengths, refine_legacy_config,
    skipped_track, track_spec_from_trak,
};
use crate::pipeline::Sample;
use isobmff::init_segment::{MOOV, MovieBox, TrackBox};
use isobmff::sample_table::TrackCursor;

/// Demux a non-fragmented ISOBMFF/MP4 byte stream into a [`Media`].
///
/// Walks the single top-level `moov`: each `trak`'s sample entry supplies the
/// [`CodecConfig`](crate::pipeline::CodecConfig) (reusing
/// [`crate::media::Fmp4Demux`]'s reconstruction), and its `stbl` sample tables
/// supply every coded sample's bytes, duration, composition offset and sync
/// flag, in decode order.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes
/// via [`Unpackage::Input`]; construct one per call with
/// [`ProgressiveDemux::new`].
///
/// `buf`/`media`/`finished` exist only for the [`Stage`] adapter below (media
/// plane step 2e): this demuxer's parse is inherently whole-file (it walks
/// `moov` over the complete input to resolve `stbl`-driven, file-absolute
/// sample offsets), so unlike [`crate::ts_demux::StreamingTsDemux`] there is
/// no incremental parse to drive — `Stage::feed` accumulates bytes up to
/// `max_bytes`, and `Stage::finish` runs the same `demux_progressive` the
/// inherent [`Unpackage::unpackage`] uses, once, over the accumulated buffer.
#[derive(Debug, Clone)]
pub struct ProgressiveDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
    buf: Vec<u8>,
    media: Option<Media>,
    finished: bool,
    /// Set the moment a [`Stage::feed`] is rejected for exceeding
    /// `max_bytes`, and never cleared: that rejection **discarded a chunk of
    /// the file**, so `buf` now has a hole in it.
    ///
    /// This demuxer resolves sample payloads through `stbl`'s *file-absolute*
    /// chunk offsets (ISO/IEC 14496-12:2026 §8.7.5), so parsing a buffer with
    /// a hole does not fail cleanly — every offset past the hole is shifted,
    /// and the parse either reports a misleading [`Error::UnexpectedBox`] or,
    /// far worse, succeeds and returns a [`Media`] whose samples carry the
    /// **wrong bytes**. There is no resynchronisation point to recover from
    /// (unlike the PES/backlog caps elsewhere in this crate), so the only
    /// safe response is to stay in error: once poisoned,
    /// [`feed`](Stage::feed) and [`finish`](Stage::finish) both return the
    /// original cap error and no parse is ever attempted.
    poisoned: bool,
    /// Hard cap on `buf` (issue B7, media plane step 2 fix wave 3): under the
    /// old [`Unpackage`] API the *caller* owned the input buffer, but under
    /// [`Stage`] this type owns it, so the bound is this constructor's
    /// responsibility — there is no `Default`/no-argument constructor that
    /// lets it be omitted (this crate's "no unbounded buffer anywhere" rule).
    /// Unused by the inherent [`Unpackage::unpackage`] path, which borrows
    /// the caller's slice directly and never touches `buf`.
    max_bytes: usize,
}

impl ProgressiveDemux<'_> {
    /// Create a new demuxer whose [`Stage::feed`] buffer is capped at
    /// `max_bytes`: a [`Stage`] driver shovelling network bytes into this
    /// type (Step 3's whole premise) must not be able to grow `buf` without
    /// bound while waiting for the rest of the file to arrive. Exceeding the
    /// cap returns [`Error::BufferCapExceeded`] from [`Stage::feed`] rather
    /// than silently accumulating past it, and **poisons** the demuxer — see
    /// [`Stage::feed`].
    ///
    /// Returns [`Error::InvalidInput`] for `max_bytes == 0`: a zero cap can
    /// never accept a single byte (the very first non-empty `feed` would
    /// exceed it), so a [`Stage`] built from it is permanently saturated and
    /// poisoned before it ever sees data — a wedged demuxer indistinguishable
    /// from a real cap-exceeded failure, for no reason a caller can act on.
    /// Rejecting it at construction, rather than on the first `feed`, means a
    /// caller misconfiguring the bound learns immediately instead of via a
    /// confusing downstream `BufferCapExceeded`. This is the same class of
    /// fix as [`Stage::feed`]'s poison-on-cap-rejection: an unrepresentable
    /// or unusable state should fail at its source, not propagate as a
    /// working-looking value that wedges later. The [`Unpackage::unpackage`]
    /// path is unaffected: it never consults `max_bytes` at all, borrowing
    /// the caller's slice directly.
    pub fn new(max_bytes: usize) -> Result<Self> {
        if max_bytes == 0 {
            return Err(Error::InvalidInput(
                "ProgressiveDemux::new: max_bytes must be non-zero — a zero cap can never accept \
                 any bytes and would permanently wedge the Stage",
            ));
        }
        Ok(Self {
            _marker: PhantomData,
            buf: Vec::new(),
            media: None,
            finished: false,
            poisoned: false,
            max_bytes,
        })
    }

    /// The error a poisoned demuxer keeps returning — see the `poisoned`
    /// field docs for why the poison is permanent.
    fn poison_error(&self) -> Error {
        Error::BufferCapExceeded {
            what: "ProgressiveDemux Stage buffer",
            cap: self.max_bytes,
        }
    }
}

impl<'a> Unpackage for ProgressiveDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        demux_progressive(input)
    }
}

/// Demux a whole non-fragmented ISOBMFF/MP4 byte stream into a [`Media`] —
/// the shared implementation behind both [`Unpackage::unpackage`] (borrowing
/// the caller's buffer directly) and the [`Stage`] adapter (borrowing this
/// type's own accumulated `buf`). Nothing in the returned [`Media`] borrows
/// `input` (every [`Sample`]'s bytes are copied out), so this needs no
/// lifetime tied to the caller's input at all.
fn demux_progressive(input: &[u8]) -> Result<Media> {
    let moov_bytes = find_box(input, &MOOV).ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let moov = MovieBox::parse(moov_bytes)?;
    let movie_timescale = moov.mvhd.timescale;

    // DEMUX = lenient but loud (media plane step-2 fix wave 1, B2/B3): a
    // track whose codec the crate cannot reconstruct, or whose sample tables
    // are incomplete, is skipped rather than failing the whole file —
    // mirrors [`crate::media::Fmp4Demux`]'s per-track handling exactly (the
    // two used to diverge: this demuxer was already lenient, `Fmp4Demux` was
    // fatal on the same input). The caller still learns about it via
    // [`Media::skipped`], never silently.
    let mut tracks = Vec::with_capacity(moov.tracks.len());
    let mut skipped: Vec<SkippedTrack> = Vec::new();
    for trak in &moov.tracks {
        let mut spec = match track_spec_from_trak(trak) {
            Ok(spec) => spec,
            Err(err) => {
                skipped.push(skipped_track(err, trak.tkhd.track_id.into()));
                continue;
            }
        };
        let mut samples = match samples_from_stbl(input, trak) {
            Ok(samples) => samples,
            Err(err) => {
                skipped.push(SkippedTrack {
                    fourcc: String::from("unknown"),
                    reason: err.to_string(),
                    track_id: Some(trak.tkhd.track_id.into()),
                });
                continue;
            }
        };
        refine_legacy_config(&mut spec.config, &samples);
        if let Err(err) = normalise_track_nal_lengths(&mut spec, &mut samples) {
            skipped.push(SkippedTrack {
                fourcc: String::from("unknown"),
                reason: err.to_string(),
                track_id: Some(trak.tkhd.track_id.into()),
            });
            continue;
        }
        tracks.push(Track::new(spec, samples));
    }
    let mut media = Media::new(tracks, movie_timescale);
    media.skipped = skipped;
    Ok(media)
}

/// [`Stage`] adoption (media plane step 2e). `Out = Media` rather than the
/// demux family's [`crate::ir::DemuxEvent`]: this demuxer has no incremental
/// per-track/per-sample discovery to report (see the struct docs) — it always
/// produces the whole parsed [`Media`] atomically at `finish()`, so naming
/// that as the `Out` type is the honest shape, not a manufactured event
/// stream this demuxer doesn't actually have.
impl Stage for ProgressiveDemux<'_> {
    type In<'a> = &'a [u8];
    type Out = Media;
    type Error = Error;

    /// Accumulates `input` — this demuxer cannot parse anything until the
    /// whole file has arrived (see the struct docs), so `feed` never unlocks
    /// output; drain the parsed [`Media`] via [`poll`](Self::poll) after
    /// [`finish`](Self::finish).
    ///
    /// Returns [`Error::BufferCapExceeded`] (issue B7) rather than growing
    /// `buf` past `max_bytes` — a legitimate progressive MP4 the caller
    /// expects to accept must fit under the bound supplied to
    /// [`new`](Self::new); a larger one is rejected outright (this demuxer
    /// has no partial-unit resync point to drop and continue from, unlike
    /// the PES/backlog caps elsewhere in this crate).
    ///
    /// That rejection is **terminal**: the rejected chunk is gone, so `buf`
    /// has a hole, and `stbl`'s file-absolute offsets would resolve every
    /// later sample to the wrong bytes. Every subsequent `feed` — including a
    /// smaller chunk that would otherwise fit — and [`finish`](Self::finish)
    /// return this same error, [`demand`](Self::demand) reports `saturated`,
    /// and no parse is ever attempted. Feeding on regardless used to yield a
    /// spurious [`Error::UnexpectedBox`] or, worse, a `Media` whose samples
    /// silently carried the wrong payloads.
    ///
    /// Also returns it for a `feed` after [`finish`](Self::finish): those
    /// bytes arrive too late for the one whole-file parse this demuxer runs,
    /// so appending them would silently do nothing.
    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<()> {
        if self.poisoned {
            return Err(self.poison_error());
        }
        if self.finished {
            return Err(Error::InvalidInput(
                "ProgressiveDemux::feed after finish: the whole-file parse has already run, so \
                 these bytes would never be parsed",
            ));
        }
        let new_len = self.buf.len().saturating_add(input.len());
        if new_len > self.max_bytes {
            self.poisoned = true;
            return Err(self.poison_error());
        }
        self.buf.extend_from_slice(input);
        Ok(())
    }

    fn poll(&mut self) -> Option<Media> {
        self.media.take()
    }

    /// Runs the whole-file parse once, over every byte accumulated by
    /// [`feed`](Self::feed) so far. Idempotent: a second call does not
    /// re-parse or emit a second [`Media`].
    ///
    /// Returns the original [`Error::BufferCapExceeded`] — and parses nothing
    /// — if a `feed` was ever rejected for exceeding the cap: the accumulated
    /// buffer is missing the rejected chunk, and parsing it would produce
    /// wrong sample payloads rather than a clean failure. See
    /// [`feed`](Self::feed).
    ///
    /// Releases the accumulated `buf` once the parse is done: past `finish`
    /// this demuxer holds only the parsed [`Media`], not that plus a second
    /// whole copy of the file it was built from.
    fn finish(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(self.poison_error());
        }
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let media = demux_progressive(&self.buf);
        // Free the input copy either way: the parse is one-shot, so nothing
        // will read `buf` again.
        self.buf = Vec::new();
        self.media = Some(media?);
        Ok(())
    }

    /// Reports the remaining headroom under `max_bytes` (issue B7): `want_bytes`
    /// is however much of the cap is not yet used, and `saturated` becomes
    /// `true` the moment `buf` reaches `max_bytes` — at which point any
    /// further [`feed`](Self::feed) call returns [`Error::BufferCapExceeded`]
    /// rather than growing the buffer, so a cooperative driver should stop
    /// feeding once this flips.
    ///
    /// Also `saturated` once the demuxer is poisoned (a cap rejection
    /// happened) or finished — in both states every further `feed` is an
    /// error, so continuing to advertise headroom would invite a driver to
    /// keep pushing bytes this demuxer will never accept.
    fn demand(&self) -> Demand {
        if self.poisoned || self.finished {
            return Demand::saturated();
        }
        let remaining = self.max_bytes.saturating_sub(self.buf.len());
        if remaining == 0 {
            Demand::saturated()
        } else {
            Demand::new(remaining)
        }
    }
}

/// Build one track's decode-ordered [`Sample`]s from its `stbl` sample tables,
/// slicing coded bytes directly out of `file` via the (file-absolute) chunk
/// offsets.
///
/// The run-length sample tables are walked by
/// [`isobmff::sample_table::TrackCursor`]; nothing here expands them to one
/// entry per sample, and a hostile table is a typed `isobmff::Error` before a
/// single byte is copied.
fn samples_from_stbl(file: &[u8], trak: &TrackBox) -> Result<Vec<Sample>> {
    let stbl = trak
        .mdia
        .as_ref()
        .and_then(|m| m.minf.as_ref())
        .and_then(|m| m.stbl.as_ref())
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    let file_len =
        u64::try_from(file.len()).map_err(|_| Error::InvalidInput("input length exceeds u64"))?;
    let cursor = TrackCursor::new(
        stbl,
        trak.tkhd.track_id,
        &isobmff::Limits::default(),
        Some(file_len),
    )?;
    let capacity = usize::try_from(cursor.len()).unwrap_or(0);
    let mut samples = Vec::with_capacity(capacity);
    for item in cursor {
        let sample = item?;
        let start = usize::try_from(sample.offset)
            .map_err(|_| Error::InvalidInput("sample offset exceeds addressable range"))?;
        let end = start
            .checked_add(sample.size as usize)
            .ok_or(Error::InvalidInput("sample byte range overflow"))?;
        need_in(file, end, "progressive sample data")?;
        let dts = i64::try_from(sample.decode_time)
            .map_err(|_| Error::InvalidInput("decode time exceeds i64"))?;
        samples.push(Sample::new(
            file[start..end].to_vec(),
            Some(dts),
            Some(dts + i64::from(sample.composition_offset)),
            Some(sample.duration),
            sample.sync,
        )?);
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ProgressiveDemux::new(0)` must be rejected outright: a zero cap can
    /// never accept a byte, so a `Stage` built from it would be permanently
    /// saturated/poisoned before ever seeing data (R1).
    #[test]
    fn new_zero_cap_is_rejected() {
        let err =
            ProgressiveDemux::new(0).expect_err("a zero cap must be rejected at construction");
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected Error::InvalidInput for a zero cap, got {err:?}"
        );
    }

    /// A non-zero cap still constructs successfully and the `Stage` accepts
    /// bytes under it.
    #[test]
    fn new_nonzero_cap_still_works() {
        use broadcast_common::{Stage, Timestamp};

        let mut demux = ProgressiveDemux::new(16).expect("non-zero cap must construct");
        Stage::feed(&mut demux, &[0u8; 4], Timestamp::ZERO).expect("feed under the cap fits");
        assert!(
            !Stage::demand(&demux).saturated,
            "headroom remains under the cap"
        );
    }
}
