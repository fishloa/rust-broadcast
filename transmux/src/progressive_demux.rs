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

use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Parse, Unpackage};

use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, MovieBox, SampleSizeBox, SampleToChunkBox, StblChild,
    SyncSampleBox, TrackBox,
};
use crate::media::{Media, Track, find_top_box, refine_legacy_config, track_spec_from_trak};
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
#[derive(Debug, Default, Clone)]
pub struct ProgressiveDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl ProgressiveDemux<'_> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<'a> Unpackage for ProgressiveDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        let moov_bytes =
            find_top_box(input, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
        let moov = MovieBox::parse(moov_bytes)?;
        let movie_timescale = moov.mvhd.timescale;

        // A track whose codec the crate cannot reconstruct, or whose sample
        // tables are incomplete, is skipped rather than failing the whole
        // file — mirrors Fmp4Demux's forgiving per-track handling.
        let mut tracks = Vec::with_capacity(moov.tracks.len());
        for trak in &moov.tracks {
            let Ok(mut spec) = track_spec_from_trak(trak) else {
                continue;
            };
            let Ok(samples) = samples_from_stbl(input, trak) else {
                continue;
            };
            refine_legacy_config(&mut spec.config, &samples);
            tracks.push(Track::new(spec, samples));
        }
        Ok(Media::new(tracks, movie_timescale))
    }
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

    let stts = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stts(b) => Some(b),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stts" })?;
    let ctts = stbl.children.iter().find_map(|c| match c {
        StblChild::Ctts(b) => Some(b),
        _ => None,
    });
    let stss = stbl.children.iter().find_map(|c| match c {
        StblChild::Stss(b) => Some(b),
        _ => None,
    });
    let stsz = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsz(b) => Some(b),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stsz" })?;
    let stsc = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsc(b) => Some(b),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stsc" })?;
    let co64 = stbl.children.iter().find_map(|c| match c {
        StblChild::Co64(b) => Some(b),
        _ => None,
    });
    let stco = stbl.children.iter().find_map(|c| match c {
        StblChild::Stco(b) => Some(b),
        _ => None,
    });

    let chunk_offsets = chunk_offsets(co64, stco)?;
    let samples_per_chunk = expand_stsc(stsc, chunk_offsets.len());
    let total_samples: usize = samples_per_chunk.iter().map(|&n| n as usize).sum();

    let layout = chunk_layout(&chunk_offsets, &samples_per_chunk, stsz, total_samples)?;
    let durations = expand_stts(stts, total_samples)?;
    let composition_offsets = expand_ctts(ctts, total_samples)?;
    let sync_flags = expand_stss(stss, total_samples);

    let mut samples = Vec::with_capacity(total_samples);
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
        samples.push(Sample::new(
            file[start..end].to_vec(),
            durations[i],
            sync_flags[i],
            composition_offsets[i],
        ));
    }
    Ok(samples)
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

/// Expand `stsc`'s compact `(first_chunk, samples_per_chunk)` runs (§8.7.4)
/// into an explicit per-chunk sample count, one entry per chunk in
/// `num_chunks` (from `stco`/`co64`).
fn expand_stsc(stsc: &SampleToChunkBox, num_chunks: usize) -> Vec<u32> {
    let mut table = alloc::vec![0u32; num_chunks];
    for (i, entry) in stsc.entries.iter().enumerate() {
        // first_chunk is 1-based; a run covers [first_chunk, next_run.first_chunk)
        // or through the last chunk for the final run.
        let start = entry.first_chunk as usize;
        let end = stsc
            .entries
            .get(i + 1)
            .map(|next| next.first_chunk as usize)
            .unwrap_or(num_chunks + 1);
        for chunk in start..end {
            if chunk >= 1 && chunk <= num_chunks {
                table[chunk - 1] = entry.samples_per_chunk;
            }
        }
    }
    table
}

/// Walk each chunk in order, resolving every sample's `(absolute_offset,
/// size)` from the chunk's starting file offset plus a running cursor over
/// `stsz` sizes.
fn chunk_layout(
    chunk_offsets: &[u64],
    samples_per_chunk: &[u32],
    stsz: &SampleSizeBox,
    total_samples: usize,
) -> Result<Vec<(usize, usize)>> {
    let mut layout = Vec::with_capacity(total_samples);
    let mut sample_index = 0usize;
    for (chunk, &count) in samples_per_chunk.iter().enumerate() {
        let mut cursor = chunk_offsets[chunk];
        for _ in 0..count {
            let size = sample_size(stsz, sample_index)?;
            let start = usize::try_from(cursor)
                .map_err(|_| Error::InvalidInput("chunk offset exceeds addressable range"))?;
            layout.push((start, size));
            cursor += size as u64;
            sample_index += 1;
        }
    }
    if layout.len() != total_samples {
        return Err(Error::InvalidInput(
            "stsc-derived sample count does not match chunk layout",
        ));
    }
    Ok(layout)
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
