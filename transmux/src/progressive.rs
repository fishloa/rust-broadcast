//! Progressive (single-file, non-fragmented) MP4 packager — ISO/IEC 14496-12:2015 §8.
//!
//! [`ProgressiveMux`] muxes the crate's [`Media`] IR into one **non-fragmented**
//! `.mp4` file: an `ftyp`, a single `moov` carrying per-track `trak` boxes with
//! the *full* sample tables (`stbl`: `stsd`/`stts`/`ctts`/`stsc`/`stsz`/
//! `stco`|`co64`/`stss`, ISO/IEC 14496-12:2015 §8.5–§8.7), and a single `mdat`
//! holding every track's sample data concatenated. This is the VOD/download
//! counterpart to the fragmented [`crate::media::CmafMux`].
//!
//! Sample tables are derived directly from the sample stream (§8.6/§8.7):
//! decode deltas — each sample's own `dts` to the next, so a gap survives —
//! are run-length coded into `stts` (§8.6.1.2); each track is placed on the
//! movie timeline, relative to the earliest track, by an `elst` (§8.6.6)
//! whenever it starts later or opens with a composition lead-in; composition
//! offsets go into `ctts` (§8.6.1.3) only when some sample has a non-zero
//! offset; per-sample byte sizes fill `stsz` (§8.7.3); a single-chunk-per-track
//! `stsc` (§8.7.4) maps samples to chunks; each track's chunk byte offset lands
//! in `stco` (§8.7.5) — promoted to the 64-bit `co64` when any offset exceeds
//! [`u32::MAX`]; and the sync-sample list is emitted as `stss` (§8.6.2), omitted
//! when every sample is a sync sample.
//!
//! With [`ProgressiveMux::faststart`] set, `moov` is written **before** `mdat`
//! for progressive-download friendliness. Because the `stco`/`co64` chunk
//! offsets are absolute file offsets that depend on the `moov` size, the mux
//! runs two passes: it lays out the `moov` (whose size is independent of the
//! offset *values*), computes the final chunk offsets against the resulting
//! `mdat` position, then re-serialises. When faststart is `false`, `mdat`
//! precedes `moov` and the same offset arithmetic applies.
//!
//! The per-codec sample entries + config boxes are reused verbatim from
//! [`crate::pipeline::build_init_segment`] (the record is rebuilt from the
//! [`TrackSpec`](crate::pipeline::TrackSpec), never copied from an input file).

use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::{Package, Parse, Serialize};

use crate::box_types::{BoxHeader, BoxType};
use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, EditBox, MovieBox, SampleSizeBox, SampleToChunkBox,
    StblChild, StscEntry, SyncSampleBox,
};
use crate::media::{Media, TimelineOrigin, Track, relative_decode_times};
use crate::pipeline::{Sample, build_init_segment};
use crate::segments::{FileTypeBox, MediaDataBox};
use crate::timing::{
    CompositionOffsetBox, CttsEntry, EditListBox, EditListEntry, SttsEntry, TimeToSampleBox,
};

/// Default movie timescale used when a [`Media`] does not specify one.
const DEFAULT_MOVIE_TIMESCALE: u32 = 1000;
/// `ftyp` major brand for a progressive (non-fragmented) MP4.
const FTYP_MAJOR_BRAND: [u8; 4] = *b"isom";
/// `ftyp` minor version.
const FTYP_MINOR_VERSION: u32 = 512;
/// The `mdat` box type (ISO/IEC 14496-12:2015 §8.1.1).
const MDAT_TYPE: [u8; 4] = *b"mdat";
/// The `default_sample_description_index` referenced by the single `stsd` entry.
const SAMPLE_DESCRIPTION_INDEX: u32 = 1;
/// `elst` `media_time` of an empty edit (ISO/IEC 14496-12:2015 §8.6.6).
const ELST_EMPTY_EDIT: i64 = -1;
/// `elst` `media_rate_integer` for normal-speed playback (§8.6.6).
const ELST_NORMAL_RATE: i16 = 1;

/// Package a [`Media`] into a single-file, non-fragmented `.mp4`.
///
/// Implements [`broadcast_common::Package`] with `Output = Vec<u8>`: the whole
/// file is returned as one byte vector (`ftyp` + `moov` + `mdat`).
#[derive(Debug, Clone, Default)]
pub struct ProgressiveMux {
    /// When `true`, place `moov` before `mdat` (progressive-download friendly).
    /// When `false`, `mdat` precedes `moov`.
    pub faststart: bool,
}

impl ProgressiveMux {
    /// Create a muxer with the given faststart preference.
    pub fn new(faststart: bool) -> Self {
        Self { faststart }
    }
}

/// Each sample's decode delta (§8.6.1.2): the gap to the next sample's decode
/// time, so a gap in the IR's own `dts` timeline is kept; the last sample (and
/// any sample whose next decode time goes backwards or is unknown) uses its own
/// duration.
fn decode_deltas(samples: &[Sample], times: &[i64]) -> Vec<u32> {
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let own = s.duration.unwrap_or(0);
            match (times.get(i), times.get(i + 1)) {
                (Some(&now), Some(&next)) if next >= now => {
                    u32::try_from(next - now).unwrap_or(own)
                }
                _ => own,
            }
        })
        .collect()
}

/// Run-length code a per-sample decode-delta list into `stts` entries
/// (ISO/IEC 14496-12:2015 §8.6.1.2): consecutive equal deltas collapse into a
/// single `(sample_count, sample_delta)` run.
fn build_stts(deltas: &[u32]) -> TimeToSampleBox {
    let mut entries: Vec<SttsEntry> = Vec::new();
    for &delta in deltas {
        match entries.last_mut() {
            Some(last) if last.sample_delta == delta => last.sample_count += 1,
            _ => entries.push(SttsEntry {
                sample_count: 1,
                sample_delta: delta,
            }),
        }
    }
    TimeToSampleBox {
        version: 0,
        flags: 0,
        entries,
    }
}

/// Build the `ctts` composition-offset table (§8.6.1.3), run-length coded, or
/// `None` when every sample has a zero composition offset (in which case the box
/// is omitted per §8.6.1.3). Uses version 1 (signed offsets) to support
/// negative offsets from B-frame reordering.
fn build_ctts(samples: &[Sample]) -> Option<CompositionOffsetBox> {
    if samples.iter().all(|s| s.composition_offset() == 0) {
        return None;
    }
    let mut entries: Vec<CttsEntry> = Vec::new();
    for s in samples {
        match entries.last_mut() {
            Some(last) if last.sample_offset == s.composition_offset() => last.sample_count += 1,
            _ => entries.push(CttsEntry {
                sample_count: 1,
                sample_offset: s.composition_offset(),
            }),
        }
    }
    Some(CompositionOffsetBox {
        version: 1,
        flags: 0,
        entries,
    })
}

/// Build the `stss` sync-sample list (§8.6.2) of 1-based sample indices, or
/// `None` when every sample is a sync sample (then the box is omitted and all
/// samples are implicitly random-access points).
fn build_stss(samples: &[Sample]) -> Option<SyncSampleBox> {
    if samples.iter().all(|s| s.flags.is_sync) {
        return None;
    }
    let entries: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if s.flags.is_sync {
                Some(i as u32 + 1)
            } else {
                None
            }
        })
        .collect();
    Some(SyncSampleBox {
        version: 0,
        flags: 0,
        entries,
    })
}

/// Assemble the ordered `stbl` children for one track from its samples, given
/// the already-resolved absolute `chunk_offset` (the file position of this
/// track's single chunk) and the existing `stsd` child (reused verbatim).
///
/// One chunk per track keeps `stsc` trivial and correct (§8.7.4). `use_co64`
/// selects the 64-bit `co64` chunk-offset box (§8.7.5) over the 32-bit `stco`;
/// it is chosen once for the whole file so the `moov` size stays offset-stable.
fn build_stbl_children(
    stsd: StblChild,
    samples: &[Sample],
    deltas: &[u32],
    chunk_offsets: &[u64],
    samples_per_chunk: &[u32],
    use_co64: bool,
) -> Result<Vec<StblChild>> {
    let stsz_entries: Vec<u32> = samples.iter().map(|s| s.data.len() as u32).collect();
    let stsz = SampleSizeBox {
        version: 0,
        flags: 0,
        sample_size: 0,
        sample_count: stsz_entries.len() as u32,
        entries: stsz_entries,
    };
    // `stsc` runs: one entry per distinct `samples_per_chunk`, keyed by the
    // 1-based first chunk it applies to (§8.7.4). An interleaved file gives
    // each track several equal-sized chunks, so the whole run is one entry.
    let mut stsc_entries: Vec<StscEntry> = Vec::new();
    for (i, &count) in samples_per_chunk.iter().enumerate() {
        let first_chunk = broadcast_common::len::fit_u32(i + 1, "first_chunk")?;
        match stsc_entries.last_mut() {
            Some(last) if last.samples_per_chunk == count => {}
            _ => stsc_entries.push(StscEntry {
                first_chunk,
                samples_per_chunk: count,
                sample_description_index: SAMPLE_DESCRIPTION_INDEX,
            }),
        }
    }
    let stsc = SampleToChunkBox {
        version: 0,
        flags: 0,
        entries: stsc_entries,
    };

    let mut children = vec![
        stsd,
        StblChild::Stts(build_stts(deltas)),
        StblChild::Stsc(stsc),
        StblChild::Stsz(stsz),
    ];
    if let Some(ctts) = build_ctts(samples) {
        // ctts follows stts in the recommended stbl order (§8.5.1).
        children.insert(2, StblChild::Ctts(ctts));
    }
    if use_co64 {
        children.push(StblChild::Co64(ChunkLargeOffsetBox {
            version: 0,
            flags: 0,
            entries: chunk_offsets.to_vec(),
        }));
    } else {
        let entries: Vec<u32> = chunk_offsets
            .iter()
            .map(|&o| {
                u32::try_from(o).map_err(|_| {
                    Error::InvalidInput("progressive: a chunk offset exceeds the 32-bit stco field")
                })
            })
            .collect::<Result<_>>()?;
        children.push(StblChild::Stco(ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries,
        }));
    }
    if let Some(stss) = build_stss(samples) {
        children.push(StblChild::Stss(stss));
    }
    Ok(children)
}

/// Order the per-track chunks into the `mdat` interleave.
///
/// The cross-track order is by start tick; the per-track order is **sample
/// order**, which the tick cannot express when a track's decode times step
/// backwards (a TS discontinuity, a spliced `Media`) — keying on
/// `(tick, track, first sample)` then reordered a track's own chunks and the
/// merge rejected legal input (audit item 5, round 3). Ties break on
/// `(track, first sample)` so the result is deterministic.
fn interleave_order(pending: Vec<(i64, usize, usize, usize)>) -> Vec<(i64, usize, usize, usize)> {
    // First give each track's own chunks their sample order, then interleave
    // the tracks by tick. Two stable sorts: the first is a no-op for the
    // sample-ordered list `plan_mdat_layout` builds, but pinning it here means
    // the invariant holds no matter how the caller produced `pending`. The
    // second (tick) is stable, so chunks with equal ticks keep the
    // sample-ordered sequence the first pass established.
    //
    // The alternative — one key of `(tick, track, first)` — interleaves *and*
    // orders per track in one step, so a track whose chunk-start ticks run
    // backwards (a non-monotonic `dts`) has its own chunks reordered and the
    // merge's cursor then rejects legal input (audit item 5, round 3).
    // A tick-major order cannot express a track whose own chunk ticks run
    // backwards, so the merge is built by *merging* the per-track sequences
    // rather than sorting a flat list: each track is already in sample order,
    // and the next chunk emitted is the one with the smallest start tick among
    // the tracks' current cursors. Ties break on track index, so the result is
    // deterministic.
    let mut per_track: alloc::collections::BTreeMap<usize, Vec<(i64, usize, usize, usize)>> =
        alloc::collections::BTreeMap::new();
    for entry in pending {
        per_track.entry(entry.1).or_default().push(entry);
    }
    let mut heads: alloc::vec::Vec<usize> = alloc::vec![0; per_track.len()];
    let mut out = Vec::new();
    loop {
        let mut best: Option<(usize, i64)> = None;
        for (&track, chunks) in &per_track {
            let Some(head) = chunks.get(heads[track]) else {
                continue;
            };
            // `per_track` is keyed by track index, so iteration is already in
            // track order and a strict `<` keeps the lowest track on a tie.
            if best.is_none_or(|(_, tick)| head.0 < tick) {
                best = Some((track, head.0));
            }
        }
        let Some((track, _)) = best else {
            break;
        };
        out.push(per_track[&track][heads[track]]);
        heads[track] += 1;
    }
    out
}

/// One chunk of one track: the samples it holds and where it lands in the
/// `mdat` payload.
struct Chunk {
    /// Index of the chunk's first sample within its track.
    first_sample: usize,
    /// Number of consecutive samples in the chunk.
    sample_count: u32,
    /// Byte offset of the chunk's first sample, relative to the `mdat`
    /// payload start.
    rel_offset: u64,
}

/// The `mdat` payload layout: one `Vec<Chunk>` per track, in file order
/// within each track (which is what `stco`/`co64` and `stsc` require).
struct MdatLayout {
    track_chunks: Vec<Vec<Chunk>>,
    /// Total `mdat` payload length.
    payload_len: u64,
}

/// The wall-clock length one interleaved chunk should hold — half a second.
///
/// An interleaved file lets a player that can only read strictly forward start
/// decoding as soon as it has the first chunk of every track; with one chunk
/// per track (what this muxer wrote before, audit r05-W25) that means
/// downloading essentially the whole video track before the first audio sample
/// arrives, so `faststart` bought nothing on a simple HTTP-progressive client.
/// Half a second is the usual trade (chunk headers are 4–8 bytes per chunk).
const INTERLEAVE_CHUNK_MS: u64 = 500;

/// Milliseconds per second, for converting [`INTERLEAVE_CHUNK_MS`] into a
/// movie-timescale tick budget.
const MS_PER_SECOND: u64 = 1_000;

/// Lay the tracks' samples out in the `mdat` payload, interleaved in about
/// [`INTERLEAVE_CHUNK_MS`]-sized runs.
///
/// Each track is cut into chunks of consecutive samples spanning one
/// interleave interval, then the chunks of every track are merged by start
/// time (ties broken by track order, so the result is deterministic). Every
/// sample of a track still appears exactly once, in decode order.
fn plan_mdat_layout(media: &Media, movie_timescale: u32) -> Result<MdatLayout> {
    let mts = i128::from(movie_timescale.max(1));
    let to_movie = |ticks: i64, ts: u32| -> i64 {
        (i128::from(ticks) * mts).div_euclid(i128::from(ts.max(1))) as i64
    };
    let chunk_ticks = i128::from(movie_timescale.max(1)) * i128::from(INTERLEAVE_CHUNK_MS)
        / i128::from(MS_PER_SECOND);
    let chunk_ticks = i64::try_from(chunk_ticks).unwrap_or(i64::MAX).max(1);
    let mut track_chunks: Vec<Vec<Chunk>> = Vec::with_capacity(media.tracks.len());
    // (start tick, track index, first sample, count) for the global merge.
    let mut pending: Vec<(i64, usize, usize, usize)> = Vec::new();
    for (i, track) in media.tracks.iter().enumerate() {
        let times = relative_decode_times(track, &TimelineOrigin::of(&media.tracks));
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut start = 0usize;
        while start < track.samples.len() {
            let chunk_start_tick = to_movie(times[start], track.timescale());
            let mut end = start;
            while end < track.samples.len()
                && to_movie(times[end], track.timescale()) - chunk_start_tick < chunk_ticks
            {
                end += 1;
            }
            let count = end - start;
            pending.push((chunk_start_tick, i, start, count));
            chunks.push(Chunk {
                first_sample: start,
                sample_count: broadcast_common::len::fit_u32(count, "samples_per_chunk")?,
                rel_offset: 0,
            });
            start = end;
        }
        track_chunks.push(chunks);
    }
    // Merge the chunks across tracks by start time — the cross-track
    // interleave criterion. A track's own chunk starts are *not* assumed to be
    // non-decreasing: a source with a non-monotonic decode time (a TS
    // discontinuity, a spliced `Media`) legitimately steps its timestamps
    // backwards, and the old key `(tick, track, first)` then walked a track's
    // chunks out of its own sample order and returned `InvalidInput` on legal
    // input (audit item 5, round 3). Ties break on track then first sample, so
    // the order is deterministic.
    pending = interleave_order(pending);
    let mut payload_len = 0u64;
    // Each track's chunk list is in sample order, so one cursor per track
    // walks it — an O(chunks) merge, not an O(chunks²) search.
    let mut cursor: Vec<usize> = alloc::vec![0; track_chunks.len()];
    for (_, track, first, count) in pending {
        let index = cursor[track];
        let chunk = track_chunks[track]
            .get_mut(index)
            .ok_or(Error::InvalidInput(
                "progressive: chunk plan is inconsistent",
            ))?;
        if chunk.first_sample != first {
            return Err(Error::InvalidInput(
                "progressive: chunk plan is out of order",
            ));
        }
        cursor[track] += 1;
        chunk.rel_offset = payload_len;
        for s in &media.tracks[track].samples[first..first + count] {
            payload_len = payload_len.saturating_add(s.data.len() as u64);
        }
    }
    Ok(MdatLayout {
        track_chunks,
        payload_len,
    })
}

/// Replace a `trak`'s `stbl` children with `new_children`, in place.
fn set_track_stbl(
    moov: &mut MovieBox,
    track_index: usize,
    new_children: Vec<StblChild>,
) -> Result<()> {
    let trak = moov
        .tracks
        .get_mut(track_index)
        .ok_or(Error::UnexpectedBox { expected: "trak" })?;
    let stbl = trak
        .mdia
        .as_mut()
        .and_then(|m| m.minf.as_mut())
        .and_then(|m| m.stbl.as_mut())
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    stbl.children = new_children;
    Ok(())
}

/// Extract the reusable `stsd` child from a `trak` (the per-codec sample entry
/// built by [`build_init_segment`]).
fn take_stsd(moov: &MovieBox, track_index: usize) -> Result<StblChild> {
    let trak = moov
        .tracks
        .get(track_index)
        .ok_or(Error::UnexpectedBox { expected: "trak" })?;
    let stbl = trak
        .mdia
        .as_ref()
        .and_then(|m| m.minf.as_ref())
        .and_then(|m| m.stbl.as_ref())
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    stbl.children
        .iter()
        .find(|c| matches!(c, StblChild::Stsd(_)))
        .cloned()
        .ok_or(Error::UnexpectedBox { expected: "stsd" })
}

impl Package for ProgressiveMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        // MUX = strict but filterable (media plane step-2 fix wave 1,
        // B2-B4): a track `build_init_segment` (below) cannot place into an
        // ISOBMFF `trak` (opaque `CodecConfig::Data` or
        // `CodecConfig::Subtitle`) is no longer silently omitted here — it
        // surfaces the same named error every other mux entry point does.
        // The caller must pre-filter first, e.g. with
        // `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())`.
        let movie_timescale = if media.movie_timescale == 0 {
            DEFAULT_MOVIE_TIMESCALE
        } else {
            media.movie_timescale
        };

        // Reuse the sample-entry + trak skeleton machinery: build the fragmented
        // init moov, parse it back, then overwrite each track's sample tables
        // with the full progressive tables and drop the fragmentation (`mvex`).
        let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let init = build_init_segment(&specs, movie_timescale)?;
        let moov_bytes =
            find_top_box(&init, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
        let mut moov = MovieBox::parse(moov_bytes)?;
        moov.mvex = None; // not a fragmented movie
        let timings = track_timings(media, movie_timescale);
        apply_track_timings(&mut moov, &timings);

        // The mdat payload layout: each track is cut into ~0.5 s chunks and
        // the chunks are interleaved in time order, so a forward-only client
        // reaches the start of every track after a small prefix rather than
        // after the whole video track (audit r05-W25).
        let layout = plan_mdat_layout(media, movie_timescale)?;
        let mut mdat_payload: Vec<u8> = vec![0u8; layout.payload_len as usize];
        for (i, track) in media.tracks.iter().enumerate() {
            for chunk in &layout.track_chunks[i] {
                let first = chunk.first_sample;
                let end = first + chunk.sample_count as usize;
                let mut at = chunk.rel_offset as usize;
                for s in &track.samples[first..end] {
                    let next = at + s.data.len();
                    if next > mdat_payload.len() {
                        return Err(Error::InvalidInput(
                            "progressive: chunk plan overflows the mdat payload",
                        ));
                    }
                    mdat_payload[at..next].copy_from_slice(&s.data);
                    at = next;
                }
            }
        }

        let ftyp = FileTypeBox {
            major_brand: FTYP_MAJOR_BRAND,
            minor_version: FTYP_MINOR_VERSION,
            compatible_brands: vec![*b"isom", *b"iso2", *b"mp41", *b"avc1"],
        };
        let ftyp_len = ftyp.serialized_len();
        let mdat_len = mdat_payload.len() as u64;
        let mdat = MediaDataBox { data: mdat_payload };

        // Compute the absolute file offset of the mdat *payload* under each box
        // ordering, then set the final chunk offsets and build the full tables.
        //
        // Pass 1: build the tables against a placeholder mdat offset so that the
        // moov's serialized size is fixed (offset *values* do not change box
        // sizes — stco entry width is fixed at 4 bytes, co64 at 8, and whether
        // co64 is used depends only on whether the offset exceeds u32::MAX). To
        // keep that decision stable we compute the true offsets up front, which
        // requires the moov size; but the moov size depends on whether co64 is
        // chosen. We break the cycle by assuming 32-bit stco first, measuring,
        // and re-measuring once if any offset turns out to need co64.
        let stsds: Vec<StblChild> = (0..media.tracks.len())
            .map(|i| take_stsd(&moov, i))
            .collect::<Result<_>>()?;

        let mut use_co64 = false;
        let (moov_out, mdat_payload_offset) = loop {
            // Provisional moov size: build tables with the current co64 decision.
            let provisional = assemble_moov(
                &mut moov.clone(),
                &stsds,
                &media.tracks,
                &timings,
                &layout,
                // Provisional payload offset only affects offset values, not the
                // box structure once co64-vs-stco is fixed; pass 0 for sizing.
                0,
                use_co64,
            )?;
            let moov_size = provisional.len();

            // Absolute mdat payload offset under the chosen ordering.
            let mdat_payload_offset =
                mdat_payload_offset(ftyp_len as u64, moov_size as u64, mdat_len, self.faststart);

            // Does any track's absolute chunk offset need 64-bit offsets?
            if needs_co64(mdat_payload_offset, &layout) && !use_co64 {
                use_co64 = true;
                continue;
            }

            let moov_out = assemble_moov(
                &mut moov.clone(),
                &stsds,
                &media.tracks,
                &timings,
                &layout,
                mdat_payload_offset,
                use_co64,
            )?;
            debug_assert_eq!(moov_out.len(), moov_size, "moov size must be offset-stable");
            break (moov_out, mdat_payload_offset);
        };

        // Verify the payload offset we baked into the tables matches the actual
        // layout (guards the two-pass arithmetic).
        let actual_payload_offset = ftyp_len as u64
            + if self.faststart {
                moov_out.len() as u64
            } else {
                0
            }
            + (mdat.serialized_len() - mdat.data.len()) as u64;
        debug_assert_eq!(actual_payload_offset, mdat_payload_offset);

        // Emit ftyp, then moov/mdat in the requested order.
        let mut out = Vec::with_capacity(ftyp_len + moov_out.len() + mdat.serialized_len());
        let mut ftyp_buf = vec![0u8; ftyp_len];
        let n = ftyp.serialize_into(&mut ftyp_buf)?;
        out.extend_from_slice(&ftyp_buf[..n]);

        let mut mdat_buf = vec![0u8; mdat.serialized_len()];
        let m = mdat.serialize_into(&mut mdat_buf)?;
        if self.faststart {
            out.extend_from_slice(&moov_out);
            out.extend_from_slice(&mdat_buf[..m]);
        } else {
            out.extend_from_slice(&mdat_buf[..m]);
            out.extend_from_slice(&moov_out);
        }
        Ok(out)
    }
}

/// Absolute file offset of the `mdat` payload: after `ftyp` (and `moov` under
/// faststart) and the `mdat` header, whose length depends on the payload size
/// (16 bytes once the box needs `largesize`, §4.2 — issue #1019).
fn mdat_payload_offset(
    ftyp_len: u64,
    moov_len: u64,
    mdat_payload_len: u64,
    faststart: bool,
) -> u64 {
    let before = if faststart {
        ftyp_len + moov_len
    } else {
        ftyp_len
    };
    let header = BoxHeader::for_payload(BoxType::from_bytes(MDAT_TYPE), None, mdat_payload_len);
    before + header.header_size() as u64
}

/// Whether any chunk's absolute offset needs the 64-bit `co64` (§8.7.5).
fn needs_co64(mdat_payload_offset: u64, layout: &MdatLayout) -> bool {
    layout
        .track_chunks
        .iter()
        .flatten()
        .any(|c| mdat_payload_offset.saturating_add(c.rel_offset) > u64::from(u32::MAX))
}

/// Fill each track's `stbl` with the full sample tables and serialise the
/// resulting `moov`. `mdat_payload_offset` is the absolute file offset of the
/// mdat payload; per-track chunk offsets are `mdat_payload_offset + rel`.
fn assemble_moov(
    moov: &mut MovieBox,
    stsds: &[StblChild],
    tracks: &[Track],
    timings: &[TrackTiming],
    layout: &MdatLayout,
    mdat_payload_offset: u64,
    use_co64: bool,
) -> Result<Vec<u8>> {
    for (i, track) in tracks.iter().enumerate() {
        let offsets: Vec<u64> = layout.track_chunks[i]
            .iter()
            .map(|c| mdat_payload_offset + c.rel_offset)
            .collect();
        let counts: Vec<u32> = layout.track_chunks[i]
            .iter()
            .map(|c| c.sample_count)
            .collect();
        let children = build_stbl_children(
            stsds[i].clone(),
            &track.samples,
            &timings[i].deltas,
            &offsets,
            &counts,
            use_co64,
        )?;
        set_track_stbl(moov, i, children)?;
    }
    let mut buf = vec![0u8; moov.serialized_len()];
    let n = moov.serialize_into(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// One track's progressive timing: its `stts` decode deltas, its media
/// duration, and the edit list that places it on the movie timeline.
struct TrackTiming {
    deltas: Vec<u32>,
    /// `mdhd.duration`: Σ decode deltas, in the media timescale.
    media_duration: u64,
    /// `tkhd.duration`: the presentation length on the movie timeline
    /// (including a leading empty edit), in the movie timescale.
    presentation_duration: u64,
    edts: Option<EditBox>,
}

/// Build every track's [`TrackTiming`] over one origin common to all tracks
/// (issue #1020): each track's decode times are its samples' own `dts`, and
/// an `elst` (ISO/IEC 14496-12:2015 §8.6.6) delays a track that starts after
/// the earliest one by an empty edit and skips its composition lead-in, so the
/// inter-track offsets of the IR survive into the file.
fn track_timings(media: &Media, movie_timescale: u32) -> Vec<TrackTiming> {
    let origin = TimelineOrigin::of(&media.tracks);
    let mts = i128::from(movie_timescale.max(1));
    let to_movie = |ticks: i64, ts: u32| (ticks as i128 * mts).div_euclid(i128::from(ts.max(1)));

    struct Span {
        times: Vec<i64>,
        /// (first presentation, presentation end), media timescale.
        pres: Option<(i64, i64)>,
    }
    let spans: Vec<Span> = media
        .tracks
        .iter()
        .map(|t| {
            let times = relative_decode_times(t, &origin);
            let pres = t.samples.iter().zip(&times).fold(None, |acc, (s, &d)| {
                let p = d.saturating_add(i64::from(s.composition_offset()));
                let e = p.saturating_add(i64::from(s.duration.unwrap_or(0)));
                Some(match acc {
                    None => (p, e),
                    Some((lo, hi)) => (p.min(lo), e.max(hi)),
                })
            });
            Span { times, pres }
        })
        .collect();
    let movie_start = media
        .tracks
        .iter()
        .zip(&spans)
        .filter_map(|(t, sp)| sp.pres.map(|(p, _)| to_movie(p, t.timescale())))
        .min()
        .unwrap_or(0);

    media
        .tracks
        .iter()
        .zip(spans)
        .map(|(track, sp)| {
            let ts = track.timescale();
            let deltas = decode_deltas(&track.samples, &sp.times);
            let media_duration: u64 = deltas.iter().map(|&d| u64::from(d)).sum();
            let plain_duration = (i128::from(media_duration) * mts / i128::from(ts.max(1))) as u64;
            let (Some((pres_start, pres_end)), Some(&first_dts)) = (sp.pres, sp.times.first())
            else {
                return TrackTiming {
                    deltas,
                    media_duration,
                    presentation_duration: plain_duration,
                    edts: None,
                };
            };
            let empty = (to_movie(pres_start, ts) - movie_start).max(0) as u64;
            let media_time = (pres_start - first_dts).max(0);
            let segment_duration =
                (to_movie(pres_end, ts) - to_movie(pres_start, ts)).max(0) as u64;
            if empty == 0 && media_time == 0 {
                return TrackTiming {
                    deltas,
                    media_duration,
                    presentation_duration: plain_duration,
                    edts: None,
                };
            }
            let mut entries = Vec::with_capacity(2);
            if empty > 0 {
                entries.push(EditListEntry {
                    segment_duration: empty,
                    media_time: ELST_EMPTY_EDIT,
                    media_rate_integer: ELST_NORMAL_RATE,
                    media_rate_fraction: 0,
                });
            }
            entries.push(EditListEntry {
                segment_duration,
                media_time,
                media_rate_integer: ELST_NORMAL_RATE,
                media_rate_fraction: 0,
            });
            // Version 1 carries 64-bit fields; version 0 only 32-bit (§8.6.6).
            let wide = entries.iter().any(|e| {
                u32::try_from(e.segment_duration).is_err() || i32::try_from(e.media_time).is_err()
            });
            TrackTiming {
                deltas,
                media_duration,
                presentation_duration: empty.saturating_add(segment_duration),
                edts: Some(EditBox::new(
                    Some(EditListBox {
                        version: u8::from(wide),
                        flags: 0,
                        entries,
                    }),
                    Vec::new(),
                )),
            }
        })
        .collect()
}

/// Set the `mvhd` and per-track `tkhd`/`mdhd` durations and `edts` from the
/// resolved timings (a non-fragmented movie carries its duration in the
/// header boxes, not in `trun`s).
///
/// A header box's version selects the width of its `duration` field — 32 bits
/// at version 0, 64 at version 1 (ISO/IEC 14496-12:2015 §8.2.2.2, §8.3.2.2,
/// §8.4.2.2). `build_init_segment` writes version-0 headers, so a duration
/// past `u32::MAX` used to be truncated by the v0 serializer's `as u32`: a
/// 10 MHz track (a Smooth-sourced timeline, `smooth_parse`) wrapped after
/// 7.2 minutes and a 90 kHz one after 13.2 hours, so players reported a wrong
/// length and sought wrongly (audit r05-W24). Each box is promoted to version
/// 1 when its own duration does not fit version 0.
fn apply_track_timings(moov: &mut MovieBox, timings: &[TrackTiming]) {
    let mut max_movie_duration = 0u64;
    for (i, timing) in timings.iter().enumerate() {
        max_movie_duration = max_movie_duration.max(timing.presentation_duration);
        if let Some(trak) = moov.tracks.get_mut(i) {
            trak.tkhd.version = if duration_fits_v0(timing.presentation_duration) {
                0
            } else {
                1
            };
            trak.tkhd.duration = timing.presentation_duration;
            trak.edts = timing.edts.clone();
            if let Some(mdhd) = trak.mdia.as_mut().and_then(|m| m.mdhd.as_mut()) {
                mdhd.version = if duration_fits_v0(timing.media_duration) {
                    0
                } else {
                    1
                };
                mdhd.duration = timing.media_duration;
            }
        }
    }
    moov.mvhd.version = if duration_fits_v0(max_movie_duration) {
        0
    } else {
        1
    };
    moov.mvhd.duration = max_movie_duration;
}

/// Whether a header box's `duration` fits the 32-bit version-0 field
/// (ISO/IEC 14496-12:2015 §8.2.2.2).
fn duration_fits_v0(duration: u64) -> bool {
    duration <= u64::from(u32::MAX)
}

/// Find a top-level box by four-CC in an ISOBMFF byte buffer, returning its full
/// bytes (header + body). Walks top-level boxes by their declared `size`.
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let (bx, consumed) = crate::box_types::parse_box(&data[offset..]).ok()?;
        if bx.header.box_type.is(fourcc) {
            let end = if bx.header.size == 0 {
                data.len()
            } else {
                offset + bx.header.size as usize
            };
            return Some(&data[offset..end]);
        }
        if consumed == 0 {
            break;
        }
        offset += consumed;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISO/IEC 14496-12:2015 §4.2: a box whose size does not fit 32 bits is
    /// written `size = 1` + 64-bit `largesize`, a 16-byte header. Driven with a
    /// synthetic payload length, no 4 GiB allocation.
    /// The interleave must keep each track's own chunks in sample order even
    /// when that track's chunk-start ticks run backwards (audit item 5,
    /// round 3). Inputs are the exact `(tick, track, first_sample, count)`
    /// tuples `plan_mdat_layout` produces.
    #[test]
    fn interleave_order_keeps_a_track_in_sample_order() {
        // Track 0's chunk starts step 0, 1000, 2000, then *back* to 500 (a
        // discontinuity), then on to 3000. Track 1 interleaves by tick.
        let pending = alloc::vec![
            (0i64, 0usize, 0usize, 1usize),
            (1000, 0, 1, 1),
            (2000, 0, 2, 1),
            (500, 0, 3, 1),
            (3000, 0, 4, 1),
            (500, 1, 0, 1),
            (1500, 1, 1, 1),
        ];
        let ordered = interleave_order(pending);

        // The merge's invariant: consuming the merged order with one cursor
        // per track must see each track's chunks in sample order.
        let mut cursor = [0usize; 2];
        for &(_, track, first, _) in &ordered {
            assert_eq!(
                first, cursor[track],
                "track {track} chunk arrived out of sample order: {ordered:?}"
            );
            cursor[track] += 1;
        }
        assert_eq!(cursor, [5, 2], "every chunk was consumed");
        // The cross-track property the merge exists for: the first chunk of a
        // second track is not deferred behind every chunk of the first — the
        // tracks interleave by tick where the ticks allow it. (The sequence is
        // *not* globally tick-sorted, and cannot be: track 0's own ticks run
        // backwards, and reordering to make them sorted is exactly the bug.)
        let tracks: alloc::vec::Vec<usize> = ordered.iter().map(|&(_, t, ..)| t).collect();
        assert_eq!(
            tracks,
            alloc::vec![0, 1, 0, 1, 0, 0, 0],
            "tracks interleave by tick: {ordered:?}"
        );
    }

    #[test]
    fn mdat_payload_offset_accounts_for_largesize_header() {
        let fits = u64::from(u32::MAX) - 8; // 8 + payload == u32::MAX
        let over = fits + 1; // 8 + payload == u32::MAX + 1
        assert_eq!(mdat_payload_offset(24, 1000, fits, true), 24 + 1000 + 8);
        assert_eq!(mdat_payload_offset(24, 1000, fits, false), 24 + 8);
        assert_eq!(mdat_payload_offset(24, 1000, over, true), 24 + 1000 + 16);
        assert_eq!(mdat_payload_offset(24, 1000, over, false), 24 + 16);
    }

    #[test]
    fn co64_chosen_once_an_offset_exceeds_u32() {
        let base = 1024;
        let layout = |offsets: &[u64]| MdatLayout {
            track_chunks: alloc::vec![
                offsets
                    .iter()
                    .map(|&rel_offset| Chunk {
                        first_sample: 0,
                        sample_count: 1,
                        rel_offset,
                    })
                    .collect()
            ],
            payload_len: 0,
        };
        assert!(!needs_co64(base, &layout(&[0, u64::from(u32::MAX) - base])));
        assert!(needs_co64(
            base,
            &layout(&[0, u64::from(u32::MAX) - base + 1])
        ));
    }
}
