//! Progressive (single-file, non-fragmented) MP4 demux — ISO/IEC
//! 14496-12:2015 §8.5–§8.7.
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
//!   [`crate::timing::CompositionOffsetBox`]'s own convention), expanded to a
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
//! This demuxer never reads the `elst` edit list (ISO/IEC 14496-12:2015
//! §8.6.6): the [`Track`]/[`Sample`] IR the tracks are built into carries only
//! decode-order sample timing, matching every other demuxer in this crate — a
//! presentation-timeline edit remains a mux/consumer-side concern.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Demand, Parse, Stage, Timestamp, Unpackage};

use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, MovieBox, SampleSizeBox, SampleToChunkBox, StblChild,
    SyncSampleBox, TrackBox,
};
use crate::media::{
    Media, SkippedTrack, Track, find_top_box, normalise_track_nal_lengths, refine_legacy_config,
    skipped_track, track_spec_from_trak,
};
use crate::pipeline::Sample;
use crate::timing::{CompositionOffsetBox, TimeToSampleBox};

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
    /// chunk offsets (ISO/IEC 14496-12:2015 §8.7.5), so parsing a buffer with
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
    let moov_bytes =
        find_top_box(input, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
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
                skipped.push(skipped_track(err));
                continue;
            }
        };
        let mut samples = match samples_from_stbl(input, trak) {
            Ok(samples) => samples,
            Err(err) => {
                skipped.push(SkippedTrack {
                    fourcc: String::from("unknown"),
                    reason: err.to_string(),
                });
                continue;
            }
        };
        refine_legacy_config(&mut spec.config, &samples);
        if let Err(err) = normalise_track_nal_lengths(&mut spec, &mut samples) {
            skipped.push(SkippedTrack {
                fourcc: String::from("unknown"),
                reason: err.to_string(),
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

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

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

/// Look up a typed `stbl` child, e.g. `StblChild::Stts`.
///
/// Tries `variant` first. If that misses, checks whether there's a
/// same-four-CC [`StblChild::Opaque`] instead — meaning the box is *present*
/// but `init_segment::parse_stbl_children` couldn't parse it — and if so,
/// re-parses those bytes to recover and return the real parse error, rather
/// than falling through to the generic "absent" case. Without this, a
/// corrupt `stts`/`ctts`/… would silently behave exactly like a well-formed
/// empty or missing box (issue #952 for `stsd`; audit finding #3 for the
/// remaining `stbl` children): a mandatory box (`stts`/`stsz`/`stsc`) would
/// report a misleading "expected stts" instead of the actual cause, and an
/// optional box (`ctts`/`stss`/`stco`/`co64`) would be treated as *legitimately
/// absent* — e.g. a corrupt `ctts` silently presenting every composition
/// offset as `0` instead of failing the track loud enough to reach
/// [`Media::skipped`](crate::media::Media::skipped).
fn find_stbl_child<'a, T>(
    children: &'a [StblChild],
    fourcc: &[u8; 4],
    variant: impl Fn(&'a StblChild) -> Option<&'a T>,
    reparse_err: impl Fn(&'a [u8]) -> Error,
) -> Result<Option<&'a T>> {
    for child in children {
        if let Some(t) = variant(child) {
            return Ok(Some(t));
        }
    }
    for child in children {
        if let StblChild::Opaque(bytes) = child
            && bytes.len() >= 8
            && &bytes[4..8] == fourcc
        {
            return Err(reparse_err(bytes));
        }
    }
    Ok(None)
}

/// Build one track's decode-ordered [`Sample`]s from its `stbl` sample tables,
/// slicing coded bytes directly out of `file` via the (file-absolute) chunk
/// offsets.
fn samples_from_stbl(file: &[u8], trak: &TrackBox) -> Result<Vec<Sample>> {
    let stbl = trak
        .mdia
        .as_ref()
        .and_then(|m| m.minf.as_ref())
        .and_then(|m| m.stbl.as_ref())
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;

    let stts = find_stbl_child(
        &stbl.children,
        b"stts",
        |c| match c {
            StblChild::Stts(b) => Some(b),
            _ => None,
        },
        |bytes| TimeToSampleBox::parse(bytes).unwrap_err(),
    )?
    .ok_or(Error::UnexpectedBox { expected: "stts" })?;
    let ctts = find_stbl_child(
        &stbl.children,
        b"ctts",
        |c| match c {
            StblChild::Ctts(b) => Some(b),
            _ => None,
        },
        |bytes| CompositionOffsetBox::parse(bytes).unwrap_err(),
    )?;
    let stss = find_stbl_child(
        &stbl.children,
        b"stss",
        |c| match c {
            StblChild::Stss(b) => Some(b),
            _ => None,
        },
        |bytes| SyncSampleBox::parse(bytes).unwrap_err(),
    )?;
    let stsz = find_stbl_child(
        &stbl.children,
        b"stsz",
        |c| match c {
            StblChild::Stsz(b) => Some(b),
            _ => None,
        },
        |bytes| SampleSizeBox::parse(bytes).unwrap_err(),
    )?
    .ok_or(Error::UnexpectedBox { expected: "stsz" })?;
    let stsc = find_stbl_child(
        &stbl.children,
        b"stsc",
        |c| match c {
            StblChild::Stsc(b) => Some(b),
            _ => None,
        },
        |bytes| SampleToChunkBox::parse(bytes).unwrap_err(),
    )?
    .ok_or(Error::UnexpectedBox { expected: "stsc" })?;
    let co64 = find_stbl_child(
        &stbl.children,
        b"co64",
        |c| match c {
            StblChild::Co64(b) => Some(b),
            _ => None,
        },
        |bytes| ChunkLargeOffsetBox::parse(bytes).unwrap_err(),
    )?;
    let stco = find_stbl_child(
        &stbl.children,
        b"stco",
        |c| match c {
            StblChild::Stco(b) => Some(b),
            _ => None,
        },
        |bytes| ChunkOffsetBox::parse(bytes).unwrap_err(),
    )?;

    let layout = sample_layout(stsz, stsc, co64, stco, file.len())?;
    let total_samples = layout.len();
    let durations = expand_stts(stts, total_samples)?;
    let composition_offsets = expand_ctts(ctts, total_samples)?;
    let sync_flags = expand_stss(stss, total_samples);

    let mut samples = Vec::with_capacity(total_samples);
    // Absolute decode time (media plane step 2c): a progressive movie's media
    // timeline starts at 0 and `stts` carries per-sample decode *deltas*
    // (ISO/IEC 14496-12:2015 §8.6.1.2), so sample `i`'s absolute DTS is the
    // running sum of the preceding deltas; PTS folds in the `ctts`
    // composition offset (§8.6.1.3).
    let mut next_dts: i64 = 0;
    for i in 0..total_samples {
        let (start, size) = layout[i];
        let end = start
            .checked_add(size)
            .ok_or(Error::InvalidInput("sample byte range overflow"))?;
        if end > file.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: file.len(),
                what: "progressive sample data",
            });
        }
        let dts = next_dts;
        samples.push(Sample::new(
            file[start..end].to_vec(),
            Some(dts),
            Some(dts + composition_offsets[i] as i64),
            Some(durations[i]),
            sync_flags[i],
        ));
        next_dts += durations[i] as i64;
    }
    Ok(samples)
}

#[cfg(test)]
thread_local! {
    /// Work counter for [`sample_layout`]'s single forward pass: chunks visited
    /// plus `stsc` entries stepped over (tests pin it linear in both).
    static LAYOUT_STEPS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Resolve every sample's `(absolute_file_offset, size)` from the sample
/// tables `stsz` + `stsc` + `stco`/`co64`.
///
/// The one stbl sample-layout expander: [`ProgressiveDemux`] and the
/// protected-progressive path of `cenc_decrypt` both go through it (audit
/// r05-O1 / #1141).
///
/// **One rule for uniform and per-sample `stsz`:** `stsz.sample_count` is
/// authoritative. Surplus `stsc`/`stco` capacity (a last chunk shorter than
/// `samples_per_chunk`, or more chunks than needed) is *tolerated* — the
/// layout is clamped to `sample_count`. Only a *shortfall* (the chunk tables
/// cover fewer samples than `stsz` declares) is an error. `sample_count` is
/// bounded by `file_len` first (no sample occupies fewer than one byte,
/// #987), so a hostile table cannot drive a large allocation.
///
/// `stsc.first_chunk` must be >= 1 and strictly ascending (ISO/IEC 14496-12
/// §8.7.4); anything else is a structured `InvalidValue`. That ordering is
/// what lets the expansion be one forward pass, linear in chunks + entries.
pub(crate) fn sample_layout(
    stsz: &SampleSizeBox,
    stsc: &SampleToChunkBox,
    co64: Option<&ChunkLargeOffsetBox>,
    stco: Option<&ChunkOffsetBox>,
    file_len: usize,
) -> Result<Vec<(usize, usize)>> {
    let chunk_offsets = chunk_offsets(co64, stco)?;
    let declared = stsz.sample_count as usize;
    if declared > file_len {
        return Err(Error::BufferTooShort {
            need: declared,
            have: file_len,
            what: "stsz sample_count vs file length",
        });
    }
    let entries = &stsc.entries;
    let mut prev_first = 0u32;
    for e in entries {
        if e.first_chunk <= prev_first {
            return Err(Error::InvalidValue {
                field: "stsc.first_chunk",
                value: u64::from(e.first_chunk),
                reason: "first_chunk must be >= 1 and strictly ascending (ISO/IEC 14496-12 §8.7.4)",
            });
        }
        prev_first = e.first_chunk;
    }

    let mut layout = Vec::with_capacity(declared);
    // Index of the stsc run applying to the current chunk, once one starts.
    let mut run = 0usize;
    for (c, &chunk_base) in chunk_offsets.iter().enumerate() {
        if layout.len() == declared {
            break;
        }
        #[cfg(test)]
        LAYOUT_STEPS.with(|n| n.set(n.get() + 1));
        let chunk_no = c as u64 + 1;
        while run + 1 < entries.len() && u64::from(entries[run + 1].first_chunk) <= chunk_no {
            #[cfg(test)]
            LAYOUT_STEPS.with(|n| n.set(n.get() + 1));
            run += 1;
        }
        let per_chunk = match entries.get(run) {
            Some(e) if u64::from(e.first_chunk) <= chunk_no => e.samples_per_chunk,
            _ => 0,
        };
        let mut cursor = chunk_base;
        for _ in 0..per_chunk {
            if layout.len() == declared {
                break;
            }
            let size = sample_size(stsz, layout.len())?;
            let start = usize::try_from(cursor)
                .map_err(|_| Error::InvalidInput("chunk offset exceeds addressable range"))?;
            layout.push((start, size));
            cursor += size as u64;
        }
    }
    if layout.len() < declared {
        return Err(Error::InvalidInput(
            "stsc/stco sample-to-chunk mapping did not cover all samples",
        ));
    }
    Ok(layout)
}

/// Resolve the per-chunk absolute file byte offsets, preferring `co64`
/// (64-bit, §8.7.5) over `stco` (32-bit) when both are present (well-formed
/// files carry exactly one).
fn chunk_offsets(
    co64: Option<&ChunkLargeOffsetBox>,
    stco: Option<&ChunkOffsetBox>,
) -> Result<Vec<u64>> {
    if let Some(co64) = co64 {
        Ok(co64.entries.clone())
    } else if let Some(stco) = stco {
        Ok(stco.entries.iter().map(|&o| o as u64).collect())
    } else {
        Err(Error::UnexpectedBox {
            expected: "stco or co64",
        })
    }
}

/// Resolve one sample's byte size from `stsz` (§8.7.3): the uniform
/// `sample_size` when non-zero, else the per-sample `entries[index]`.
fn sample_size(stsz: &SampleSizeBox, index: usize) -> Result<usize> {
    if stsz.sample_size != 0 {
        Ok(stsz.sample_size as usize)
    } else {
        stsz.entries
            .get(index)
            .map(|&s| s as usize)
            .ok_or(Error::InvalidInput("stsz has fewer entries than samples"))
    }
}

/// Expand `stts`'s run-length `(sample_count, sample_delta)` table (§8.6.1.2)
/// into an explicit per-sample duration.
fn expand_stts(stts: &TimeToSampleBox, total_samples: usize) -> Result<Vec<u32>> {
    let mut out = Vec::with_capacity(total_samples);
    for entry in &stts.entries {
        // A wire `sample_count` is untrusted: a run longer than the slots
        // left can never be valid, so reject it before pushing rather than
        // after the loop — a ~100-byte file declaring 0xFFFFFFFF here used
        // to push ~17 GB first (r04-C5). Same error as the mismatch case
        // below.
        if entry.sample_count as usize > total_samples - out.len() {
            return Err(Error::InvalidInput(
                "stts sample count does not match chunk layout",
            ));
        }
        for _ in 0..entry.sample_count {
            out.push(entry.sample_delta);
        }
    }
    if out.len() != total_samples {
        return Err(Error::InvalidInput(
            "stts sample count does not match chunk layout",
        ));
    }
    Ok(out)
}

/// Expand `ctts`'s run-length `(sample_count, sample_offset)` table
/// (§8.6.1.3) into an explicit per-sample composition offset; `None` (no
/// `ctts`) yields all-zero offsets (every sample's CT == DT).
fn expand_ctts(ctts: Option<&CompositionOffsetBox>, total_samples: usize) -> Result<Vec<i32>> {
    let Some(ctts) = ctts else {
        return Ok(alloc::vec![0i32; total_samples]);
    };
    let mut out = Vec::with_capacity(total_samples);
    for entry in &ctts.entries {
        // Same r04-C5 bound as `expand_stts`: reject an over-long run before
        // pushing, not after the loop.
        if entry.sample_count as usize > total_samples - out.len() {
            return Err(Error::InvalidInput(
                "ctts sample count does not match chunk layout",
            ));
        }
        for _ in 0..entry.sample_count {
            out.push(entry.sample_offset);
        }
    }
    if out.len() != total_samples {
        return Err(Error::InvalidInput(
            "ctts sample count does not match chunk layout",
        ));
    }
    Ok(out)
}

/// Resolve every sample's sync flag from `stss`'s 1-based index list
/// (§8.6.2); absent ⇒ every sample is implicitly a sync sample.
fn expand_stss(stss: Option<&SyncSampleBox>, total_samples: usize) -> Vec<bool> {
    let Some(stss) = stss else {
        return alloc::vec![true; total_samples];
    };
    let mut flags = alloc::vec![false; total_samples];
    for &one_based in &stss.entries {
        let idx = one_based as usize;
        if idx >= 1 && idx <= total_samples {
            flags[idx - 1] = true;
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_segment::StscEntry;

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

    // -----------------------------------------------------------------------
    // find_stbl_child (audit finding #3)
    // -----------------------------------------------------------------------

    /// A genuinely malformed `stsc`: a bare 8-byte header, below
    /// `SampleToChunkBox::parse`'s own 16-byte minimum, so it is guaranteed
    /// to fail to parse.
    fn malformed_stsc_opaque() -> StblChild {
        let mut raw = alloc::vec![0u8; 8];
        raw[0..4].copy_from_slice(&8u32.to_be_bytes());
        raw[4..8].copy_from_slice(b"stsc");
        StblChild::Opaque(raw)
    }

    fn stsc_variant(c: &StblChild) -> Option<&SampleToChunkBox> {
        match c {
            StblChild::Stsc(b) => Some(b),
            _ => None,
        }
    }

    fn stsc_reparse_err(bytes: &[u8]) -> Error {
        SampleToChunkBox::parse(bytes).unwrap_err()
    }

    /// The typed variant, when present, is returned as-is — no Opaque
    /// fallback is even consulted.
    #[test]
    fn find_stbl_child_returns_typed_when_present() {
        let stsc = SampleToChunkBox {
            version: 0,
            flags: 0,
            entries: Vec::new(),
        };
        let children = alloc::vec![StblChild::Stsc(stsc.clone())];
        let found = find_stbl_child(&children, b"stsc", stsc_variant, stsc_reparse_err)
            .expect("typed present must not error");
        assert_eq!(found, Some(&stsc));
    }

    /// A same-fourcc `Opaque` box (present but failed to parse —
    /// `init_segment::parse_stbl_children`'s outcome for a malformed box)
    /// must surface its real parse error, not be treated as "absent". Before
    /// this helper existed, `init_segment::parse_stbl_children` defaulted the
    /// box to an empty typed box instead, so a corrupt `stsc` silently
    /// behaved as a well-formed one with zero entries (audit finding #3).
    #[test]
    fn find_stbl_child_recovers_real_error_from_matching_opaque() {
        let children = alloc::vec![malformed_stsc_opaque()];
        let err = find_stbl_child(&children, b"stsc", stsc_variant, stsc_reparse_err)
            .expect_err("a matching Opaque box must surface its real parse error");
        assert!(
            matches!(err, Error::BufferTooShort { .. }),
            "expected the real BufferTooShort from SampleToChunkBox::parse, got {err:?}"
        );
    }

    /// An `Opaque` box for a *different* four-CC must not be mistaken for
    /// this box's parse failure — a genuinely absent box still resolves to
    /// `Ok(None)`.
    #[test]
    fn find_stbl_child_returns_none_for_non_matching_opaque() {
        let mut raw = alloc::vec![0u8; 8];
        raw[0..4].copy_from_slice(&8u32.to_be_bytes());
        raw[4..8].copy_from_slice(b"stsz");
        let children = alloc::vec![StblChild::Opaque(raw)];

        let found = find_stbl_child(&children, b"stsc", stsc_variant, stsc_reparse_err)
            .expect("a non-matching Opaque box must not be treated as this box's error");
        assert!(found.is_none());
    }

    // -----------------------------------------------------------------------
    // sample_layout
    // -----------------------------------------------------------------------

    fn run(first_chunk: u32, samples_per_chunk: u32) -> StscEntry {
        StscEntry {
            first_chunk,
            samples_per_chunk,
            sample_description_index: 1,
        }
    }

    fn stsc_of(entries: Vec<StscEntry>) -> SampleToChunkBox {
        SampleToChunkBox {
            version: 0,
            flags: 0,
            entries,
        }
    }

    fn stco_of(offsets: &[u32]) -> ChunkOffsetBox {
        ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries: offsets.to_vec(),
        }
    }

    fn per_sample_stsz(sizes: &[u32]) -> SampleSizeBox {
        SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: 0,
            sample_count: sizes.len() as u32,
            entries: sizes.to_vec(),
        }
    }

    fn uniform_stsz(size: u32, count: u32) -> SampleSizeBox {
        SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: size,
            sample_count: count,
            entries: Vec::new(),
        }
    }

    /// (a) The last chunk holds fewer samples than `samples_per_chunk`
    /// (surplus capacity) with a per-sample `stsz`: tolerated.
    #[test]
    fn layout_tolerates_surplus_capacity_per_sample_stsz() {
        // 3 per chunk x 2 chunks = capacity 6, but only 5 samples declared.
        let stsz = per_sample_stsz(&[10, 20, 30, 40, 50]);
        let layout = sample_layout(
            &stsz,
            &stsc_of(alloc::vec![run(1, 3)]),
            None,
            Some(&stco_of(&[100, 1000])),
            1 << 20,
        )
        .unwrap();
        assert_eq!(
            layout,
            [(100, 10), (110, 20), (130, 30), (1000, 40), (1040, 50)]
        );
    }

    /// (b) Same surplus, uniform `stsz`: clamped to `sample_count`, same rule.
    #[test]
    fn layout_tolerates_surplus_capacity_uniform_stsz() {
        let layout = sample_layout(
            &uniform_stsz(8, 4),
            &stsc_of(alloc::vec![run(1, 3)]),
            None,
            Some(&stco_of(&[0, 100, 200])),
            1 << 20,
        )
        .unwrap();
        assert_eq!(layout, [(0, 8), (8, 8), (16, 8), (100, 8)]);
    }

    /// (c) The chunk tables cover fewer samples than `stsz` declares.
    #[test]
    fn layout_rejects_shortfall() {
        for stsz in [uniform_stsz(8, 5), per_sample_stsz(&[8; 5])] {
            let err = sample_layout(
                &stsz,
                &stsc_of(alloc::vec![run(1, 2)]),
                None,
                Some(&stco_of(&[0, 100])),
                1 << 20,
            )
            .unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
        }
    }

    /// `first_chunk` must be strictly ascending (§8.7.4): a repeat, a
    /// descent and a zero are each a structured `InvalidValue` (this replaces
    /// the W22 clamp test: the oversized/alternating entries it fed are now
    /// rejected outright rather than iterated).
    #[test]
    fn layout_rejects_non_ascending_first_chunk() {
        for entries in [
            alloc::vec![run(1, 2), run(0xFFFF_FFFF, 3), run(1, 4)],
            alloc::vec![run(2, 1), run(2, 1)],
            alloc::vec![run(0, 1)],
        ] {
            let err = sample_layout(
                &uniform_stsz(1, 1),
                &stsc_of(entries),
                None,
                Some(&stco_of(&[0])),
                1 << 20,
            )
            .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::InvalidValue {
                        field: "stsc.first_chunk",
                        ..
                    }
                ),
                "{err:?}"
            );
        }
    }

    /// (d) Work bound: one `stsc` run per chunk is one forward pass — steps
    /// stay linear in chunks + entries (a per-chunk rescan of the run table
    /// would be ~chunks^2/2 = 80 000 here).
    #[test]
    fn layout_work_is_linear_in_chunks_and_entries() {
        const CHUNKS: u32 = 400;
        let entries: Vec<StscEntry> = (1..=CHUNKS).map(|c| run(c, 1)).collect();
        let offsets: Vec<u32> = (1..=CHUNKS).map(|c| c * 100).collect();
        LAYOUT_STEPS.with(|n| n.set(0));
        let layout = sample_layout(
            &uniform_stsz(10, CHUNKS),
            &stsc_of(entries),
            None,
            Some(&stco_of(&offsets)),
            1 << 20,
        )
        .unwrap();
        let steps = LAYOUT_STEPS.with(core::cell::Cell::get);
        assert_eq!(layout[0], (100, 10));
        assert_eq!(layout[CHUNKS as usize - 1], (CHUNKS as usize * 100, 10));
        assert!(steps <= 2 * CHUNKS as usize, "steps {steps}");
    }
}
