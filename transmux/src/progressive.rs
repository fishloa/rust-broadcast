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
    chunk_offset: u64,
    use_co64: bool,
) -> Vec<StblChild> {
    let stsz_entries: Vec<u32> = samples.iter().map(|s| s.data.len() as u32).collect();
    let stsz = SampleSizeBox {
        version: 0,
        flags: 0,
        sample_size: 0,
        sample_count: stsz_entries.len() as u32,
        entries: stsz_entries,
    };
    // One chunk holding every sample of this track.
    let stsc = SampleToChunkBox {
        version: 0,
        flags: 0,
        entries: vec![StscEntry {
            first_chunk: 1,
            samples_per_chunk: samples.len() as u32,
            sample_description_index: SAMPLE_DESCRIPTION_INDEX,
        }],
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
            entries: vec![chunk_offset],
        }));
    } else {
        children.push(StblChild::Stco(ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries: vec![chunk_offset as u32],
        }));
    }
    if let Some(stss) = build_stss(samples) {
        children.push(StblChild::Stss(stss));
    }
    children
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

        // The mdat payload layout: each track's samples are concatenated in
        // track order into a single chunk; record each chunk's byte offset
        // *relative to the mdat payload start* for later absolutisation.
        let mut mdat_payload: Vec<u8> = Vec::new();
        let mut rel_chunk_offsets: Vec<u64> = Vec::with_capacity(media.tracks.len());
        for track in &media.tracks {
            rel_chunk_offsets.push(mdat_payload.len() as u64);
            for s in &track.samples {
                mdat_payload.extend_from_slice(&s.data);
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
                &rel_chunk_offsets,
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
            if needs_co64(mdat_payload_offset, &rel_chunk_offsets) && !use_co64 {
                use_co64 = true;
                continue;
            }

            let moov_out = assemble_moov(
                &mut moov.clone(),
                &stsds,
                &media.tracks,
                &timings,
                &rel_chunk_offsets,
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
fn needs_co64(mdat_payload_offset: u64, rel_chunk_offsets: &[u64]) -> bool {
    rel_chunk_offsets
        .iter()
        .any(|&rel| mdat_payload_offset.saturating_add(rel) > u64::from(u32::MAX))
}

/// Fill each track's `stbl` with the full sample tables and serialise the
/// resulting `moov`. `mdat_payload_offset` is the absolute file offset of the
/// mdat payload; per-track chunk offsets are `mdat_payload_offset + rel`.
fn assemble_moov(
    moov: &mut MovieBox,
    stsds: &[StblChild],
    tracks: &[Track],
    timings: &[TrackTiming],
    rel_chunk_offsets: &[u64],
    mdat_payload_offset: u64,
    use_co64: bool,
) -> Result<Vec<u8>> {
    for (i, track) in tracks.iter().enumerate() {
        let abs_offset = mdat_payload_offset + rel_chunk_offsets[i];
        let children = build_stbl_children(
            stsds[i].clone(),
            &track.samples,
            &timings[i].deltas,
            abs_offset,
            use_co64,
        );
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
                edts: Some(EditBox {
                    elst: Some(EditListBox {
                        version: u8::from(wide),
                        flags: 0,
                        entries,
                    }),
                    opaque: Vec::new(),
                }),
            }
        })
        .collect()
}

/// Set the `mvhd` and per-track `tkhd`/`mdhd` durations and `edts` from the
/// resolved timings (a non-fragmented movie carries its duration in the
/// header boxes, not in `trun`s).
fn apply_track_timings(moov: &mut MovieBox, timings: &[TrackTiming]) {
    let mut max_movie_duration = 0u64;
    for (i, timing) in timings.iter().enumerate() {
        max_movie_duration = max_movie_duration.max(timing.presentation_duration);
        if let Some(trak) = moov.tracks.get_mut(i) {
            trak.tkhd.duration = timing.presentation_duration;
            trak.edts = timing.edts.clone();
            if let Some(mdhd) = trak.mdia.as_mut().and_then(|m| m.mdhd.as_mut()) {
                mdhd.duration = timing.media_duration;
            }
        }
    }
    moov.mvhd.duration = max_movie_duration;
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
        assert!(!needs_co64(base, &[0, u64::from(u32::MAX) - base]));
        assert!(needs_co64(base, &[0, u64::from(u32::MAX) - base + 1]));
    }
}
