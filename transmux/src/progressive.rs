//! Progressive (single-file, non-fragmented) MP4 packager — ISO/IEC 14496-12:2026 §8.
//!
//! [`ProgressiveMux`] muxes the crate's [`Media`] IR into one **non-fragmented**
//! `.mp4` file: an `ftyp`, a single `moov` carrying per-track `trak` boxes with
//! the *full* sample tables (`stbl`: `stsd`/`stts`/`ctts`/`stsc`/`stsz`/
//! `stco`|`co64`/`stss`, ISO/IEC 14496-12:2026 §8.5–§8.7), and a single `mdat`
//! holding every track's sample data concatenated. This is the VOD/download
//! counterpart to the fragmented [`crate::media::CmafMux`].
//!
//! This module's job is only the IR-to-plan mapping: it reads each sample's
//! decode delta, composition offset, sync flag and size, and each track's
//! `elst` and durations (`track_timings`), then hands the
//! [`PlanBuilder`] a complete plan. The layout — chunk boundaries, the
//! cross-track interleave, `stco`/`co64`, the `mdat` header form and every
//! sample's absolute offset — is computed by
//! [`isobmff::writer::Writer::progressive`] from that plan alone, before a
//! sample byte is pushed. Sample bytes go straight from the IR to their planned
//! file offset (a seekable sink), so there is no intermediate payload buffer.
//!
//! With [`ProgressiveMux::faststart`] set, `moov` is written **before** `mdat`
//! for progressive-download friendliness; otherwise `mdat` precedes `moov`.
//! The sample entries and `trak` skeleton are built by the same
//! [`crate::pipeline::build_init_segment`] machinery the fragmented muxers use
//! (the record is rebuilt from the [`TrackSpec`](crate::pipeline::TrackSpec),
//! never copied from an input file).

use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::len::fit_u32;
use broadcast_common::timescale::{Rounding, Timescale, rescale, rescale_u64};

use crate::error::{Error, Result};
use crate::media::{Media, TimelineOrigin, relative_decode_times};
use crate::pipeline::Sample;
use isobmff::plan::{PlanBuilder, PlanSample};
use isobmff::timing::EditListEntry;
use isobmff::writer::{FileType, Interleave, Layout, MovieHeader, Sink, Write, Writer};
use isobmff::{Limits, TrackId};

/// `ftyp` major brand for a progressive (non-fragmented) MP4.
const FTYP_MAJOR_BRAND: [u8; 4] = *b"isom";
/// `ftyp` minor version.
const FTYP_MINOR_VERSION: u32 = 512;
/// `elst` `media_time` of an empty edit (ISO/IEC 14496-12:2026 §8.6.6).
const ELST_EMPTY_EDIT: i64 = -1;
/// `elst` `media_rate_integer` for normal-speed playback (§8.6.6).
const ELST_NORMAL_RATE: i16 = 1;
/// Longest edit list this muxer emits: an optional empty edit plus the media edit.
const EDIT_LIST_MAX_ENTRIES: usize = 2;

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
            let own = s.timing.and_then(|t| t.duration()).unwrap_or(0);
            match (times.get(i), times.get(i + 1)) {
                (Some(&now), Some(&next)) if next >= now => {
                    u32::try_from(next - now).unwrap_or(own)
                }
                _ => own,
            }
        })
        .collect()
}

/// The wall-clock length one interleaved chunk should hold — half a second.
///
/// An interleaved file lets a player that can only read strictly forward start
/// decoding as soon as it has the first chunk of every track; with one chunk
/// per track (what this muxer wrote before, audit r05-W25) that means
/// downloading essentially the whole video track before the first audio sample
/// arrives, so `faststart` bought nothing on a simple HTTP-progressive client.
/// Half a second is the usual trade (chunk headers are 4–8 bytes per chunk).
const INTERLEAVE_CHUNK_MS: u32 = 500;

/// One track's progressive timing: its `stts` decode deltas, its media
/// duration, and the edit list that places it on the movie timeline.
struct TrackTiming {
    deltas: Vec<u32>,
    /// `mdhd.timing.map(|t| t.duration)`: Σ decode deltas, in the media timescale.
    media_duration: u64,
    /// `tkhd.timing.map(|t| t.duration)`: the presentation length on the movie timeline
    /// (including a leading empty edit), in the movie timescale.
    presentation_duration: u64,
    edits: Vec<EditListEntry>,
}

impl TrackTiming {
    /// The `elst` entries the plan writes.
    fn edit_list(&self) -> Vec<EditListEntry> {
        self.edits.clone()
    }
}

/// Build every track's [`TrackTiming`] over one origin common to all tracks
/// (issue #1020): each track's decode times are its samples' own `dts`, and
/// an `elst` (ISO/IEC 14496-12:2026 §8.6.6) delays a track that starts after
/// the earliest one by an empty edit and skips its composition lead-in, so the
/// inter-track offsets of the IR survive into the file.
fn track_timings(media: &Media, movie_timescale: Timescale) -> Result<Vec<TrackTiming>> {
    let origin = TimelineOrigin::of(&media.tracks);
    let to_movie = |ticks: i64, ts: Timescale| {
        rescale(i128::from(ticks), ts, movie_timescale, Rounding::Floor)
    };

    struct Span {
        times: Vec<i64>,
        /// (first presentation, presentation end), media timescale.
        pres: Option<(i64, i64)>,
    }
    let spans: Vec<Span> = media
        .tracks
        .iter()
        .map(|t| -> Result<Span> {
            let times = relative_decode_times(t, &origin)?;
            let pres = t.samples.iter().zip(&times).fold(None, |acc, (s, &d)| {
                let p = d.saturating_add(i64::from(
                    s.composition_offset().ok().flatten().unwrap_or(0),
                ));
                let e =
                    p.saturating_add(i64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)));
                Some(match acc {
                    None => (p, e),
                    Some((lo, hi)) => (p.min(lo), e.max(hi)),
                })
            });
            Ok(Span { times, pres })
        })
        .collect::<Result<Vec<_>>>()?;
    let movie_start = media
        .tracks
        .iter()
        .zip(&spans)
        .filter_map(|(t, sp)| sp.pres.map(|(p, _)| to_movie(p, t.timescale())))
        .min()
        .unwrap_or(0);

    let result = media
        .tracks
        .iter()
        .zip(spans)
        .map(|(track, sp)| {
            let ts = track.timescale();
            let deltas = decode_deltas(&track.samples, &sp.times);
            let media_duration: u64 = deltas.iter().map(|&d| u64::from(d)).sum();
            let plain_duration =
                rescale_u64(media_duration, ts, movie_timescale, Rounding::TowardZero);
            let (Some((pres_start, pres_end)), Some(&first_dts)) = (sp.pres, sp.times.first())
            else {
                return TrackTiming {
                    deltas,
                    media_duration,
                    presentation_duration: plain_duration,
                    edits: Vec::new(),
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
                    edits: Vec::new(),
                };
            }
            let mut entries = Vec::with_capacity(EDIT_LIST_MAX_ENTRIES);
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
            TrackTiming {
                deltas,
                media_duration,
                presentation_duration: empty.saturating_add(segment_duration),
                edits: entries,
            }
        })
        .collect::<Vec<_>>();
    Ok(result)
}

/// Place one writer output at its absolute offset in the output buffer.
fn place(out: &mut Vec<u8>, write: Write<'_>) -> Result<()> {
    let start = usize::try_from(write.offset)
        .map_err(|_| Error::InvalidInput("progressive: offset exceeds the address space"))?;
    let end = start
        .checked_add(write.bytes.len())
        .ok_or(Error::InvalidInput(
            "progressive: write overflows the address space",
        ))?;
    if out.len() < end {
        out.resize(end, 0);
    }
    out.get_mut(start..end)
        .ok_or(Error::InvalidInput("progressive: write outside the file"))?
        .copy_from_slice(&write.bytes);
    Ok(())
}

impl broadcast_common::Package for ProgressiveMux {
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
        let movie_timescale = media.movie_timescale;

        let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let setups = crate::pipeline::track_setups(&specs)?;
        let timings = track_timings(media, movie_timescale)?;
        let origin = TimelineOrigin::of(&media.tracks);

        let mut builder = PlanBuilder::new(setups, Limits::default());
        for (index, track) in media.tracks.iter().enumerate() {
            let id = TrackId::new(track.spec.track_id.value());
            let timing = timings
                .get(index)
                .ok_or(Error::InvalidInput("progressive: track timing is missing"))?;
            let times = relative_decode_times(track, &origin)?;
            let start = times.first().copied().unwrap_or(0);
            let start = u64::try_from(start).map_err(|_| {
                Error::InvalidInput("progressive: a track starts before the timeline origin")
            })?;
            builder.set_start_decode_time(id, start)?;
            builder.set_durations(id, timing.media_duration, timing.presentation_duration)?;
            builder.set_edits(id, timing.edit_list())?;
            for (sample_index, sample) in track.samples.iter().enumerate() {
                let duration = timing.deltas.get(sample_index).copied().unwrap_or(0);
                builder.push(
                    id,
                    PlanSample {
                        duration,
                        composition_offset: sample.composition_offset()?.unwrap_or(0),
                        sync: sample.flags.is_sync,
                        size: fit_u32(sample.data.len(), "stsz entry_size")?,
                        description_index: isobmff::plan::DEFAULT_DESCRIPTION_INDEX,
                    },
                )?;
            }
        }
        let plan = builder.finish()?;

        let file_type = FileType {
            major_brand: FTYP_MAJOR_BRAND,
            minor_version: FTYP_MINOR_VERSION,
            compatible_brands: vec![*b"isom", *b"iso2", *b"mp41", *b"avc1"],
            emit_ftyp: true,
        };
        let layout = Layout::new(
            self.faststart,
            Interleave::ByDuration {
                millis: INTERLEAVE_CHUNK_MS,
            },
            Sink::Seekable,
        );
        let mut writer = Writer::progressive(
            plan,
            layout,
            file_type,
            MovieHeader::new(movie_timescale),
            Limits::default(),
        )?;

        let file_len = usize::try_from(writer.file_len())
            .map_err(|_| Error::InvalidInput("progressive: the file exceeds the address space"))?;
        let mut out = vec![0u8; file_len];
        for write in writer.header() {
            place(&mut out, write)?;
        }
        for track in &media.tracks {
            let id = TrackId::new(track.spec.track_id.value());
            for (sample_index, sample) in track.samples.iter().enumerate() {
                let index = u32::try_from(sample_index)
                    .map_err(|_| Error::InvalidInput("progressive: too many samples"))?;
                for write in writer.push(id, index, &sample.data)? {
                    place(&mut out, write)?;
                }
            }
        }
        for write in writer.finish()? {
            place(&mut out, write)?;
        }
        Ok(out)
    }
}
