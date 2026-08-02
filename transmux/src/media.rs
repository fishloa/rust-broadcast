//! The any-to-any hub impls over the [`crate::ir`] media IR.
//!
//! This module is the transmux side of the `broadcast-common` container-mux
//! vocabulary ([`broadcast_common::Unpackage`] / [`broadcast_common::Package`]).
//! The [`Media`] IR itself ([`Media`]/[`Track`]/[`TrackEncryption`]/[`PcrSample`],
//! re-exported here from [`crate::ir`]) is elementary [`Track`]s of coded
//! [`Sample`]s; this module holds the packagers/depackagers that convert
//! between that IR and concrete container forms:
//!
//! - [`Fmp4Demux`] : [`Unpackage`] `<Input = &[u8]>` — parse a fragmented
//!   ISOBMFF/CMAF file (init `moov` + one or more `moof`/`mdat` fragments) into
//!   a [`Media`], reusing the crate's existing box parsers.
//! - [`CmafMux`] : [`Package`] `<Output = Vec<u8>>` — mux a [`Media`] back into
//!   a CMAF init segment + one media segment, a transparent wrapper over
//!   [`build_init_segment`] / [`build_media_segment`].
//! - [`HlsPackager`] : [`Package`] `<Output = String>` — render an RFC 8216
//!   media playlist describing a [`Media`].
//!
//! The [`Track`] IR is a thin wrapper over the existing public
//! [`TrackSpec`] (codec config + timescale + track_id) and [`Sample`] (coded
//! access units); those types stay public and unchanged. `no_std` + `alloc`.
//!
//! Sample [`Decrypt`](broadcast_common::Decrypt) is implemented for CENC/CBCS
//! (ISO/IEC 23001-7 AES-CTR / AES-CBC-pattern) by
//! [`CencDecryptor`](crate::cenc_decrypt::CencDecryptor), and the inverse
//! [`Encrypt`](broadcast_common::Encrypt) by
//! [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor) — both behind the
//! `cenc` feature (issues #465/#564). Either direction records its per-track
//! crypto metadata onto [`Track::encryption`].

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Package, Parse, Unpackage};

use crate::ac3::{Ac3SpecificBox, Ec3SpecificBox};
use broadcast_hls::{ByteRange, MapTag, MediaPlaylist, MediaSegment};

use crate::ac4::Ac4SpecificBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, parse_box};
use crate::dts::DtsSpecificBox;
use crate::error::{Error, Result};
use crate::flac::FlacSpecificBox;
use crate::init_segment::{MovieBox, OpaqueBox, SampleEntryVariant, StblChild, TrackBox};
use crate::ir::{CodecConfig, FragmentTrackData, Sample, SubtitleFormat, TrackSpec};
use crate::movie_fragment::MovieFragmentBox;
use crate::mp4esds::EsdsBox;
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer};
use crate::mpegh::{MHAC_FOURCC, MHADecoderConfigurationRecord};
use crate::opus::OpusSpecificBox;
use crate::pipeline::{build_init_segment, build_media_segment};

/// [`Media`], [`Track`], [`TrackEncryption`], [`PcrSample`] moved to
/// [`crate::ir`] (media plane step 2a) — re-exported here so every existing
/// `crate::media::`/`transmux::media::` path keeps resolving unchanged.
pub use crate::ir::{Media, PcrSample, SkippedTrack, Track, TrackEncryption};

/// `sample_is_non_sync_sample` bit within a 32-bit `sample_flags` word
/// (ISO/IEC 14496-12:2015 §8.8.3.1, bit `[16]`). Set = the sample is **not** a
/// sync sample (random-access point).
const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

/// Default movie timescale used when a source does not specify one.
const DEFAULT_MOVIE_TIMESCALE: u32 = 1000;

/// `esds` `objectTypeIndication` for MPEG-2 Audio (ISO/IEC 13818-3) —
/// ISO/IEC 14496-1 §7.2.6.6 Table 5.
const OTI_MPEG2_AUDIO: u8 = 0x69;
/// `esds` `objectTypeIndication` for MPEG-1 Audio (ISO/IEC 11172-3) — Table 5.
const OTI_MPEG1_AUDIO: u8 = 0x6B;

// ---------------------------------------------------------------------------
// Fmp4Demux — Unpackage<Input = &[u8]>
// ---------------------------------------------------------------------------

/// Demux a fragmented ISOBMFF/CMAF byte stream into a [`Media`].
///
/// Walks top-level boxes: the `moov` supplies each track's identity + codec
/// config (rebuilt into a [`TrackSpec`]); every `moof`/`mdat` fragment pair
/// supplies coded samples (sizes/durations/flags resolved from the `trun`,
/// falling back to the `tfhd` defaults), which are appended to the matching
/// track in decode order.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes
/// via [`Unpackage::Input`]; construct one per call with [`Fmp4Demux::new`].
#[derive(Debug, Default, Clone)]
pub struct Fmp4Demux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl Fmp4Demux<'_> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// A track skeleton built from the init `moov`, plus a running sample list.
struct TrackBuilder {
    spec: TrackSpec,
    samples: Vec<Sample>,
    /// Absolute decode-time anchor: the track's start time, taken from the
    /// **first** movie fragment's `tfdt` seen for this track (`None` until that
    /// fragment is absorbed; a stream with no `tfdt` at all yields a `0`
    /// anchor). Distinct from `next_dts`, which every later `tfdt` re-seeds.
    start_decode_time: Option<u64>,
    /// Running absolute decode time for the *next* sample to be pushed, in
    /// this track's media timescale (media plane step 2c: `Sample::dts` is
    /// now absolute, so this replaces the old "reconstruct via anchor + Σ
    /// duration downstream" model with the equivalent running value carried
    /// forward while demuxing). **Re-seeded from every fragment's own `tfdt`**
    /// (falling back to the running sum for a fragment that carries none, and
    /// to `0` if the stream never carries one), then advanced by each sample's
    /// duration in decode order — see `absorb_fragment`.
    next_dts: i64,
}

impl<'a> Unpackage for Fmp4Demux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        // 1. Locate and parse the init `moov` for track specs.
        let moov_bytes =
            find_top_box(input, b"moov").ok_or(Error::UnexpectedBox { expected: "moov" })?;
        let moov = MovieBox::parse(moov_bytes)?;
        let movie_timescale = moov.mvhd.timescale;

        // DEMUX = lenient but loud (media plane step-2 fix wave 1, B2/B3): a
        // track whose sample entry the crate cannot reconstruct into a
        // `CodecConfig` (a QuickTime hint/chapter track, `c608`/`c708`,
        // GoPro `gpmd`, or any other FourCC with no `CodecConfig`
        // reconstruction — codec coverage (`stpp`/`wvtt`/`ac-4`) already
        // landed, so what remains unreconstructable is a genuine gap) is
        // skipped, not a whole-file failure — mirrors
        // [`crate::progressive_demux::ProgressiveDemux`]'s per-track
        // handling, which this used to diverge from. The caller still
        // learns about it via [`Media::skipped`], never silently.
        let mut builders: Vec<TrackBuilder> = Vec::with_capacity(moov.tracks.len());
        let mut skipped: Vec<SkippedTrack> = Vec::new();
        for trak in &moov.tracks {
            match track_spec_from_trak(trak) {
                Ok(spec) => builders.push(TrackBuilder {
                    spec,
                    samples: Vec::new(),
                    start_decode_time: None,
                    next_dts: 0,
                }),
                Err(err) => skipped.push(skipped_track(err)),
            }
        }

        // 2. Walk every top-level box; each `moof` pairs with the next `mdat`.
        //    Sample bytes are resolved via the trun `data_offset` measured from
        //    the enclosing `moof` (default-base-is-moof), so we track the moof's
        //    absolute file offset.
        let mut offset = 0usize;
        let mut pending_moof: Option<(usize, MovieFragmentBox)> = None;
        while offset + BOX_HEADER_MIN_SIZE <= input.len() {
            let (bx, consumed) = parse_box(&input[offset..])?;
            let ty = bx.header.box_type.0;
            if &ty == b"moof" {
                let moof = MovieFragmentBox::parse_body(bx.body)?;
                pending_moof = Some((offset, moof));
            } else if &ty == b"mdat" {
                if let Some((moof_off, moof)) = pending_moof.take() {
                    absorb_fragment(input, moof_off, &moof, &mut builders)?;
                }
            }
            if consumed == 0 {
                break;
            }
            offset += consumed;
        }

        let tracks = builders
            .into_iter()
            .map(|mut b| {
                refine_legacy_config(&mut b.spec.config, &b.samples);
                Track {
                    spec: b.spec,
                    samples: b.samples,
                    // First-fragment tfdt baseMediaDecodeTime; 0 when the stream
                    // carried no tfdt at all (ISO/IEC 14496-12:2015 §8.8.12).
                    start_decode_time: b.start_decode_time.unwrap_or(0),
                    encryption: None,
                }
            })
            .collect();
        Ok(Media {
            tracks,
            movie_timescale,
            pcr: Vec::new(),
            skipped,
        })
    }
}

/// Resolve a fragment's samples into `builders`, slicing coded bytes from the
/// file using each `trun.data_offset` (relative to the `moof` start).
fn absorb_fragment(
    file: &[u8],
    moof_off: usize,
    moof: &MovieFragmentBox,
    builders: &mut [TrackBuilder],
) -> Result<()> {
    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        let Some(builder) = builders
            .iter_mut()
            .find(|b| b.spec.track_id == tfhd.track_id)
        else {
            // A track present in a fragment but absent from the init movie:
            // skip it (well-formed CMAF declares every track in the init moov).
            continue;
        };
        // `tfdt` is authoritative for **every** fragment that carries one, not
        // just the first (ISO/IEC 14496-12:2015 §8.8.12: it *is* the absolute
        // decode time of the fragment's first sample). Seeding once and then
        // pure-summing `trun` durations silently assumes the source is gapless;
        // for a discontinuous or gapped fMP4 (a DVR recording spliced across a
        // dropout, a live capture that missed segments) every sample after the
        // gap came out short by the gap, which then re-muxes to a wrong `tfdt`
        // / PES stamp and permanently desyncs A/V.
        //
        // Re-seeding per fragment carries the gap through into the samples'
        // absolute `dts` — which, for a batch `Media`, is what representing the
        // discontinuity means: this IR has no event channel to raise it on, and
        // the honest alternative to a preserved gap is a fabricated
        // contiguity, not a report.
        //
        // `start_decode_time` still records the FIRST fragment's anchor: it is
        // the track's start time, not a running cursor.
        if let Some(tfdt) = &traf.tfdt {
            let base = tfdt.base_media_decode_time();
            if builder.start_decode_time.is_none() {
                builder.start_decode_time = Some(base);
            }
            builder.next_dts = base as i64;
        }
        for trun in &traf.trun {
            // data_offset is measured from the start of the moof box when
            // default-base-is-moof is set (the base transmux emits and the
            // near-universal fragmented-MP4 convention).
            let base = moof_off as i64 + trun.data_offset.unwrap_or(0) as i64;
            let mut cursor = base;
            for (i, ts) in trun.samples.iter().enumerate() {
                let size = ts
                    .sample_size
                    .or(tfhd.default_sample_size)
                    .ok_or(Error::InvalidInput(
                    "trun sample has no size (no trun.sample_size, no tfhd default_sample_size)",
                ))? as usize;
                let duration = ts
                    .sample_duration
                    .or(tfhd.default_sample_duration)
                    .unwrap_or(0);
                // Per-sample flags precedence: explicit trun sample_flags, else
                // first_sample_flags for sample 0, else the tfhd default.
                let flags = ts
                    .sample_flags
                    .or(if i == 0 {
                        trun.first_sample_flags
                    } else {
                        None
                    })
                    .or(tfhd.default_sample_flags)
                    .unwrap_or(0);
                let is_sync = flags & SAMPLE_FLAG_IS_NON_SYNC == 0;
                let composition_offset = ts.sample_composition_time_offset.unwrap_or(0) as i64;

                let start = usize::try_from(cursor)
                    .map_err(|_| Error::InvalidInput("negative sample data offset"))?;
                let end = start + size;
                if end > file.len() {
                    return Err(Error::BufferTooShort {
                        need: end,
                        have: file.len(),
                        what: "fragment sample data",
                    });
                }
                // Absolute dts/pts (media plane step 2c): `next_dts` is the
                // running cursor seeded from this track's first `tfdt` (or 0
                // if the stream never carried one); pts folds in the trun
                // composition offset directly rather than storing it
                // separately.
                let dts = builder.next_dts;
                let pts = dts + composition_offset;
                builder.samples.push(Sample {
                    data: file[start..end].to_vec().into(),
                    dts: Some(dts),
                    pts: Some(pts),
                    duration: Some(duration),
                    flags: crate::ir::SampleFlags::new(is_sync),
                    // fMP4 sources carry no per-sample source-container
                    // timestamps distinct from the fragment's own tfdt/trun
                    // timing (see `Sample::provenance`).
                    provenance: None,
                });
                builder.next_dts += duration as i64;
                cursor += size as i64;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CmafMux — Package<Output = Vec<u8>>
// ---------------------------------------------------------------------------

/// Mux a [`Media`] into a CMAF init segment + one media segment (concatenated).
///
/// A transparent wrapper: the bytes are exactly
/// `build_init_segment(specs, movie_timescale)` followed by
/// `build_media_segment(sequence_number, fragments)` for the equivalent input,
/// so it composes with the rest of the crate's box layer.
#[derive(Debug, Clone)]
pub struct CmafMux {
    /// `moof.mfhd` sequence number for the emitted media segment (1-based).
    pub sequence_number: u32,
}

impl Default for CmafMux {
    fn default() -> Self {
        Self { sequence_number: 1 }
    }
}

impl CmafMux {
    /// Create a muxer with the given media-segment sequence number.
    pub fn new(sequence_number: u32) -> Self {
        Self { sequence_number }
    }
}

impl Package for CmafMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        // MUX = strict but filterable (media plane step-2 fix wave 1,
        // B1-B4): `build_init_segment` below rejects (naming it) the first
        // track it cannot place into an ISOBMFF `trak` — an opaque
        // `CodecConfig::Data` track (issue #557/#576) or a
        // `CodecConfig::Subtitle` track (B1) — rather than silently
        // dropping it, so a caller mixing carriable and non-carriable
        // streams must explicitly opt in to dropping the latter, e.g. with
        // `media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())`
        // before calling `package`. This is the single shared check every
        // fMP4/CMAF mux entry point uses, not a `CmafMux`-only one.
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let movie_timescale = if media.movie_timescale == 0 {
            DEFAULT_MOVIE_TIMESCALE
        } else {
            media.movie_timescale
        };
        let mut out = build_init_segment(&specs, movie_timescale)?;

        // Each track's single-segment fragment is anchored at the track's
        // absolute decode time (its `tfdt` baseMediaDecodeTime, ISO/IEC
        // 14496-12:2015 §8.8.12) — the anchor a demuxer populated and the
        // `crate::rebase` transforms condition.
        let fragments: Vec<FragmentTrackData<'_>> = media
            .tracks
            .iter()
            .map(|t| FragmentTrackData {
                track_id: t.spec.track_id,
                base_media_decode_time: t.start_decode_time,
                samples: &t.samples,
            })
            .collect();
        let media_seg = build_media_segment(self.sequence_number, &fragments)?;
        out.extend_from_slice(&media_seg);
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// HlsPackager — Package<Output = String>
// ---------------------------------------------------------------------------

/// Render an RFC 8216 media playlist describing a [`Media`].
///
/// Each track's total duration (sum of sample durations ÷ timescale) becomes a
/// single `#EXTINF` segment entry pointing at `{uri_prefix}{track_id}.m4s`.
#[derive(Debug, Clone)]
pub struct HlsPackager {
    /// `#EXT-X-VERSION`.
    pub version: u8,
    /// `#EXT-X-MEDIA-SEQUENCE`.
    pub media_sequence: u64,
    /// URI prefix for the generated per-track segment entries.
    pub uri_prefix: String,
}

impl Default for HlsPackager {
    fn default() -> Self {
        Self {
            version: 7,
            media_sequence: 0,
            uri_prefix: String::from("seg"),
        }
    }
}

impl Package for HlsPackager {
    type Media = Media;
    type Output = String;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<String> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        // A section-carried track (SCTE-35 `stream_type` 0x86, DSM-CC, private
        // sections) has no timestamps at all (`Sample::duration` is `None` for
        // every sample, deliberately) — see the per-track skip below for why
        // it is omitted from the playlist. Such a track is also frequently
        // not BMFF-muxable at all (an opaque `CodecConfig::Data` track), so it
        // must be excluded here too: only tracks that will actually become a
        // `#EXTINF` segment below feed the shared init segment's track list.
        let kept: Vec<&Track> = media
            .tracks
            .iter()
            .filter(|t| t.samples.iter().any(|s| s.duration.is_some()))
            .collect();

        // Every segment this packager names (`{prefix}{track_id}.m4s`) is
        // produced downstream (`cli::package`'s `OutputFormat::Hls` arm) as a
        // *self-initializing* CMAF artifact: `build_init_segment` immediately
        // followed by `build_media_segment` in the same file — i.e. the exact
        // byte layout `CmafMux` builds (media.rs doc comment above), fed the
        // same kept-track set (`filter_for_bmff_mux` already stripped any
        // non-muxable track before either packager runs, in the real
        // pipeline). Apple's HLS Authoring Specification requires an
        // `#EXT-X-MAP` on every fMP4 Media Segment even when it is
        // self-initializing (mediastreamvalidator 1.25.36: "Each fMP4 Segment
        // in a Media Playlist MUST have an EXT-X-MAP tag applied to it" —
        // issue #870 caught this gap: no `#EXT-X-MAP` was emitted at all). The
        // Media Initialization Section is the leading `ftyp`+`moov` — exactly
        // `build_init_segment`'s output length — so every segment's
        // `#EXT-X-MAP` points at itself with a `BYTERANGE` covering just that
        // leading span; `build_init_segment` is a pure function of (specs,
        // movie_timescale), the same inputs `CmafMux` uses, so this computes
        // the true offset without needing the actual segment bytes in hand.
        let specs: Vec<TrackSpec> = kept.iter().map(|t| t.spec.clone()).collect();
        let movie_timescale = if media.movie_timescale == 0 {
            DEFAULT_MOVIE_TIMESCALE
        } else {
            media.movie_timescale
        };
        let init_len = if specs.is_empty() {
            0
        } else {
            build_init_segment(&specs, movie_timescale)?.len() as u64
        };

        let mut segments = Vec::with_capacity(kept.len());
        // Target duration is the ceiling of the longest track's duration in
        // whole seconds, computed with integer ceil-division so no std-only
        // float intrinsic (`f64::ceil`) is needed in `no_std`.
        let mut target_secs = 0u32;
        for t in kept {
            let ticks: u64 = t
                .samples
                .iter()
                .map(|s| s.duration.unwrap_or(0) as u64)
                .sum();
            let ts = if t.spec.timescale == 0 {
                1
            } else {
                t.spec.timescale
            } as u64;
            let ceil_secs = ticks.div_ceil(ts) as u32;
            if ceil_secs > target_secs {
                target_secs = ceil_secs;
            }
            let uri = format!("{}{}.m4s", self.uri_prefix, t.spec.track_id);
            segments.push(MediaSegment {
                map: Some(MapTag {
                    uri: uri.clone(),
                    byte_range: Some(ByteRange {
                        length: init_len,
                        offset: Some(0),
                    }),
                }),
                uri,
                duration: ticks as f64 / ts as f64,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            });
        }
        if segments.is_empty() {
            return Err(Error::InvalidInput(
                "cannot package a Media whose every track is timestamp-less \
                 (section-carried): an HLS media playlist needs at least one \
                 segment with a real EXTINF duration",
            ));
        }
        let playlist = MediaPlaylist {
            version: self.version,
            target_duration: target_secs,
            media_sequence: self.media_sequence,
            discontinuity_sequence: 0,
            segments,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
            ..Default::default()
        };
        Ok(playlist.to_m3u8())
    }
}

// ---------------------------------------------------------------------------
// Helpers: box location + TrackSpec reconstruction
// ---------------------------------------------------------------------------

/// Find a top-level box by four-CC, returning its full bytes (header + body).
///
/// `pub(crate)`: shared with [`crate::progressive_demux::ProgressiveDemux`],
/// which walks the same top-level `moov` box before descending into sample
/// tables instead of movie fragments.
pub(crate) fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 0usize;
    while offset + BOX_HEADER_MIN_SIZE <= data.len() {
        let (bx, consumed) = parse_box(&data[offset..]).ok()?;
        if &bx.header.box_type.0 == fourcc {
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

/// Rebuild a [`TrackSpec`] from a parsed `trak` box (identity + codec config).
///
/// `pub(crate)`: the single shared stsd → [`CodecConfig`] reconstruction path,
/// reused verbatim by [`crate::progressive_demux::ProgressiveDemux`] (issue
/// #561) so progressive and fragmented demux never diverge on codec-config
/// recovery.
pub(crate) fn track_spec_from_trak(trak: &TrackBox) -> Result<TrackSpec> {
    let track_id = trak.tkhd.track_id;
    let mdia = trak
        .mdia
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
    let timescale = mdia
        .mdhd
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdhd" })?
        .timescale;
    let minf = mdia
        .minf
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "minf" })?;
    let stbl = minf
        .stbl
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    let stsd = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(s),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stsd" })?;
    let entry = stsd.entries.first().ok_or(Error::UnexpectedBox {
        expected: "stsd entry",
    })?;

    let config = codec_config_from_entry(entry)?;
    Ok(TrackSpec::new(track_id, timescale, config))
}

/// Build a [`SkippedTrack`] record for a `trak` [`track_spec_from_trak`]
/// rejected (media plane step-2 fix wave 1, B2/B3).
///
/// Prefers the [`Error::UnsupportedSampleEntry`] FourCC — the common
/// real-world case: an unrecognised `stsd` entry (a QuickTime hint/chapter
/// track, `c608`/`c708`, GoPro `gpmd`, ...) is the *only* way
/// `track_spec_from_trak` can fail once it has reached a `stsd` entry at all
/// ([`codec_config_from_entry`] only errors on
/// [`SampleEntryVariant::Unknown`](crate::init_segment::SampleEntryVariant::Unknown)).
/// Any other error means the `trak` was too structurally malformed to even
/// reach an entry (missing `mdia`/`mdhd`/`minf`/`stbl`/`stsd`), so there is no
/// FourCC to recover; `"unknown"` names that case honestly rather than
/// guessing.
pub(crate) fn skipped_track(err: Error) -> SkippedTrack {
    let fourcc = match &err {
        Error::UnsupportedSampleEntry { fourcc } => fourcc.clone(),
        _ => String::from("unknown"),
    };
    SkippedTrack {
        fourcc,
        reason: err.to_string(),
    }
}

/// Reconstruct a [`CodecConfig`] from an `stsd` sample entry.
///
/// Every codec the crate can output reconstructs losslessly by re-parsing the
/// config record out of the sample entry: video codecs carry a typed config
/// box on the sample entry (`avcC`/`hvcC`/`av1C`/`vpcC`); audio codecs carry
/// the config box as an [`OpaqueBox`] body (`esds`/`dac3`/`dec3`/`dOps`/
/// `dfLa`/`ddts`/`dac4`/`mhaC`) which is re-parsed here; `stpp`/`wvtt`
/// subtitle entries carry no per-track config this crate needs to decode —
/// only their format tag — and become [`CodecConfig::Subtitle`] (media plane
/// step 2d; samples stay opaque TTML/WebVTT payloads, never parsed).
///
/// Only `SampleEntryVariant::Unknown` — a sample entry naming a codec this
/// crate genuinely does not implement — yields
/// [`Error::UnsupportedSampleEntry`], propagated by [`Fmp4Demux`] rather than
/// silently skipping the track (media plane step 2d: codec coverage lands
/// *before* this strictness, so `stpp`/`wvtt`/`ac-4` — which used to hit this
/// path — no longer do) (ISO/IEC 14496-12:2015 §8.5.2 sample entries; -15
/// §5.4/§8.4 for AVC/HEVC; ISO/IEC 23008-3 §20 for MPEG-H `mha*`; ISO/IEC
/// 14496-30 §7.2/§9.2 for `stpp`/`wvtt`; ETSI TS 103 190-2 Annex E for `ac-4`).
fn codec_config_from_entry(entry: &SampleEntryVariant) -> Result<CodecConfig> {
    match entry {
        SampleEntryVariant::Avc1(avc) => Ok(CodecConfig::Avc {
            config: avc.config.clone(),
            width: avc.visual.width,
            height: avc.visual.height,
        }),
        SampleEntryVariant::Hevc1(hevc) => Ok(CodecConfig::Hevc {
            config: hevc.config.clone(),
            width: hevc.visual.width,
            height: hevc.visual.height,
        }),
        SampleEntryVariant::Vvc(vvc) => {
            // Prefer the SPS-decoded dimensions from the vvcC NAL array (the
            // authoritative coded geometry); fall back to the sample-entry
            // visual dims when the SPS is absent or cannot be decoded.
            let (width, height) = vvc
                .config
                .config
                .dimensions()
                .unwrap_or((vvc.visual.width, vvc.visual.height));
            Ok(CodecConfig::Vvc {
                config: vvc.config.clone(),
                width,
                height,
            })
        }
        SampleEntryVariant::Av01(av1) => Ok(CodecConfig::Av1 {
            config: av1.config.clone(),
            width: av1.visual.width,
            height: av1.visual.height,
        }),
        SampleEntryVariant::Vp09(vp9) => Ok(CodecConfig::Vp9 {
            config: vp9.config.clone(),
            width: vp9.visual.width,
            height: vp9.visual.height,
        }),
        SampleEntryVariant::Mp4v(mp4v) => {
            // The esds is the mp4v's first child box body (no 8-byte header).
            let esds = EsdsBox::parse_body(config_box_body(&mp4v.config_boxes, b"esds")?)?;
            Ok(CodecConfig::Mpeg2Video {
                esds,
                // Provisional geometry from the visual sample entry; refined
                // from the in-band sequence_header() once samples are collected.
                width: mp4v.visual.width,
                height: mp4v.visual.height,
            })
        }
        SampleEntryVariant::Mp4a(mp4a) => {
            let esds = esds_from_config_boxes(&mp4a.config_boxes)?;
            let oti = esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .map(|dc| dc.object_type_indication.0);
            // OTI 0x69 (MPEG-2 audio) / 0x6B (MPEG-1 audio) → legacy MPEG audio;
            // otherwise the mp4a carries AAC (ISO/IEC 14496-1 §7.2.6.6 Table 5).
            if oti == Some(OTI_MPEG2_AUDIO) || oti == Some(OTI_MPEG1_AUDIO) {
                Ok(CodecConfig::MpegAudio {
                    esds,
                    // Provisional layer; refined from the first frame header.
                    layer: MpegAudioLayer::LayerII,
                    channel_count: mp4a.channelcount,
                    sample_rate: mp4a.samplerate >> 16,
                    sample_size: mp4a.samplesize,
                })
            } else {
                Ok(CodecConfig::Aac {
                    esds,
                    channel_count: mp4a.channelcount,
                    sample_rate: mp4a.samplerate >> 16,
                    sample_size: mp4a.samplesize,
                })
            }
        }
        SampleEntryVariant::Ac3(ac3) => {
            let config = Ac3SpecificBox::parse(config_box_body(&ac3.config_boxes, b"dac3")?)?;
            Ok(CodecConfig::Ac3 {
                config,
                channel_count: ac3.channelcount,
                sample_rate: ac3.samplerate >> 16,
                sample_size: ac3.samplesize,
            })
        }
        SampleEntryVariant::Ec3(ec3) => {
            let config = Ec3SpecificBox::parse(config_box_body(&ec3.config_boxes, b"dec3")?)?;
            Ok(CodecConfig::Eac3 {
                config,
                channel_count: ec3.channelcount,
                sample_rate: ec3.samplerate >> 16,
                sample_size: ec3.samplesize,
            })
        }
        SampleEntryVariant::Opus(opus) => {
            let config = OpusSpecificBox::parse(config_box_body(&opus.config_boxes, b"dOps")?)?;
            Ok(CodecConfig::Opus {
                config,
                channel_count: opus.channelcount,
                sample_rate: opus.samplerate >> 16,
                sample_size: opus.samplesize,
            })
        }
        SampleEntryVariant::Flac(flac) => {
            let config = FlacSpecificBox::parse(config_box_body(&flac.config_boxes, b"dfLa")?)?;
            Ok(CodecConfig::Flac {
                config,
                channel_count: flac.channelcount,
                sample_rate: flac.samplerate >> 16,
                sample_size: flac.samplesize,
            })
        }
        SampleEntryVariant::Dts(dts) => {
            let config = DtsSpecificBox::parse(config_box_body(&dts.config_boxes, b"ddts")?)?;
            Ok(CodecConfig::Dts {
                config,
                codec_fourcc: dts.codec_type,
                channel_count: dts.channelcount,
                sample_rate: dts.samplerate >> 16,
                sample_size: dts.samplesize,
            })
        }
        SampleEntryVariant::Mha(mha) => {
            // The mhaC record is carried as an OpaqueBox body (no 8-byte header)
            // in the MPEG-H sample entry — re-parse it verbatim (ISO/IEC
            // 23008-3 §20). channel_count / sample_rate come from the
            // AudioSampleEntry fixed fields (the reference channel layout lives
            // in the record's `reference_channel_layout`).
            let config = MHADecoderConfigurationRecord::parse(config_box_body(
                &mha.config_boxes,
                &MHAC_FOURCC,
            )?)?;
            Ok(CodecConfig::MpegH {
                config,
                channel_count: mha.channelcount,
                sample_rate: mha.samplerate >> 16,
                sample_size: mha.samplesize,
            })
        }
        SampleEntryVariant::Ac4(ac4) => {
            let config = Ac4SpecificBox::parse(config_box_body(
                &ac4.config_boxes,
                &crate::ac4::DAC4_FOURCC,
            )?)?;
            Ok(CodecConfig::Ac4 {
                config,
                channel_count: ac4.channelcount,
                sample_rate: ac4.samplerate >> 16,
                sample_size: ac4.samplesize,
            })
        }
        // Subtitle sample entries (media plane step 2d): only the format tag
        // is reconstructed — the TTML namespace / WebVTT header block lives
        // in the sample entry but this crate has no field to carry it back
        // out to yet (see `CodecConfig::Subtitle`'s doc comment). Samples
        // stay opaque (TTML XML / WebVTT `vttc`/`vtte` boxes), never parsed.
        SampleEntryVariant::Stpp(_) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::Ttml,
        }),
        SampleEntryVariant::Wvtt(_) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::WebVtt,
        }),
        // Genuinely unimplemented sample entry: name it so the caller learns
        // which track was rejected, rather than a generic message.
        SampleEntryVariant::Unknown(entry) => Err(Error::UnsupportedSampleEntry {
            fourcc: String::from_utf8_lossy(&entry.box_type).into_owned(),
        }),
    }
}

/// Refine the provisional geometry/layer of a legacy-codec config from the
/// first coded sample's in-band header (the sample entry alone cannot carry it):
/// MPEG-2 video takes width/height from the `sequence_header()`, MPEG audio
/// takes its layer from the first frame header. A no-op for other codecs and
/// when the header cannot be decoded (the provisional values then stand).
///
/// `pub(crate)`: shared with [`crate::progressive_demux::ProgressiveDemux`].
pub(crate) fn refine_legacy_config(config: &mut CodecConfig, samples: &[Sample]) {
    let Some(first) = samples.first() else {
        return;
    };
    match config {
        CodecConfig::Mpeg2Video { width, height, .. } => {
            if let Ok(sh) = Mpeg2SeqHeader::find(&first.data) {
                *width = sh.width;
                *height = sh.height;
            }
        }
        CodecConfig::MpegAudio { layer, .. } => {
            if let Ok(hdr) = MpegAudioFrameHeader::parse(&first.data) {
                *layer = hdr.layer;
            }
        }
        _ => {}
    }
}

/// Re-parse the `esds` box body preserved as an [`OpaqueBox`] in an `mp4a`
/// sample entry's `config_boxes` into an owned [`EsdsBox`].
fn esds_from_config_boxes(boxes: &[OpaqueBox]) -> Result<EsdsBox> {
    // `OpaqueBox.data` is the FullBox *body* (no size/type header); parse it
    // directly (EsdsBox is fully owned, so no lifetime is retained).
    EsdsBox::parse_body(config_box_body(boxes, b"esds")?)
}

/// Locate a config box by FourCC among a sample entry's `config_boxes` and
/// return its body bytes (the [`OpaqueBox`] holds the box body, no 8-byte
/// header), so a codec-specific `Parse` can re-parse the decoder config record.
fn config_box_body<'b>(boxes: &'b [OpaqueBox], fourcc: &[u8; 4]) -> Result<&'b [u8]> {
    boxes
        .iter()
        .find(|b| &b.box_type == fourcc)
        .map(|b| b.data.as_slice())
        .ok_or(Error::UnexpectedBox {
            expected: "config box in audio sample entry",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitle_entries::WvttSampleEntry;

    /// ffmpeg's `mov` muxer cannot produce a `wvtt` sample entry (only
    /// `stpp`/TTML — see `tests/fixtures/mp4/cmaf/PROVENANCE.md`), so WebVTT
    /// demux coverage is exercised directly here: synthesise a real `wvtt`
    /// sample entry with the crate's own builder (already covered by
    /// `subtitle_entries::tests::wvtt_sample_entry_round_trip`) and feed it
    /// through the same reconstruction path the `stpp` fixture exercises.
    #[test]
    fn wvtt_sample_entry_demuxes_to_subtitle_webvtt() {
        let wvtt = WvttSampleEntry::new("WEBVTT\n");
        let entry = SampleEntryVariant::Wvtt(alloc::boxed::Box::new(wvtt));
        let config = codec_config_from_entry(&entry).expect("wvtt must now demux");
        assert!(
            matches!(
                config,
                CodecConfig::Subtitle {
                    format: SubtitleFormat::WebVtt
                }
            ),
            "wvtt must map to CodecConfig::Subtitle{{format: SubtitleFormat::WebVtt}}, got {config:?}"
        );
    }

    /// A genuinely unrecognised sample entry must be named in the error
    /// (media plane step 2d Phase 2), not just a generic "unsupported"
    /// message.
    #[test]
    fn unknown_sample_entry_errors_naming_the_fourcc() {
        let unknown = OpaqueBox::new(*b"xyz9", vec![1, 2, 3]);
        let entry = SampleEntryVariant::Unknown(unknown);
        let err = codec_config_from_entry(&entry).unwrap_err();
        match err {
            Error::UnsupportedSampleEntry { fourcc } => assert_eq!(fourcc, "xyz9"),
            other => panic!("expected UnsupportedSampleEntry, got {other:?}"),
        }
    }

    // ── Gapped / discontinuous fMP4 (per-fragment `tfdt` re-seed) ───────────

    /// A minimal but real `avcC` — the same Baseline SPS/PPS pair
    /// `avc_config::tests::make_minimal_avcc_body` builds, expressed
    /// structurally so this test needs no fixture file.
    fn minimal_avc_config() -> crate::avc_config::AVCConfigurationBox {
        use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
        use crate::nalu_types::{AvcPps, AvcSps};
        AVCConfigurationBox {
            config: AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 66,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40])],
                pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            },
        }
    }

    /// PROVENANCE: synthesised, not a captured file. No committed fixture in
    /// this workspace carries a *gapped* fMP4 (every CMAF capture here is
    /// contiguous), so the two fragments are built with this crate's own
    /// [`build_init_segment`] / [`build_media_segment`] — i.e. the exact
    /// `moof`/`tfdt`/`trun` layout the muxer emits (ISO/IEC 14496-12 §8.8),
    /// not hand-rolled bytes — with the second fragment's
    /// `base_media_decode_time` deliberately advanced past the running sum of
    /// the first fragment's `trun` durations.
    ///
    /// Seeding `next_dts` from only the FIRST `tfdt` and then pure-summing
    /// durations makes every post-gap sample's absolute `dts` short by exactly
    /// the gap, which re-muxes to a wrong `tfdt`/PES stamp and permanently
    /// desyncs A/V. `tfdt` is per-fragment and authoritative (§8.8.12).
    #[test]
    fn gapped_fmp4_reseeds_absolute_dts_from_each_fragments_own_tfdt() {
        const TIMESCALE: u32 = 90_000;
        const FRAME_DUR: u32 = 3_000;
        const FRAMES_PER_FRAGMENT: u32 = 4;
        /// Decode time the second fragment declares — one whole extra frame
        /// beyond where the first fragment's durations leave off.
        const GAP_TICKS: u64 = FRAME_DUR as u64;

        let spec = TrackSpec::new(
            1,
            TIMESCALE,
            CodecConfig::Avc {
                config: minimal_avc_config(),
                width: 16,
                height: 16,
            },
        );
        let init = build_init_segment(core::slice::from_ref(&spec), TIMESCALE)
            .expect("init segment builds");

        let frames = |base: i64| -> Vec<Sample> {
            (0..FRAMES_PER_FRAGMENT)
                .map(|i| {
                    // One length-prefixed non-IDR NAL; content is irrelevant
                    // here (this test is about timing, not codec bytes).
                    let nal = [0x41u8, 0xAA, 0xBB];
                    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                    data.extend_from_slice(&nal);
                    let dts = base + i64::from(i) * i64::from(FRAME_DUR);
                    Sample::new(data, Some(dts), Some(dts), Some(FRAME_DUR), i == 0)
                })
                .collect()
        };

        let first_base = 0u64;
        let contiguous_second_base = u64::from(FRAMES_PER_FRAGMENT * FRAME_DUR);
        let gapped_second_base = contiguous_second_base + GAP_TICKS;

        let f1 = frames(first_base as i64);
        let f2 = frames(gapped_second_base as i64);
        let seg1 = build_media_segment(1, &[FragmentTrackData::new(1, first_base, &f1)])
            .expect("fragment 1 builds");
        let seg2 = build_media_segment(2, &[FragmentTrackData::new(1, gapped_second_base, &f2)])
            .expect("fragment 2 builds");

        let mut file = init;
        file.extend_from_slice(&seg1);
        file.extend_from_slice(&seg2);

        let media = Fmp4Demux::new()
            .unpackage(&file)
            .expect("demux gapped fMP4");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples.len(),
            (FRAMES_PER_FRAGMENT * 2) as usize,
            "both fragments' samples must be present"
        );
        assert_eq!(
            track.start_decode_time, first_base,
            "start_decode_time stays the FIRST fragment's anchor, not the last"
        );

        // The bite: the first post-gap sample must land on the second
        // fragment's own declared tfdt, NOT on the running duration sum.
        let first_post_gap = &track.samples[FRAMES_PER_FRAGMENT as usize];
        assert_eq!(
            first_post_gap.dts,
            Some(gapped_second_base as i64),
            "post-gap dts must come from the second fragment's tfdt \
             ({gapped_second_base}), not the pure duration sum \
             ({contiguous_second_base})"
        );

        // ...and so must every later sample in that fragment.
        for (i, s) in track.samples[FRAMES_PER_FRAGMENT as usize..]
            .iter()
            .enumerate()
        {
            let expected = gapped_second_base as i64 + (i as i64) * i64::from(FRAME_DUR);
            assert_eq!(s.dts, Some(expected), "sample {i} after the gap");
        }
    }

    /// Regression guard for the fix above: a **contiguous** two-fragment
    /// stream (every `tfdt` exactly equal to the running duration sum) must
    /// demux to exactly the same absolute dts sequence it always did —
    /// re-seeding per fragment must be a no-op when there is no gap.
    #[test]
    fn contiguous_fmp4_dts_is_unchanged_by_the_per_fragment_tfdt_reseed() {
        const TIMESCALE: u32 = 90_000;
        const FRAME_DUR: u32 = 3_000;
        const FRAMES: u32 = 3;

        let spec = TrackSpec::new(
            7,
            TIMESCALE,
            CodecConfig::Avc {
                config: minimal_avc_config(),
                width: 16,
                height: 16,
            },
        );
        let init = build_init_segment(core::slice::from_ref(&spec), TIMESCALE).unwrap();
        let frames = |base: i64| -> Vec<Sample> {
            (0..FRAMES)
                .map(|i| {
                    let nal = [0x41u8, 0xAA, 0xBB];
                    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                    data.extend_from_slice(&nal);
                    let dts = base + i64::from(i) * i64::from(FRAME_DUR);
                    Sample::new(data, Some(dts), Some(dts), Some(FRAME_DUR), i == 0)
                })
                .collect()
        };
        let second_base = u64::from(FRAMES * FRAME_DUR);
        let f1 = frames(0);
        let f2 = frames(second_base as i64);
        let mut file = init;
        file.extend_from_slice(
            &build_media_segment(1, &[FragmentTrackData::new(7, 0, &f1)]).unwrap(),
        );
        file.extend_from_slice(
            &build_media_segment(2, &[FragmentTrackData::new(7, second_base, &f2)]).unwrap(),
        );

        let media = Fmp4Demux::new().unpackage(&file).unwrap();
        let got: Vec<i64> = media.tracks[0]
            .samples
            .iter()
            .filter_map(|s| s.dts)
            .collect();
        let want: Vec<i64> = (0..FRAMES * 2)
            .map(|i| i64::from(i) * i64::from(FRAME_DUR))
            .collect();
        assert_eq!(got, want, "a gapless stream must be unaffected by the fix");
    }

    // ── HlsPackager: no knowingly-wrong #EXTINF:0.000 ──────────────────────

    /// A timestamp-less (section-carried) track has `duration: None` on every
    /// sample and therefore no truthful `EXTINF` — it is omitted from the
    /// playlist rather than rendered as `#EXTINF:0.000`, which RFC 8216
    /// §4.3.2.1 defines as a real playback duration a player would honour.
    #[test]
    fn hls_packager_omits_a_timestampless_track_instead_of_emitting_extinf_zero() {
        const TIMESCALE: u32 = 90_000;
        const FRAME_DUR: u32 = 3_000;

        let timed = Track::new(
            TrackSpec::new(
                1,
                TIMESCALE,
                CodecConfig::Avc {
                    config: minimal_avc_config(),
                    width: 16,
                    height: 16,
                },
            ),
            (0..3)
                .map(|i| {
                    Sample::new(
                        vec![0u8; 4],
                        Some(i64::from(i) * i64::from(FRAME_DUR)),
                        Some(i64::from(i) * i64::from(FRAME_DUR)),
                        Some(FRAME_DUR),
                        true,
                    )
                })
                .collect::<Vec<_>>(),
        );
        // A section-carried data track: no dts, no pts, no duration — ever.
        let sections = Track::new(
            TrackSpec::new(
                2,
                TIMESCALE,
                CodecConfig::Data {
                    stream_type: 0x86,
                    descriptors: Vec::new(),
                    carriage: crate::ir::DataCarriage::Sections,
                },
            ),
            vec![Sample::new(vec![0xFCu8; 8], None, None, None, true)],
        );

        let playlist = HlsPackager::default()
            .package(&Media::new(vec![timed, sections], TIMESCALE))
            .expect("a Media with one timed track still packages");
        let zero_extinf = playlist.lines().any(|l| {
            l.strip_prefix("#EXTINF:")
                .and_then(|rest| rest.trim_end_matches(',').parse::<f64>().ok())
                .is_some_and(|secs| secs == 0.0)
        });
        assert!(
            !zero_extinf,
            "no zero-duration EXTINF may be emitted, got:\n{playlist}"
        );
        assert!(
            !playlist.contains("seg2.m4s"),
            "the timestamp-less track must be omitted entirely, got:\n{playlist}"
        );
        assert!(
            playlist.contains("seg1.m4s"),
            "the timed track must still be present, got:\n{playlist}"
        );

        // Every track timestamp-less => nothing truthful to render at all.
        let only_sections = Track::new(
            TrackSpec::new(
                2,
                TIMESCALE,
                CodecConfig::Data {
                    stream_type: 0x86,
                    descriptors: Vec::new(),
                    carriage: crate::ir::DataCarriage::Sections,
                },
            ),
            vec![Sample::new(vec![0xFCu8; 8], None, None, None, true)],
        );
        assert!(
            HlsPackager::default()
                .package(&Media::new(vec![only_sections], TIMESCALE))
                .is_err(),
            "an all-timestamp-less Media must error, not render an empty playlist"
        );
    }
}
