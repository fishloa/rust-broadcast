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
use broadcast_hls::{ByteRange, DecimalSeconds, MapTag, MediaPlaylist, MediaSegment};

use crate::ac4::Ac4SpecificBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, parse_box};
use crate::display_track::{SpsTracker, TrackCodec};
use crate::dts::DtsSpecificBox;
use crate::error::{Error, Result};
use crate::flac::FlacSpecificBox;
use crate::init_segment::{
    MovieBox, OpaqueBox, SampleDescriptionBox, SampleEntryVariant, StblChild, TrackBox,
    sampling_rate_override,
};
use crate::ir::{
    AspectRatioIdc, CodecConfig, ColourDescription, DisplayGeometry, DisplayInfo,
    FragmentTrackData, InputDegradation, Sample, SubtitleFormat, SubtitleSampleEntry, TrackSpec,
    VideoFormat, VideoSignalType,
};
use crate::movie_fragment::{MovieFragmentBox, SAMPLE_FLAG_IS_NON_SYNC};
use crate::mp4esds::EsdsBox;
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer};
use crate::mpegh::{MHAC_FOURCC, MHADecoderConfigurationRecord};
use crate::opus::OpusSpecificBox;
use crate::pipeline::{build_init_segment, build_media_segment};
use crate::visual_ext::{ColourInformationBox, PixelAspectRatioBox};
use alloc::sync::Arc;

/// [`Media`], [`Track`], [`TrackEncryption`], [`PcrSample`] moved to
/// [`crate::ir`] (media plane step 2a) — re-exported here so every existing
/// `crate::media::`/`transmux::media::` path keeps resolving unchanged.
pub use crate::ir::{Media, PcrSample, SkippedTrack, Track, TrackEncryption};

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
    /// Whether this track's sample entry carries parameter sets in-band
    /// (avc3/avc4/hev1), so per-AU display tagging applies.
    in_band: bool,
    /// Container-level boxes from the sample entry, used to overlay `pasp` /
    /// `colr nclx` onto every tag the in-band tracker produces.
    entry_extra_boxes: Vec<crate::sample_entries::OpaqueBox>,
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
        let trex_defaults: &[crate::init_segment::TrackExtendsBox] =
            moov.mvex.as_ref().map_or(&[], |m| m.trex.as_slice());

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
                Ok(spec) => {
                    let (in_band, entry_extra_boxes) = track_entry_inband_and_extras(trak);
                    builders.push(TrackBuilder {
                        spec,
                        samples: Vec::new(),
                        start_decode_time: None,
                        next_dts: 0,
                        in_band,
                        entry_extra_boxes,
                    })
                }
                Err(err) => skipped.push(skipped_track(err, trak.tkhd.track_id)),
            }
        }

        // 2. Walk every top-level box; each `moof` pairs with the next `mdat`.
        //    Sample bytes are resolved via the trun `data_offset` measured from
        //    the enclosing `moof` (default-base-is-moof), so we track the moof's
        //    absolute file offset.
        let mut offset = 0usize;
        // Every `mdat` payload range, scanned once for the whole file (not once
        // per `traf`).
        let mdats = crate::frag_offsets::mdat_ranges(input);
        // Fragments waiting for their `mdat`. A queue, not a single slot:
        // `moof`, `moof`, `mdat` is legal (ISO/IEC 14496-12:2015 §8.8.4 allows
        // any number of `moof`s, and CMAF writes one per track for
        // CMF2/multi-track segments), and a single slot silently dropped every
        // fragment before the last (audit r05-W26). Each `mdat` resolves the
        // fragment that follows it, per §8.8.4's "the `mdat` carrying a
        // fragment's data follows that fragment".
        let mut pending_moofs: Vec<(usize, MovieFragmentBox)> = Vec::new();
        // The zero-width-`trun` sample allowance is shared by every `moof`
        // queued here (and refilled once an `mdat` consumes them): a per-`trun`
        // or per-`moof` cap lets N tiny fragments held before one `mdat` cost
        // N x 40 MiB of `TrunSample`s.
        let mut zero_width_budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
        // A `moof` whose body does not parse is skipped and reported (audit
        // T16, #1144): the box framing is intact, so the next box is read from
        // a correct offset and one corrupt fragment costs only itself.
        let mut degraded: Vec<InputDegradation> = Vec::new();
        while offset + BOX_HEADER_MIN_SIZE <= input.len() {
            // A truncated *final* box is ordinary in a live capture or an
            // interrupted recording, so a tail whose declared size runs past
            // the end of the data ends the walk rather than failing the whole
            // file — nothing follows it to be mis-framed.
            //
            // Only that case. Any other parse failure (`BoxSizeUnderflow` from
            // a size of 2..7, a bad `largesize`, a `uuid` header cut short)
            // means the *framing* is wrong, so every box after it would be
            // read from a bogus offset; treating it as end-of-data silently
            // truncated the file at the first corrupt box (audit item 4).
            let (bx, consumed) = match parse_box(&input[offset..]) {
                Ok(parsed) => parsed,
                // Every "the declared box does not fit the bytes present"
                // error ends the walk: the box is the last one and the file
                // was cut inside its header. `parse_box` reports these as three
                // variants — a body past the end (`BufferTooShort`), a `uuid`
                // usertype cut short (`UuidBufferTooShort`), and a `largesize`
                // field cut short (`LargesizeBufferTooShort`) — and all three
                // mean the same thing at the tail (audit item 9, round 3).
                Err(Error::BufferTooShort { .. })
                | Err(Error::UuidBufferTooShort { .. })
                | Err(Error::LargesizeBufferTooShort { .. }) => break,
                Err(e) => return Err(e),
            };
            let ty = bx.header.box_type.0;
            if &ty == b"moof" {
                match MovieFragmentBox::parse_body_budgeted(bx.body, &mut zero_width_budget) {
                    Ok(mut moof) => {
                        apply_trex_defaults(&mut moof, trex_defaults);
                        pending_moofs.push((offset, moof));
                    }
                    Err(_) => degraded.push(InputDegradation::MalformedUnit {
                        position: offset as u64,
                    }),
                }
            } else if &ty == b"mdat" {
                // A `moof`'s samples live in the `mdat` that follows it; a
                // segment carrying several `moof`s before one `mdat` resolves
                // them in order.
                for (moof_off, moof) in pending_moofs.drain(..) {
                    for _ in 0..absorb_fragment(input, &mdats, moof_off, &moof, &mut builders) {
                        degraded.push(InputDegradation::MalformedUnit {
                            position: moof_off as u64,
                        });
                    }
                }
                zero_width_budget = crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES;
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
                normalise_track_nal_lengths(&mut b.spec, &mut b.samples)?;
                if b.in_band {
                    tag_fmp4_inband_display(&mut b);
                }
                Ok(Track {
                    spec: b.spec,
                    samples: b.samples,
                    // First-fragment tfdt baseMediaDecodeTime; 0 when the stream
                    // carried no tfdt at all (ISO/IEC 14496-12:2015 §8.8.12).
                    start_decode_time: b.start_decode_time.unwrap_or(0),
                    encryption: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Media {
            tracks,
            movie_timescale,
            pcr: Vec::new(),
            skipped,
            degraded,
            degraded_at: Vec::new(),
        })
    }
}

/// Fill what a `tfhd` leaves out from the track's `trex` (ISO/IEC 14496-12
/// §8.8.3, §8.8.7): a sample's duration, size and flags default to the `tfhd`
/// value, and when that is absent too, to the `trex` of the init movie.
///
/// A packager that declares its durations once in `trex` (GPAC does for a
/// fragment whose samples all last the same) writes `trun` entries with sizes
/// only; without the `trex` fallback every such sample came out with duration
/// 0 and the whole fragment at one `dts`. A `trex` size of zero means "not
/// declared" in practice and is not applied.
fn apply_trex_defaults(moof: &mut MovieFragmentBox, trex: &[crate::init_segment::TrackExtendsBox]) {
    for traf in &mut moof.traf {
        let Some(defaults) = trex.iter().find(|t| t.track_id == traf.tfhd.track_id) else {
            continue;
        };
        let tfhd = &mut traf.tfhd;
        tfhd.default_sample_duration
            .get_or_insert(defaults.default_sample_duration);
        tfhd.default_sample_flags
            .get_or_insert(defaults.default_sample_flags);
        if defaults.default_sample_size != 0 {
            tfhd.default_sample_size
                .get_or_insert(defaults.default_sample_size);
        }
    }
}

/// Resolve a fragment's samples into `builders`, slicing coded bytes from the
/// file per ISO/IEC 14496-12 §8.8.7 / §8.8.8.
///
/// The addressing (and its bounds) live in [`crate::frag_offsets`], shared with
/// [`crate::cenc_decrypt::CencDecryptor`] so the two cannot drift.
fn absorb_fragment(
    file: &[u8],
    mdats: &[crate::frag_offsets::MdatRange],
    moof_off: usize,
    moof: &MovieFragmentBox,
    builders: &mut [TrackBuilder],
) -> usize {
    // A `traf` whose `trun` addressing or timing is invalid is skipped whole
    // and counted (audit T16, #1144); each `traf` is committed atomically so a
    // failure leaves its track exactly as it was.
    let mut bad = 0usize;
    for traf in &moof.traf {
        if absorb_traf(file, mdats, moof_off, moof, traf, builders).is_err() {
            bad += 1;
        }
    }
    bad
}

fn absorb_traf(
    file: &[u8],
    mdats: &[crate::frag_offsets::MdatRange],
    moof_off: usize,
    moof: &MovieFragmentBox,
    traf: &crate::movie_fragment::TrackFragmentBox,
    builders: &mut [TrackBuilder],
) -> Result<()> {
    {
        let tfhd = &traf.tfhd;
        let Some(builder) = builders
            .iter_mut()
            .find(|b| b.spec.track_id == tfhd.track_id)
        else {
            // A track present in a fragment but absent from the init movie:
            // skip it (well-formed CMAF declares every track in the init moov).
            return Ok(());
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
        // Resolve and validate everything first; the builder is only touched
        // once the whole `traf` is known good.
        let mut start_decode_time = builder.start_decode_time;
        let mut next_dts = builder.next_dts;
        if let Some(tfdt) = &traf.tfdt {
            let base = tfdt.base_media_decode_time();
            if start_decode_time.is_none() {
                start_decode_time = Some(base);
            }
            next_dts = crate::frag_offsets::tfdt_as_i64(base)?;
        }
        let mut samples = Vec::new();
        for r in crate::frag_offsets::sample_ranges_in(file, mdats, moof_off, moof, tfhd.track_id)?
        {
            let is_sync = r.flags & SAMPLE_FLAG_IS_NON_SYNC == 0;
            let dts = next_dts;
            let pts = crate::frag_offsets::add_offset(dts, r.composition_offset)?;
            samples.push(Sample {
                data: file[r.start..r.end].to_vec().into(),
                dts: Some(dts),
                pts: Some(pts),
                duration: Some(r.duration),
                flags: crate::ir::SampleFlags::new(is_sync),
                // fMP4 sources carry no per-sample source-container
                // timestamps distinct from the fragment's own tfdt/trun
                // timing (see `Sample::provenance`).
                provenance: None,
                display: None,
            });
            next_dts = crate::frag_offsets::add_duration(dts, r.duration)?;
        }
        builder.start_decode_time = start_decode_time;
        builder.next_dts = next_dts;
        builder.samples.extend(samples);
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
// Shared output timeline (issues #1020-#1023)
// ---------------------------------------------------------------------------

/// The common decode-time origin of a set of tracks: the earliest first-sample
/// [`Sample::dts`] across them, compared across timescales exactly.
///
/// Every [`Sample::dts`] is absolute in its own track's timescale, so one
/// shift applied to every track keeps inter-track offsets and gaps intact —
/// which a per-track rebase (or a per-track `Σ duration` from zero) loses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimelineOrigin {
    ticks: i64,
    timescale: u32,
}

impl TimelineOrigin {
    /// The origin of `tracks`; `0` when no sample carries a `dts`.
    pub(crate) fn of<'t>(tracks: impl IntoIterator<Item = &'t Track>) -> Self {
        let mut best: Option<(i64, u32)> = None;
        for t in tracks {
            let Some(d) = t.samples.iter().find_map(|s| s.dts) else {
                continue;
            };
            let ts = t.spec.timescale.max(1);
            let earlier = match best {
                None => true,
                Some((bd, bts)) => (d as i128) * (bts as i128) < (bd as i128) * (ts as i128),
            };
            if earlier {
                best = Some((d, ts));
            }
        }
        let (ticks, timescale) = best.unwrap_or((0, 1));
        Self { ticks, timescale }
    }

    /// The origin expressed in `timescale` ticks, rounded down.
    pub(crate) fn in_timescale(&self, timescale: u32) -> i64 {
        let n = self.ticks as i128 * timescale.max(1) as i128;
        let v = n.div_euclid(self.timescale.max(1) as i128);
        v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }
}

/// Each sample's decode time relative to `origin`, in the track's own
/// timescale: the sample's own absolute `dts`, falling back to the previous
/// sample's time + duration only for a track that carries no `dts` at all
/// (section-carried data), which then starts at the origin.
pub(crate) fn relative_decode_times(track: &Track, origin: &TimelineOrigin) -> Vec<i64> {
    let base = origin.in_timescale(track.spec.timescale);
    let mut out = Vec::with_capacity(track.samples.len());
    let mut next: i64 = 0;
    for s in &track.samples {
        let t = match s.dts {
            Some(d) => d.saturating_sub(base),
            None => next,
        };
        out.push(t);
        next = t.saturating_add(i64::from(s.duration.unwrap_or(0)));
    }
    out
}

/// Presentation span of `tracks` in seconds: from the earliest sample
/// presentation time to the latest presentation end, over one common origin.
pub(crate) fn presentation_span_secs(tracks: &[&Track]) -> f64 {
    let origin = TimelineOrigin::of(tracks.iter().copied());
    let mut span: Option<(f64, f64)> = None;
    for t in tracks {
        let ts = f64::from(t.spec.timescale.max(1));
        for (s, dts) in t.samples.iter().zip(relative_decode_times(t, &origin)) {
            let pts = (dts + i64::from(s.composition_offset())) as f64 / ts;
            let end = pts + f64::from(s.duration.unwrap_or(0)) / ts;
            span = Some(match span {
                None => (pts, end),
                Some((lo, hi)) => (lo.min(pts), hi.max(end)),
            });
        }
    }
    span.map_or(0.0, |(lo, hi)| (hi - lo).max(0.0))
}

/// `secs` rounded up to whole seconds, without the std-only `f64::ceil`.
fn ceil_secs(secs: f64) -> u32 {
    let whole = secs as u32; // truncates toward zero; `secs` is non-negative
    if (whole as f64) < secs {
        whole.saturating_add(1)
    } else {
        whole
    }
}

// ---------------------------------------------------------------------------
// HlsPackager — Package<Output = String>
// ---------------------------------------------------------------------------

/// Render an RFC 8216 media playlist describing a [`Media`].
///
/// The whole `Media` is one multi-track CMAF segment (the [`CmafMux`] output),
/// so the playlist carries a single `#EXTINF` entry whose duration is the
/// presentation span across every timed track, pointing at
/// `{uri_prefix}{track_id}.m4s` named after the first timed track. Segments in
/// a media playlist play one after another (RFC 8216 §3), so one entry per
/// track would play the content once per track.
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

        let Some(first) = kept.first() else {
            return Err(Error::InvalidInput(
                "cannot package a Media whose every track is timestamp-less \
                 (section-carried): an HLS media playlist needs at least one \
                 segment with a real EXTINF duration",
            ));
        };
        // One segment for the one multi-track CMAF artifact: RFC 8216 §3 plays
        // a media playlist's segments one after another, so a per-track entry
        // would repeat the content once per track.
        let duration = presentation_span_secs(&kept);
        // EXTINF rounded to the nearest integer must not exceed the target
        // duration (RFC 8216 §4.3.3.1); the ceiling always satisfies that.
        let target_secs = ceil_secs(duration);
        let uri = format!("{}{}.m4s", self.uri_prefix, first.spec.track_id);
        let segments = vec![MediaSegment {
            map: Some(MapTag {
                uri: uri.clone(),
                byte_range: Some(ByteRange {
                    length: init_len,
                    offset: Some(0),
                }),
                extra_attrs: Vec::new(),
            }),
            uri,
            // `presentation_span_secs` is `(hi - lo).max(0.0)` or `0.0`
            // (issue #1140): always finite (a bounded division of finite
            // sample timestamps by a non-zero timescale) and non-negative
            // by construction.
            duration: DecimalSeconds::new(duration).expect("presentation span is finite, >= 0"),
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }];
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
        Ok(playlist.to_m3u8()?)
    }
}

// ---------------------------------------------------------------------------
// Helpers: box location + TrackSpec reconstruction
// ---------------------------------------------------------------------------

/// Find a top-level box by four-CC, returning its full bytes (header + body).
///
/// Re-exported for [`crate::progressive_demux::ProgressiveDemux`], which walks
/// the same top-level `moov` box; the one implementation is
/// [`crate::box_types::find_top_box`] (#1141).
pub(crate) use crate::box_types::find_top_box;

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
    // A `stsd` that failed to parse during the init-moov walk survives as raw
    // bytes (`StblChild::Opaque`, boxtype checked below) rather than an empty
    // placeholder (`init_segment::parse_stbl_children`) — re-parse it here so
    // the *real* error (e.g. a malformed `avcC` trailer) reaches
    // `skipped_track`'s `SkippedTrack::reason` instead of the uninformative
    // "no stsd entry" a blanked-out box would otherwise produce (#952).
    let stsd: SampleDescriptionBox = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(Ok(s.clone())),
            StblChild::Opaque(bytes) if bytes.len() >= 8 && &bytes[4..8] == b"stsd" => {
                Some(SampleDescriptionBox::parse(bytes.as_slice()))
            }
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stsd" })??;
    let entry = stsd.entries.first().ok_or(Error::UnexpectedBox {
        expected: "stsd entry",
    })?;

    let config = codec_config_from_entry(entry)?;
    let spec = TrackSpec::new(track_id, timescale, config);
    // Overlay container-level display geometry (pasp / colr nclx) from the
    // sample entry onto the SPS-derived display — ISO/IEC 14496-12:2015
    // §12.1.4 (pasp) / §12.1.5 (colr). AVC/HEVC entries only; see
    // `overlay_sample_entry_display`.
    let spec = match entry {
        SampleEntryVariant::Avc1(e) => overlay_sample_entry_display(spec, &e.extra_boxes),
        SampleEntryVariant::Hevc1(e) => overlay_sample_entry_display(spec, &e.extra_boxes),
        _ => spec,
    };
    Ok(spec)
}

/// Overlay container-level display geometry (a `pasp` / `colr nclx` box found
/// in an AVC/HEVC sample entry) onto an SPS-derived [`DisplayGeometry`].
///
/// ISO/IEC 14496-12:2015 §12.1.4 (`pasp`) / §12.1.5 (`colr`): `colr nclx`
/// overrides the bitstream colour description (§12.1.5.1); `pasp` fills in an
/// unspecified VUI sample aspect ratio. Which of two *specified* sample aspect
/// ratios wins is not defined by the text available here (the §12.1.4.1
/// definition is not transcribed), so on a disagreement the VUI SAR is kept.
/// Returns the input unchanged when neither box applies.
pub(crate) fn overlay_sample_entry_geometry(
    display: Option<DisplayGeometry>,
    config: &CodecConfig,
    extra_boxes: &[crate::sample_entries::OpaqueBox],
) -> Option<DisplayGeometry> {
    // First `pasp` in entry order.
    let pasp = extra_boxes
        .iter()
        .filter(|b| b.box_type == *b"pasp")
        .find_map(|b| PixelAspectRatioBox::parse(&b.data).ok());

    // First `colr` whose colour_type is 'nclx' (§12.1.5.1: listed from most to
    // least accurate, so the first nclx one wins). Its code points are u16 on
    // the wire but the model's enums are built from the 8-bit code points of
    // ITU-T H.273, so a box with a code point above 255 cannot be represented
    // and is ignored (the bitstream colour is kept).
    let colour = extra_boxes
        .iter()
        .filter(|b| b.box_type == *b"colr")
        .find_map(|b| {
            let c = ColourInformationBox::parse(&b.data).ok()?;
            if c.colour_type != *b"nclx" {
                return None;
            }
            let n = c.nclx?;
            let colour = ColourDescription::from_wire(
                u8::try_from(n.colour_primaries).ok()?,
                u8::try_from(n.transfer_characteristics).ok()?,
                u8::try_from(n.matrix_coefficients).ok()?,
            );
            Some((colour, n.full_range_flag))
        });

    // The pasp spacings reduced by their gcd to a u16 pair (both must be
    // positive, §12.1.4); a ratio of 1 is the Square idc.
    let pasp_ratio = pasp.and_then(|p| {
        if p.h_spacing == 0 || p.v_spacing == 0 {
            return None;
        }
        let g = gcd_u32(p.h_spacing, p.v_spacing);
        let (rw, rh) = (
            u16::try_from(p.h_spacing / g).ok()?,
            u16::try_from(p.v_spacing / g).ok()?,
        );
        Some(if rw == rh {
            AspectRatioIdc::from_wire(1, None)
        } else {
            AspectRatioIdc::from_wire(255, Some((rw, rh)))
        })
    });

    if pasp_ratio.is_none() && colour.is_none() {
        return display;
    }

    // The display the SPS yielded, or (when it yielded none) one built from
    // the sample entry's own visual size.
    let (visual_w, visual_h) = match config {
        CodecConfig::Avc { width, height, .. } | CodecConfig::Hevc { width, height, .. } => {
            (u32::from(*width), u32::from(*height))
        }
        _ => (0, 0),
    };
    let signalled_format = display
        .as_ref()
        .and_then(|g| g.video_signal.map(|v| v.video_format));
    let mut display = display.unwrap_or_else(|| DisplayGeometry::new(visual_w, visual_h, None));

    // pasp only fills in; two specified ratios that disagree keep the VUI one.
    if let Some(ratio) = pasp_ratio
        && display.sample_aspect_ratio().is_none()
    {
        display.aspect_ratio = Some(ratio);
    }

    // colr nclx overrides the bitstream colour; the VUI's video_format is kept
    // when one was signalled (colr carries none), else Unspecified.
    if let Some((colour, full_range)) = colour {
        display.video_signal = Some(VideoSignalType::new(
            signalled_format.unwrap_or(VideoFormat::Unspecified),
            full_range,
            Some(colour),
        ));
    }

    Some(display)
}

/// Overlay container-level display geometry from a `pasp` / `colr nclx` box
/// found in an AVC/HEVC sample entry onto the SPS-derived
/// [`TrackSpec::display`].
fn overlay_sample_entry_display(
    mut spec: TrackSpec,
    extra_boxes: &[crate::sample_entries::OpaqueBox],
) -> TrackSpec {
    let config = spec.config.clone();
    spec.display = overlay_sample_entry_geometry(spec.display, &config, extra_boxes);
    if let Some(hdr) = crate::hdr::HdrStaticMetadata::from_sample_entry_boxes(
        extra_boxes.iter().map(|b| (&b.box_type, b.data.as_slice())),
    ) {
        spec.hdr = Some(hdr);
    }
    spec
}

/// Whether a `trak`'s sample entry carries parameter sets in-band
/// (avc3/avc4/hev1), plus its entry-level extra boxes for the display overlay.
fn track_entry_inband_and_extras(trak: &TrackBox) -> (bool, Vec<crate::sample_entries::OpaqueBox>) {
    let Some(mdia) = trak.mdia.as_ref() else {
        return (false, Vec::new());
    };
    let Some(minf) = mdia.minf.as_ref() else {
        return (false, Vec::new());
    };
    let Some(stbl) = minf.stbl.as_ref() else {
        return (false, Vec::new());
    };
    let stsd: SampleDescriptionBox = match stbl.children.iter().find_map(|c| match c {
        StblChild::Stsd(s) => Some(Ok(s.clone())),
        StblChild::Opaque(bytes) if bytes.len() >= 8 && &bytes[4..8] == b"stsd" => {
            Some(SampleDescriptionBox::parse(bytes.as_slice()))
        }
        _ => None,
    }) {
        Some(Ok(s)) => s,
        _ => return (false, Vec::new()),
    };
    let Some(entry) = stsd.entries.first() else {
        return (false, Vec::new());
    };
    match entry {
        SampleEntryVariant::Avc1(e) => (
            e.codec_type == *b"avc3" || e.codec_type == *b"avc4",
            e.extra_boxes.clone(),
        ),
        SampleEntryVariant::Hevc1(e) => (e.codec_type == *b"hev1", e.extra_boxes.clone()),
        _ => (false, Vec::new()),
    }
}

/// Tag an in-band (avc3/avc4/hev1) track's samples with per-AU display state,
/// mirroring the TS demux path (`SpsTracker`).
fn tag_fmp4_inband_display(b: &mut TrackBuilder) {
    let config = b.spec.config.clone();
    let codec = match &config {
        CodecConfig::Avc { .. } => TrackCodec::Avc,
        CodecConfig::Hevc { .. } => TrackCodec::Hevc,
        _ => return,
    };
    let mut tracker = SpsTracker::new(codec);
    // Seed from the track's raw SPS geometry (before the container overlay).
    let raw_seed = DisplayGeometry::from_codec_config(&config);
    tracker.seed_state(raw_seed);

    let extra = b.entry_extra_boxes.clone();
    let mut cached_raw: Option<Arc<DisplayInfo>> = None;
    let mut cached_overlaid: Option<Arc<DisplayInfo>> = None;

    for sample in b.samples.iter_mut() {
        // Convert length-prefixed -> Annex B for observation only; sample bytes
        // stay unchanged. A malformed length prefix must not panic and leaves
        // the previous tag in place.
        let length_size = config_nal_length_size(&config).unwrap_or(crate::annexb::NAL_LENGTH_SIZE);
        if let Ok(nals) = crate::annexb::iter_length_prefixed_nals_with(&sample.data, length_size) {
            let mut annexb = Vec::with_capacity(sample.data.len());
            for nal in nals {
                annexb.extend_from_slice(&[0, 0, 0, 1]);
                annexb.extend_from_slice(nal);
            }
            let _ = tracker.observe(&annexb);
        }
        let raw_state = tracker.state().cloned();
        let display_arc = match raw_state {
            None => None,
            Some(raw) => {
                let overlaid =
                    if let (Some(prev_raw), Some(prev_ov)) = (&cached_raw, &cached_overlaid) {
                        if Arc::ptr_eq(prev_raw, &raw) {
                            prev_ov.clone()
                        } else {
                            let ov = overlay_display_info(&raw, &config, &extra);
                            cached_raw = Some(raw.clone());
                            cached_overlaid = Some(ov.clone());
                            ov
                        }
                    } else {
                        let ov = overlay_display_info(&raw, &config, &extra);
                        cached_raw = Some(raw.clone());
                        cached_overlaid = Some(ov.clone());
                        ov
                    };
                Some(overlaid)
            }
        };
        sample.display = display_arc;
    }
}

fn overlay_display_info(
    raw: &Arc<DisplayInfo>,
    config: &CodecConfig,
    extra: &[crate::sample_entries::OpaqueBox],
) -> Arc<DisplayInfo> {
    let mut info = (**raw).clone();
    info.geometry = overlay_sample_entry_geometry(info.geometry, config, extra);
    Arc::new(info)
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
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
pub(crate) fn skipped_track(err: Error, track_id: crate::TrackId) -> SkippedTrack {
    let fourcc = match &err {
        Error::UnsupportedSampleEntry { fourcc } => fourcc.clone(),
        _ => String::from("unknown"),
    };
    SkippedTrack {
        fourcc,
        reason: err.to_string(),
        track_id: Some(track_id),
    }
}

/// The real sampling rate of an audio sample entry.
///
/// Prefers a `SamplingRateBox` (`srat`, ISO/IEC 14496-12 §12.2.3.1) in the
/// entry's config children over the entry's 16.16 fixed-point `samplerate`
/// field: the box is present precisely when the rate does not fit that field
/// (rates above 65535 Hz — 96 kHz, 192 kHz, …), and it "overrides the
/// samplerate field and documents the actual sampling rate". Without it,
/// `192000 << 16` reads back as 60928 Hz.
fn entry_sampling_rate(samplerate_16_16: u32, config_boxes: &[OpaqueBox]) -> u32 {
    sampling_rate_override(config_boxes).unwrap_or(samplerate_16_16 >> 16)
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
            // visual dims when the SPS is absent, cannot be decoded, or
            // reports a coded size that does not fit the IR's `u16` field (the
            // SPS dimensions are `ue(v)`, so an oversized one must not truncate).
            let (width, height) = vvc
                .config
                .config
                .dimensions()
                .and_then(|(w, h)| Some((u16::try_from(w).ok()?, u16::try_from(h).ok()?)))
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
                    sample_rate: entry_sampling_rate(mp4a.samplerate, &mp4a.config_boxes),
                    sample_size: mp4a.samplesize,
                })
            } else {
                Ok(CodecConfig::Aac {
                    esds,
                    channel_count: mp4a.channelcount,
                    sample_rate: entry_sampling_rate(mp4a.samplerate, &mp4a.config_boxes),
                    sample_size: mp4a.samplesize,
                })
            }
        }
        SampleEntryVariant::Ac3(ac3) => {
            let config = Ac3SpecificBox::parse(config_box_body(&ac3.config_boxes, b"dac3")?)?;
            Ok(CodecConfig::Ac3 {
                config,
                channel_count: ac3.channelcount,
                sample_rate: entry_sampling_rate(ac3.samplerate, &ac3.config_boxes),
                sample_size: ac3.samplesize,
            })
        }
        SampleEntryVariant::Ec3(ec3) => {
            let config = Ec3SpecificBox::parse(config_box_body(&ec3.config_boxes, b"dec3")?)?;
            Ok(CodecConfig::Eac3 {
                config,
                channel_count: ec3.channelcount,
                sample_rate: entry_sampling_rate(ec3.samplerate, &ec3.config_boxes),
                sample_size: ec3.samplesize,
            })
        }
        SampleEntryVariant::Opus(opus) => {
            let config = OpusSpecificBox::parse(config_box_body(&opus.config_boxes, b"dOps")?)?;
            Ok(CodecConfig::Opus {
                config,
                channel_count: opus.channelcount,
                sample_rate: entry_sampling_rate(opus.samplerate, &opus.config_boxes),
                sample_size: opus.samplesize,
            })
        }
        SampleEntryVariant::Flac(flac) => {
            let config = FlacSpecificBox::parse(config_box_body(&flac.config_boxes, b"dfLa")?)?;
            Ok(CodecConfig::Flac {
                config,
                channel_count: flac.channelcount,
                sample_rate: entry_sampling_rate(flac.samplerate, &flac.config_boxes),
                sample_size: flac.samplesize,
            })
        }
        SampleEntryVariant::Dts(dts) => {
            let config = DtsSpecificBox::parse(config_box_body(&dts.config_boxes, b"ddts")?)?;
            Ok(CodecConfig::Dts {
                config,
                codec_fourcc: dts.codec_type,
                channel_count: dts.channelcount,
                sample_rate: entry_sampling_rate(dts.samplerate, &dts.config_boxes),
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
                sample_rate: entry_sampling_rate(mha.samplerate, &mha.config_boxes),
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
                sample_rate: entry_sampling_rate(ac4.samplerate, &ac4.config_boxes),
                sample_size: ac4.samplesize,
            })
        }
        // Subtitle sample entries (media plane step 2d): the whole entry (TTML
        // namespace / WebVTT `vttC` header block / trailing boxes) is carried
        // in `CodecConfig::Subtitle::sample_entry` so it re-muxes (#1186).
        // Samples stay opaque (TTML XML / WebVTT `vttc`/`vtte` boxes), never parsed.
        SampleEntryVariant::Stpp(s) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::Ttml,
            sample_entry: Some(SubtitleSampleEntry::Ttml((**s).clone())),
        }),
        SampleEntryVariant::Wvtt(w) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::WebVtt,
            sample_entry: Some(SubtitleSampleEntry::WebVtt((**w).clone())),
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
        CodecConfig::Mpeg2Video { width, height, .. }
            if let Ok(sh) = Mpeg2SeqHeader::find(&first.data) =>
        {
            *width = sh.width;
            *height = sh.height;
        }
        CodecConfig::MpegAudio { layer, .. }
            if let Ok(hdr) = MpegAudioFrameHeader::parse(&first.data) =>
        {
            *layer = hdr.layer;
        }
        _ => {}
    }
}

/// The NAL length-prefix size (bytes) an AVC/HEVC config declares, or `None`
/// for a codec that is not length-prefixed. ISO/IEC 14496-15 §5.3.3:
/// `lengthSizeMinusOne + 1`.
fn config_nal_length_size(config: &CodecConfig) -> Option<usize> {
    match config {
        CodecConfig::Avc { config, .. } => {
            Some(usize::from(config.config.length_size_minus_one) + 1)
        }
        CodecConfig::Hevc { config, .. } => {
            Some(usize::from(config.config.length_size_minus_one) + 1)
        }
        _ => None,
    }
}

/// Rewrite a track's samples to the crate's canonical 4-byte NAL prefixes and
/// set the config's `lengthSizeMinusOne` to match, when the source declared a
/// different size (ISO/IEC 14496-15 §5.3.3: 1, 2 or 4 bytes).
///
/// The rest of the pipeline — [`crate::annexb::iter_length_prefixed_nals`],
/// keyframe detection, the TS/RTP writers — is fixed at 4 bytes, so a 1- or
/// 2-byte-length fMP4 would otherwise parse every sample as garbage (r04-W43).
/// A `length_size_minus_one` of 2 (3-byte lengths) is reserved by §5.3.3, so
/// such a track is rejected rather than guessed at.
pub(crate) fn normalise_track_nal_lengths(
    spec: &mut TrackSpec,
    samples: &mut [Sample],
) -> Result<()> {
    let Some(length_size) = config_nal_length_size(&spec.config) else {
        return Ok(());
    };
    if length_size == crate::annexb::NAL_LENGTH_SIZE {
        // Already canonical. Samples stay opaque here (this demuxer never
        // walks coded data for the common 4-byte case), so a container whose
        // sample bytes are not NAL-framed still demuxes as before.
        return Ok(());
    }
    for sample in samples.iter_mut() {
        let data = crate::annexb::normalise_nal_length_size(&sample.data, length_size)?;
        sample.data = data.into();
    }
    match &mut spec.config {
        CodecConfig::Avc { config, .. } => {
            config.config.length_size_minus_one = crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        }
        CodecConfig::Hevc { config, .. } => {
            config.config.length_size_minus_one = crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        }
        _ => {}
    }
    Ok(())
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
    use crate::TrackId;
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
                    format: SubtitleFormat::WebVtt,
                    sample_entry: Some(SubtitleSampleEntry::WebVtt(ref e)),
                } if e.config.config == "WEBVTT\n"
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
            TrackId::new(1),
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
        let seg1 = build_media_segment(
            1,
            &[FragmentTrackData::new(TrackId::new(1), first_base, &f1)],
        )
        .expect("fragment 1 builds");
        let seg2 = build_media_segment(
            2,
            &[FragmentTrackData::new(
                TrackId::new(1),
                gapped_second_base,
                &f2,
            )],
        )
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
            TrackId::new(7),
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
            &build_media_segment(1, &[FragmentTrackData::new(TrackId::new(7), 0, &f1)]).unwrap(),
        );
        file.extend_from_slice(
            &build_media_segment(
                2,
                &[FragmentTrackData::new(TrackId::new(7), second_base, &f2)],
            )
            .unwrap(),
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
                TrackId::new(1),
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
                TrackId::new(2),
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
                TrackId::new(2),
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

    /// r04-W43: an fMP4 whose `avcC` declares 2-byte NAL lengths must have its
    /// samples normalised to the crate's 4-byte form, and the config the
    /// demuxer reports must be updated to match — the rest of the pipeline
    /// (`iter_length_prefixed_nals`, the TS/RTP writers) is fixed at 4 bytes.
    ///
    /// The file is a genuine one: an init segment built from a config that
    /// really declares `lengthSizeMinusOne = 1`, concatenated with a media
    /// segment whose samples really carry 2-byte prefixes.
    #[test]
    fn fmp4_demux_normalises_two_byte_nal_lengths() {
        use crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        const TIMESCALE: u32 = 90_000;
        use crate::ir::{FragmentTrackData, Sample, SampleFlags};
        use crate::pipeline::{CodecConfig, TrackSpec, build_init_segment, build_media_segment};

        // The same Baseline SPS/PPS pair, but declaring 2-byte NAL lengths.
        let mut avcc = minimal_avc_config();
        avcc.config.length_size_minus_one = 1;
        let spec = TrackSpec::new(
            TrackId::new(1),
            TIMESCALE,
            CodecConfig::Avc {
                config: avcc,
                width: 640,
                height: 360,
            },
        );
        // One access unit, two NALs, each with a 2-byte length prefix.
        let sample = Sample {
            data: vec![0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x01, 0x41].into(),
            dts: Some(0),
            pts: Some(0),
            duration: Some(1000),
            flags: SampleFlags::new(true),
            provenance: None,
            display: None,
        };
        let init = build_init_segment(std::slice::from_ref(&spec), TIMESCALE).expect("init");
        // The init segment really declares the 2-byte size.
        let hdr = init
            .windows(4)
            .position(|w| w == b"avcC")
            .expect("avcC present");
        // avcC body: configurationVersion(1) profile(1) compat(1) level(1)
        // reserved(6)+lengthSizeMinusOne(2).
        assert_eq!(
            init[hdr + 8] & 0x03,
            1,
            "the built init segment must declare lengthSizeMinusOne = 1"
        );
        let frag = build_media_segment(
            1,
            &[FragmentTrackData::new(
                TrackId::new(1),
                0,
                std::slice::from_ref(&sample),
            )],
        )
        .expect("media segment");
        let mut file = init;
        file.extend_from_slice(&frag);

        let media = Fmp4Demux::new()
            .unpackage(&file)
            .expect("demux the 2-byte fMP4");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples[0].data.as_ref(),
            [
                0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x00, 0x00, 0x01, 0x41,
            ],
            "2-byte NAL lengths must be rewritten to 4-byte"
        );
        let CodecConfig::Avc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Avc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the reported avcC must declare the 4-byte length size"
        );
    }
}
