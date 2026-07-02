//! Media intermediate representation (IR) + the any-to-any hub impls.
//!
//! This module is the transmux side of the `broadcast-common` container-mux
//! vocabulary ([`broadcast_common::Unpackage`] / [`broadcast_common::Package`]).
//! It defines a concrete [`Media`] IR — elementary [`Track`]s of coded
//! [`Sample`]s — and the packagers/depackagers that convert between that IR and
//! concrete container forms:
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
//! Sample `Encrypt` / `Decrypt` impls are deferred (issue #465); only the
//! `broadcast-common` trait definitions ship for those.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Package, Parse, Unpackage};

use crate::box_types::{parse_box, BOX_HEADER_MIN_SIZE};
use crate::error::{Error, Result};
use crate::hls::{MediaPlaylist, MediaSegment};
use crate::init_segment::{MovieBox, OpaqueBox, SampleEntryVariant, StblChild, TrackBox};
use crate::movie_fragment::MovieFragmentBox;
use crate::mp4esds::EsdsBox;
use crate::pipeline::{
    build_init_segment, build_media_segment, CodecConfig, FragmentTrackData, Sample, TrackSpec,
};

/// `sample_is_non_sync_sample` bit within a 32-bit `sample_flags` word
/// (ISO/IEC 14496-12:2015 §8.8.3.1, bit `[16]`). Set = the sample is **not** a
/// sync sample (random-access point).
const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

/// Default movie timescale used when a source does not specify one.
const DEFAULT_MOVIE_TIMESCALE: u32 = 1000;

/// One elementary track: its identity/codec config plus its coded samples.
///
/// A thin wrapper pairing the existing [`TrackSpec`] (track_id + timescale +
/// [`CodecConfig`]) with the track's decode-ordered [`Sample`]s.
#[derive(Debug, Clone)]
pub struct Track {
    /// Track identity + codec configuration (used to build the init segment).
    pub spec: TrackSpec,
    /// The track's coded access units, in decode order.
    pub samples: Vec<Sample>,
}

impl Track {
    /// Create a track from its spec and samples.
    pub fn new(spec: TrackSpec, samples: Vec<Sample>) -> Self {
        Self { spec, samples }
    }

    /// Track ID (1-based, unique within the movie).
    pub fn track_id(&self) -> u32 {
        self.spec.track_id
    }

    /// Media timescale (ticks per second).
    pub fn timescale(&self) -> u32 {
        self.spec.timescale
    }

    /// Codec configuration.
    pub fn config(&self) -> &CodecConfig {
        &self.spec.config
    }
}

/// The media intermediate representation: a set of elementary [`Track`]s.
///
/// This is the hub's neutral form. [`Unpackage`] impls (e.g. [`Fmp4Demux`])
/// produce a `Media`; [`Package`] impls (e.g. [`CmafMux`], [`HlsPackager`])
/// consume one.
#[derive(Debug, Clone)]
pub struct Media {
    /// Elementary tracks, in the order they appear in the source movie.
    pub tracks: Vec<Track>,
    /// Movie timescale (`mvhd.timescale`), preserved for lossless re-muxing.
    pub movie_timescale: u32,
}

impl Media {
    /// Create a `Media` from tracks and a movie timescale.
    pub fn new(tracks: Vec<Track>, movie_timescale: u32) -> Self {
        Self {
            tracks,
            movie_timescale,
        }
    }
}

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

impl<'a> Fmp4Demux<'a> {
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

        let mut builders: Vec<TrackBuilder> = Vec::with_capacity(moov.tracks.len());
        for trak in &moov.tracks {
            let spec = track_spec_from_trak(trak)?;
            builders.push(TrackBuilder {
                spec,
                samples: Vec::new(),
            });
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
            .map(|b| Track {
                spec: b.spec,
                samples: b.samples,
            })
            .collect();
        Ok(Media {
            tracks,
            movie_timescale,
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
                let composition_offset = ts.sample_composition_time_offset.unwrap_or(0);

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
                builder.samples.push(Sample {
                    data: file[start..end].to_vec(),
                    duration,
                    is_sync,
                    composition_offset,
                });
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
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let movie_timescale = if media.movie_timescale == 0 {
            DEFAULT_MOVIE_TIMESCALE
        } else {
            media.movie_timescale
        };
        let mut out = build_init_segment(&specs, movie_timescale)?;

        // base_media_decode_time = 0 for each track's single-segment fragment.
        let fragments: Vec<FragmentTrackData<'_>> = media
            .tracks
            .iter()
            .map(|t| FragmentTrackData {
                track_id: t.spec.track_id,
                base_media_decode_time: 0,
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
        let mut segments = Vec::with_capacity(media.tracks.len());
        // Target duration is the ceiling of the longest track's duration in
        // whole seconds, computed with integer ceil-division so no std-only
        // float intrinsic (`f64::ceil`) is needed in `no_std`.
        let mut target_secs = 0u32;
        for t in &media.tracks {
            let ticks: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
            let ts = if t.spec.timescale == 0 {
                1
            } else {
                t.spec.timescale
            } as u64;
            let ceil_secs = ticks.div_ceil(ts) as u32;
            if ceil_secs > target_secs {
                target_secs = ceil_secs;
            }
            segments.push(MediaSegment {
                uri: format!("{}{}.m4s", self.uri_prefix, t.spec.track_id),
                duration: ticks as f64 / ts as f64,
            });
        }
        let playlist = MediaPlaylist {
            version: self.version,
            target_duration: target_secs,
            media_sequence: self.media_sequence,
            segments,
            endlist: true,
            extra_tags: vec![],
        };
        Ok(playlist.to_m3u8())
    }
}

// ---------------------------------------------------------------------------
// Helpers: box location + TrackSpec reconstruction
// ---------------------------------------------------------------------------

/// Find a top-level box by four-CC, returning its full bytes (header + body).
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
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
fn track_spec_from_trak(trak: &TrackBox) -> Result<TrackSpec> {
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
    Ok(TrackSpec {
        track_id,
        timescale,
        config,
    })
}

/// Reconstruct a [`CodecConfig`] from an `stsd` sample entry.
///
/// AVC (`avc1`) and AAC (`mp4a`) reconstruct losslessly (their config records
/// are re-parsed from the sample entry). Other codecs whose config is carried
/// as an opaque config box are not yet reversible here and return
/// [`Error::UnexpectedBox`]; extending them is follow-up work (issue #464).
fn codec_config_from_entry(entry: &SampleEntryVariant) -> Result<CodecConfig> {
    match entry {
        SampleEntryVariant::Avc1(avc) => Ok(CodecConfig::Avc {
            config: avc.config.clone(),
            width: avc.visual.width,
            height: avc.visual.height,
        }),
        SampleEntryVariant::Mp4a(mp4a) => {
            let esds = esds_from_config_boxes(&mp4a.config_boxes)?;
            Ok(CodecConfig::Aac {
                esds,
                channel_count: mp4a.channelcount,
                sample_rate: mp4a.samplerate >> 16,
                sample_size: mp4a.samplesize,
            })
        }
        _ => Err(Error::UnexpectedBox {
            expected:
                "avc1 or mp4a sample entry (other codecs' Unpackage config reconstruction is deferred)",
        }),
    }
}

/// Re-parse the `esds` box body preserved as an [`OpaqueBox`] in an `mp4a`
/// sample entry's `config_boxes` into an owned [`EsdsBox`].
fn esds_from_config_boxes(boxes: &[OpaqueBox]) -> Result<EsdsBox> {
    let ob = boxes
        .iter()
        .find(|b| &b.box_type == b"esds")
        .ok_or(Error::UnexpectedBox {
            expected: "esds config box in mp4a entry",
        })?;
    // `OpaqueBox.data` is the FullBox *body* (no size/type header); parse it
    // directly (EsdsBox is fully owned, so no lifetime is retained).
    EsdsBox::parse_body(&ob.data)
}
