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
//! decode durations are run-length coded into `stts` (§8.6.1.2); composition
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

use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, MovieBox, SampleSizeBox, SampleToChunkBox, StblChild,
    StscEntry, SyncSampleBox,
};
use crate::media::Media;
use crate::pipeline::{Sample, build_init_segment};
use crate::segments::{FileTypeBox, MediaDataBox};
use crate::timing::{CompositionOffsetBox, CttsEntry, SttsEntry, TimeToSampleBox};

/// Default movie timescale used when a [`Media`] does not specify one.
const DEFAULT_MOVIE_TIMESCALE: u32 = 1000;
/// `ftyp` major brand for a progressive (non-fragmented) MP4.
const FTYP_MAJOR_BRAND: [u8; 4] = *b"isom";
/// `ftyp` minor version.
const FTYP_MINOR_VERSION: u32 = 512;
/// Byte size of the plain (32-bit) `mdat` box header (`size` + `type`).
const MDAT_HEADER_LEN: usize = 8;
/// The `default_sample_description_index` referenced by the single `stsd` entry.
const SAMPLE_DESCRIPTION_INDEX: u32 = 1;

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

/// Run-length code a per-sample decode-duration list into `stts` entries
/// (ISO/IEC 14496-12:2015 §8.6.1.2): consecutive equal durations collapse into a
/// single `(sample_count, sample_delta)` run.
fn build_stts(samples: &[Sample]) -> TimeToSampleBox {
    let mut entries: Vec<SttsEntry> = Vec::new();
    for s in samples {
        match entries.last_mut() {
            Some(last) if last.sample_delta == s.duration => last.sample_count += 1,
            _ => entries.push(SttsEntry {
                sample_count: 1,
                sample_delta: s.duration,
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
    if samples.iter().all(|s| s.composition_offset == 0) {
        return None;
    }
    let mut entries: Vec<CttsEntry> = Vec::new();
    for s in samples {
        match entries.last_mut() {
            Some(last) if last.sample_offset == s.composition_offset => last.sample_count += 1,
            _ => entries.push(CttsEntry {
                sample_count: 1,
                sample_offset: s.composition_offset,
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
    if samples.iter().all(|s| s.is_sync) {
        return None;
    }
    let entries: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter_map(|(i, s)| if s.is_sync { Some(i as u32 + 1) } else { None })
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
    chunk_offset: u64,
    use_co64: bool,
) -> Vec<StblChild> {
    let stsz = SampleSizeBox {
        version: 0,
        flags: 0,
        sample_size: 0,
        entries: samples.iter().map(|s| s.data.len() as u32).collect(),
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
        StblChild::Stts(build_stts(samples)),
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
        // Opaque `CodecConfig::Data` tracks have no ISOBMFF sample entry in
        // this crate (issue #557/#576) — omit them from the progressive-MP4
        // mux entirely rather than erroring. Filtering the whole `Media` up
        // front (rather than per-field below) keeps every subsequent
        // `media.tracks`-indexed computation in this function consistent.
        let filtered;
        let media: &Media = if media.tracks.iter().any(|t| t.spec.config.is_opaque_data()) {
            filtered = media.select_tracks_by(|t| !t.spec.config.is_opaque_data())?;
            &filtered
        } else {
            media
        };
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
        set_track_durations(&mut moov, media, movie_timescale);

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
            let provisional = self.assemble_moov(
                &mut moov.clone(),
                &stsds,
                media,
                &rel_chunk_offsets,
                // Provisional payload offset only affects offset values, not the
                // box structure once co64-vs-stco is fixed; pass 0 for sizing.
                0,
                use_co64,
            )?;
            let moov_size = provisional.len();

            // Absolute mdat payload offset under the chosen ordering.
            let mdat_payload_offset = if self.faststart {
                (ftyp_len + moov_size + MDAT_HEADER_LEN) as u64
            } else {
                (ftyp_len + MDAT_HEADER_LEN) as u64
            };

            // Does any track's absolute chunk offset need 64-bit offsets?
            let needs_co64 = rel_chunk_offsets
                .iter()
                .any(|&rel| mdat_payload_offset + rel > u32::MAX as u64);
            if needs_co64 && !use_co64 {
                use_co64 = true;
                continue;
            }

            let moov_out = self.assemble_moov(
                &mut moov.clone(),
                &stsds,
                media,
                &rel_chunk_offsets,
                mdat_payload_offset,
                use_co64,
            )?;
            debug_assert_eq!(moov_out.len(), moov_size, "moov size must be offset-stable");
            break (moov_out, mdat_payload_offset);
        };

        // Verify the payload offset we baked into the tables matches the actual
        // layout (guards the two-pass arithmetic).
        let actual_payload_offset = if self.faststart {
            (ftyp_len + moov_out.len() + MDAT_HEADER_LEN) as u64
        } else {
            (ftyp_len + MDAT_HEADER_LEN) as u64
        };
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

impl ProgressiveMux {
    /// Fill each track's `stbl` with the full sample tables and serialise the
    /// resulting `moov`. `mdat_payload_offset` is the absolute file offset of the
    /// mdat payload; per-track chunk offsets are `mdat_payload_offset + rel`.
    fn assemble_moov(
        &self,
        moov: &mut MovieBox,
        stsds: &[StblChild],
        media: &Media,
        rel_chunk_offsets: &[u64],
        mdat_payload_offset: u64,
        use_co64: bool,
    ) -> Result<Vec<u8>> {
        for (i, track) in media.tracks.iter().enumerate() {
            let abs_offset = mdat_payload_offset + rel_chunk_offsets[i];
            let children =
                build_stbl_children(stsds[i].clone(), &track.samples, abs_offset, use_co64);
            set_track_stbl(moov, i, children)?;
        }
        let mut buf = vec![0u8; moov.serialized_len()];
        let n = moov.serialize_into(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }
}

/// Set the `mvhd` and per-track `tkhd`/`mdhd` durations from the sample stream
/// so the file reports a real duration (a non-fragmented movie carries its
/// duration in the header boxes, not in `trun`s).
fn set_track_durations(moov: &mut MovieBox, media: &Media, movie_timescale: u32) {
    let mut max_movie_duration = 0u64;
    for (i, track) in media.tracks.iter().enumerate() {
        let media_duration: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
        let ts = if track.timescale() == 0 {
            1
        } else {
            track.timescale()
        } as u64;
        // Duration in movie-timescale units (for mvhd/tkhd).
        let movie_duration = media_duration * movie_timescale as u64 / ts;
        if movie_duration > max_movie_duration {
            max_movie_duration = movie_duration;
        }
        if let Some(trak) = moov.tracks.get_mut(i) {
            trak.tkhd.duration = movie_duration;
            if let Some(mdhd) = trak.mdia.as_mut().and_then(|m| m.mdhd.as_mut()) {
                mdhd.duration = media_duration;
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
