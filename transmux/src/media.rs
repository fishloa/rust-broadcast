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
//! Sample [`Decrypt`](broadcast_common::Decrypt) is implemented for CENC/CBCS
//! (ISO/IEC 23001-7 AES-CTR / AES-CBC-pattern) by
//! [`CencDecryptor`](crate::cenc_decrypt::CencDecryptor), and the inverse
//! [`Encrypt`](broadcast_common::Encrypt) by
//! [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor) — both behind the
//! `cenc` feature (issues #465/#564). Either direction records its per-track
//! crypto metadata onto [`Track::encryption`].

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Package, Parse, Unpackage};

use crate::ac3::{Ac3SpecificBox, Ec3SpecificBox};
use crate::box_types::{BOX_HEADER_MIN_SIZE, parse_box};
use crate::dts::DtsSpecificBox;
use crate::error::{Error, Result};
use crate::flac::FlacSpecificBox;
use crate::hls::{MediaPlaylist, MediaSegment};
use crate::init_segment::{MovieBox, OpaqueBox, SampleEntryVariant, StblChild, TrackBox};
use crate::movie_fragment::MovieFragmentBox;
use crate::mp4esds::EsdsBox;
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer};
use crate::mpegh::{MHAC_FOURCC, MHADecoderConfigurationRecord};
use crate::opus::OpusSpecificBox;
use crate::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};

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
    /// Absolute decode time of the track's **first** sample, in this track's
    /// media timescale ([`TrackSpec::timescale`]) ticks.
    ///
    /// [`Sample`] timing is stored *relatively* (per-sample [`Sample::duration`];
    /// a sample's DTS is the running sum of preceding durations, its PTS is
    /// `DTS + composition_offset`). This field is the missing absolute anchor:
    /// the DTS the first sample sits at on the presentation timeline. It maps
    /// directly onto the fragment `tfdt` `baseMediaDecodeTime`
    /// (ISO/IEC 14496-12:2015 §8.8.12) that [`CmafMux`] writes for the first
    /// media segment.
    ///
    /// Demuxers that recover an absolute timeline populate it ([`Fmp4Demux`]
    /// from the first movie fragment's `tfdt`; [`TsDemux`](crate::ts_demux::TsDemux)
    /// from the first sample's DTS). Demuxers whose source carries no absolute anchor
    /// (FLV, WebM, MPEG Program Stream, RTMP, RTP) leave it `0`. It is the
    /// input to the timeline transforms in [`crate::rebase`] (rebase-to-zero,
    /// offset, 33-bit MPEG wrap-unroll — ISO/IEC 13818-1 33-bit timestamps).
    pub start_decode_time: u64,
    /// CENC/CBCS crypto metadata for this track's samples, or `None` for
    /// cleartext. Populated by [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor)'s
    /// [`Encrypt`](broadcast_common::Encrypt) impl (issue #564) or by a
    /// demuxer of an already-protected source (e.g.
    /// [`crate::cenc_decrypt::CencDecryptor::demux`]); read by the muxer's
    /// crypto-box emission (`sinf`/`senc`/`saio`/`saiz`).
    pub encryption: Option<TrackEncryption>,
}

impl Track {
    /// Create a track from its spec and samples, anchored at decode time `0`.
    pub fn new(spec: TrackSpec, samples: Vec<Sample>) -> Self {
        Self {
            spec,
            samples,
            start_decode_time: 0,
            encryption: None,
        }
    }

    /// Create a track from its spec and samples, anchored at an absolute
    /// `start_decode_time` (in the track's media timescale).
    pub fn new_at(spec: TrackSpec, samples: Vec<Sample>, start_decode_time: u64) -> Self {
        Self {
            spec,
            samples,
            start_decode_time,
            encryption: None,
        }
    }

    /// Set the absolute [`start_decode_time`](Self::start_decode_time) anchor,
    /// returning `self` (builder style).
    pub fn with_start_decode_time(mut self, start_decode_time: u64) -> Self {
        self.start_decode_time = start_decode_time;
        self
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

/// Per-track CENC/CBCS crypto carrier attached to [`Track::encryption`].
///
/// The IR-side dual of the metadata [`crate::cenc_decrypt::CencDecryptor`]
/// harvests from an already-protected file's `sinf`/`tenc`/`senc` boxes: this
/// is exactly the shape [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor)
/// produces (issue #564), and it is what the muxer's crypto-box emission
/// (`sinf`/`senc`/`saio`/`saiz`) reads back to (re)build the boxes without
/// needing to know how the samples were protected.
#[derive(Debug, Clone)]
pub struct TrackEncryption {
    /// The protection scheme (`cenc` AES-CTR or `cbcs` AES-CBC pattern).
    pub scheme: crate::cenc::CencScheme,
    /// Track-level crypto defaults — KID, per-sample IV size, and (for
    /// `cbcs`) the pattern's `crypt`:`skip` block counts — ISO/IEC 23001-7
    /// §12.2.
    pub tenc: crate::cenc::TrackEncryptionBox,
    /// Per-sample IV + subsample map, in decode order — ISO/IEC 23001-7 §12.3.
    /// `samples.len()` must equal the owning [`Track`]'s `samples.len()`.
    pub samples: Vec<crate::cenc::SampleEncryptionEntry>,
}

/// One PCR observation from a TS adaptation field (ISO/IEC 13818-1 §2.4.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcrSample {
    /// `program_clock_reference` as a 27 MHz value (`base * 300 + extension`).
    pub pcr_27mhz: u64,
    /// PID the PCR was carried on.
    pub pid: u16,
    /// 0-based index of the 188-byte packet in the demuxed input.
    pub packet_index: u64,
    /// The adaptation field's `discontinuity_indicator` (§2.4.3.5).
    pub discontinuity: bool,
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
    /// PCR timeline recovered from the source, in wire order
    /// ([`PcrSample`], ISO/IEC 13818-1 §2.4.3.4). Empty for every demuxer that
    /// does not read a TS adaptation field (i.e. every non-[`TsDemux`](crate::ts_demux::TsDemux) source).
    pub pcr: Vec<PcrSample>,
}

impl Media {
    /// Create a `Media` from tracks and a movie timescale, with an empty PCR
    /// timeline.
    pub fn new(tracks: Vec<Track>, movie_timescale: u32) -> Self {
        Self {
            tracks,
            movie_timescale,
            pcr: Vec::new(),
        }
    }

    /// Attach a PCR timeline, returning `self` (builder style).
    pub fn with_pcr(mut self, pcr: Vec<PcrSample>) -> Self {
        self.pcr = pcr;
        self
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
    /// Absolute decode-time anchor, taken from the **first** movie fragment's
    /// `tfdt` seen for this track (`None` until that fragment is absorbed; a
    /// stream with no `tfdt` at all yields a `0` anchor).
    start_decode_time: Option<u64>,
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

        // A track whose sample entry the crate cannot reconstruct into a
        // `CodecConfig` (an unknown/unsupported codec) is skipped, never fatal:
        // its samples are simply not collected in step 2.
        let mut builders: Vec<TrackBuilder> = Vec::with_capacity(moov.tracks.len());
        for trak in &moov.tracks {
            if let Ok(spec) = track_spec_from_trak(trak) {
                builders.push(TrackBuilder {
                    spec,
                    samples: Vec::new(),
                    start_decode_time: None,
                });
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
        // The absolute decode-time anchor is the baseMediaDecodeTime of the
        // FIRST fragment seen for this track (ISO/IEC 14496-12:2015 §8.8.12).
        if builder.start_decode_time.is_none() {
            if let Some(tfdt) = &traf.tfdt {
                builder.start_decode_time = Some(tfdt.base_media_decode_time());
            }
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
                    // fMP4 sources carry no per-sample source-container
                    // timestamps distinct from the fragment's own tfdt/trun
                    // timing (see `Sample::source_timing`).
                    source_timing: None,
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
        // Opaque `CodecConfig::Data` tracks have no ISOBMFF sample entry in
        // this crate (issue #557/#576) — omit them from the fMP4/CMAF mux
        // path entirely (init segment AND media segment) rather than
        // erroring, so a TS multiplex mixing carriable and opaque streams
        // still produces a valid CMAF output for its carriable tracks.
        // `select_tracks_by` itself errors if nothing carriable remains.
        let filtered;
        let media: &Media = if media.tracks.iter().any(|t| t.spec.config.is_opaque_data()) {
            filtered = media.select_tracks_by(|t| !t.spec.config.is_opaque_data())?;
            &filtered
        } else {
            media
        };

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
                discontinuous: false,
                parts: vec![],
            });
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

/// Reconstruct a [`CodecConfig`] from an `stsd` sample entry.
///
/// Every codec the crate can output reconstructs losslessly by re-parsing the
/// config record out of the sample entry: video codecs carry a typed config
/// box on the sample entry (`avcC`/`hvcC`/`av1C`/`vpcC`); audio codecs carry
/// the config box as an [`OpaqueBox`] body (`esds`/`dac3`/`dec3`/`dOps`/
/// `dfLa`/`ddts`/`mhaC`) which is re-parsed here. Sample entries the crate does
/// not mux (e.g. `stpp`/`wvtt`/`ac-4`, and `SampleEntryVariant::Unknown`)
/// yield [`Error::UnexpectedBox`] so [`Fmp4Demux`] can skip the track
/// (ISO/IEC 14496-12:2015 §8.5.2 sample entries; -15 §5.4/§8.4 for AVC/HEVC;
/// ISO/IEC 23008-3 §20 for MPEG-H `mha*`).
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
        // Sample entries transmux does not (re)mux to an elementary track.
        SampleEntryVariant::Ac4(_)
        | SampleEntryVariant::Stpp(_)
        | SampleEntryVariant::Wvtt(_)
        | SampleEntryVariant::Unknown(_) => Err(Error::UnexpectedBox {
            expected: "a codec sample entry transmux can reconstruct (avc1/hvc1/vvc1/mp4v/av01/vp09/mp4a/ac-3/ec-3/Opus/fLaC/dts*/mha*)",
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
