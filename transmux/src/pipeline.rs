//! TS → CMAF/fMP4 remux pipeline — the muxer above the box builders.
//!
//! This is a **samples-in** API (transcode/demux stay in the caller): feed
//! already-demuxed *encoded* access units plus their per-track [`CodecConfig`],
//! and get back a CMAF **initialization segment** (`ftyp` + fragmented-init
//! `moov`) and **media segments** (`styp` + `moof` + `mdat`), per
//! ISO/IEC 14496-12:2015 §8.8 (movie fragments) and the CMAF structure.
//!
//! Video access units are carried length-prefixed in `mdat`
//! ([`crate::annexb`]); the `moof` `trun` carries per-sample size / duration /
//! flags / composition offset, `tfdt` the base media decode time, and each
//! `trun.data_offset` is computed from the finished `moof` size so the samples
//! resolve against the `default-base-is-moof` base.

mod stsd;

use crate::TrackId;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::error::InvalidValue;

use broadcast_common::Serialize;

use crate::ac4::DAC4_FOURCC;
use crate::av1::Av1SampleEntry;
use crate::dts::DDTS_FOURCC;
use crate::error::{Error, Result};
use crate::flac::DFLA_FOURCC;
use crate::init_segment::{
    Ac3SampleEntry, Ac4SampleEntry, ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox,
    DataReferenceBox, DtsSampleEntry, Ec3SampleEntry, FlacSampleEntry, HandlerBox, MediaBox,
    MediaHeaderBox, MediaInformationBox, MhaSampleEntry, MovieBox, MovieExtendsBox, MovieHeaderBox,
    Mp4aSampleEntry, OpaqueBox, OpusSampleEntry, SampleDescriptionBox, SampleEntryVariant,
    SampleSizeBox, SampleTableBox, SampleToChunkBox, SoundMediaHeaderBox, StblChild, TrackBox,
    TrackExtendsBox, TrackHeaderBox, VideoMediaHeaderBox,
};
use crate::movie_fragment::{
    MediaRunSample, MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, media_fragment_run,
};
use crate::mpegh::MHAC_FOURCC;
use crate::opus::DOPS_FOURCC;
use crate::sample_entries::{
    AVCSampleEntry, HEVCSampleEntry, Mp4vSampleEntry, VVCSampleEntry, VisualSampleEntryFields,
};
use crate::segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};
use crate::visual_ext::{ColourInformationBox, NclxColourInfo, PixelAspectRatioBox};

use crate::timing::TimeToSampleBox;
use crate::vp9::Vp9SampleEntry;
pub use mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime};

/// ISO-639-2 packed language code `und` (undetermined).
const LANG_UND: u16 = 0x55C4;
/// The ISOBMFF identity transformation matrix (§6.2.2), unity + `[2,30]` fixed.
const IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
/// `tkhd` flags: track_enabled | in_movie | in_preview.
const TKHD_ENABLED_IN_MOVIE: u32 = 0x0000_0007;

/// [`CodecConfig`], [`DataCarriage`], [`TrackSpec`], [`Sample`],
/// [`SampleFlags`], [`Provenance`], [`FragmentTrackData`] moved to
/// [`crate::ir`] (media plane step 2a) — re-exported here so every existing
/// `crate::pipeline::`/`transmux::pipeline::` path keeps resolving unchanged.
pub use crate::ir::{
    CodecConfig, DataCarriage, FragmentTrackData, Provenance, Sample, SampleFlags, SubtitleFormat,
    SubtitleSampleEntry, TrackSpec,
};

/// Build a CMAF initialization segment (`ftyp` + fragmented-init `moov`) for the
/// given tracks. The `moov` carries empty sample tables (`stts`/`stsc`/`stsz`/
/// `stco`) plus an `mvex`/`trex` per track, marking the movie as fragmented.
pub fn build_init_segment(tracks: &[TrackSpec], movie_timescale: u32) -> Result<Vec<u8>> {
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: 512,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };

    let next_track_id = tracks
        .iter()
        .map(|t| t.track_id)
        .max()
        .map_or(TrackId::new(1), TrackId::next);
    let mvhd = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: movie_timescale,
        duration: 0,
        rate: 0x0001_0000,
        volume: 0x0100,
        matrix: IDENTITY_MATRIX,
        next_track_id,
    };

    let mut trak_boxes = Vec::with_capacity(tracks.len());
    let mut trex = Vec::with_capacity(tracks.len());
    for t in tracks {
        // MUX = strict but filterable (media plane step-2 fix wave 1,
        // B1-B4): a track this crate cannot place into an ISOBMFF `trak`
        // (opaque `CodecConfig::Data` — issue #557/#576 — or
        // `CodecConfig::Subtitle` with no sample entry) is rejected, naming it,
        // rather than silently omitted — every mux entry point built on this
        // function (`CmafMux`, `ProgressiveMux`, `Segmenter`,
        // `LlSegmenter`, `LlHlsSegmenter`) shares this one check, so a
        // Media containing e.g. a DVB subtitle/DSM-CC/SCTE-35 track fails
        // the same way everywhere; the caller must pre-filter first (e.g.
        // with `Media::select_tracks_by`).
        if !t.config.is_muxable_in_bmff() {
            return Err(unmuxable_track_error(t));
        }
        trak_boxes.push(build_trak(t)?);
        trex.push(TrackExtendsBox {
            version: 0,
            flags: 0,
            track_id: t.track_id,
            default_sample_description_index: 1,
            default_sample_duration: 0,
            default_sample_size: 0,
            default_sample_flags: 0,
        });
    }

    let moov = MovieBox::new(
        mvhd,
        trak_boxes,
        Some(MovieExtendsBox::new(trex, vec![])),
        vec![],
    );

    let mut out = vec![0u8; ftyp.serialized_len() + moov.serialized_len()];
    let n1 = ftyp.serialize_into(&mut out)?;
    let n2 = moov.serialize_into(&mut out[n1..])?;
    out.truncate(n1 + n2);
    Ok(out)
}

/// Build the named, typed rejection for a track [`CodecConfig::is_muxable_in_bmff`]
/// says `build_init_segment` cannot carry — [`Error::UnmuxableDataTrack`] for
/// the opaque PES carriage, [`Error::UnmuxableSubtitleTrack`] for a subtitle
/// track (B1, media plane step-2 fix wave 1).
fn unmuxable_track_error(t: &TrackSpec) -> crate::error::Error {
    match &t.config {
        CodecConfig::Data { stream_type, .. } => crate::error::Error::UnmuxableDataTrack {
            track_id: t.track_id,
            stream_type: *stream_type,
        },
        CodecConfig::Subtitle { format, .. } => crate::error::Error::UnmuxableSubtitleTrack {
            track_id: t.track_id,
            format: *format,
        },
        _ => unreachable!("is_muxable_in_bmff() is only false for Data/Subtitle"),
    }
}

/// Serialize a typed config box body and wrap it as an [`OpaqueBox`] under the
/// given four-CC (mirrors the `esds`/`dac3` embedding for audio sample entries).
fn config_box<C: Serialize<Error = crate::error::Error>>(
    fourcc: &[u8; 4],
    config: &C,
) -> Result<OpaqueBox> {
    let mut body = vec![0u8; config.serialized_len()];
    let n = config.serialize_into(&mut body)?;
    body.truncate(n);
    Ok(OpaqueBox::new(*fourcc, body))
}

/// The sampling-rate carriage decision for one audio track's `stsd` entry.
///
/// An `AudioSampleEntry`'s `samplerate` is 16.16 fixed point, so its integer
/// part is limited to 65535 Hz. A rate above that (96, 176.4, 192 kHz, …)
/// cannot be represented — `192000 << 16` is truncated to `0xEE000000` and
/// reads back as 60928 Hz. ISO/IEC 14496-12 §12.2.3.2 (as amended by Amd 1:2017)
/// requires the `AudioSampleEntryV1` form for those rates: `entry_version = 1`,
/// `samplerate = 1 << 16`, and a [`SamplingRateBox`] (`srat`, §12.2.3.1)
/// carrying the real rate. Such an entry must sit in an `stsd` whose version
/// is 1.
///
/// [`SamplingRateBox`]: crate::init_segment::SamplingRateBox
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioRateCarriage {
    /// `entry_version` to write into the sample entry (0 or 1).
    entry_version: u16,
    /// Value for the entry's 16.16 `samplerate` field.
    samplerate_field: u32,
    /// The `srat` box to append as a config child, when the v1 form is used.
    srat: Option<crate::init_segment::SamplingRateBox>,
}

/// Decide the `stsd` sampling-rate carriage for `sample_rate`
/// (ISO/IEC 14496-12 §12.2.3 / §12.2.3.1 / §12.2.3.2).
///
/// A rate that fits the 16.16 field's integer part is written the plain way;
/// a rate above `u16::MAX` needs the v1 entry, its `srat` box and an `stsd`
/// version of 1.
fn audio_rate_carriage(sample_rate: u32) -> AudioRateCarriage {
    use crate::init_segment::{AudioSampleEntryV1, SamplingRateBox};
    if AudioSampleEntryV1::rate_fits_v0(sample_rate) {
        AudioRateCarriage {
            entry_version: 0,
            samplerate_field: sample_rate << 16,
            srat: None,
        }
    } else {
        AudioRateCarriage {
            entry_version: AudioSampleEntryV1::ENTRY_VERSION,
            samplerate_field: AudioSampleEntryV1::SAMPLERATE_PLACEHOLDER,
            srat: Some(SamplingRateBox::new(sample_rate)),
        }
    }
}

/// Serialize a [`SamplingRateBox`] into the opaque form the sample entries'
/// `config_boxes` carry (body only, 8-byte box header stripped), matching how
/// the typed config records above are embedded.
///
/// [`SamplingRateBox`]: crate::init_segment::SamplingRateBox
fn srat_opaque(rate: crate::init_segment::SamplingRateBox) -> Result<OpaqueBox> {
    let mut full = vec![0u8; rate.serialized_len()];
    let n = rate.serialize_into(&mut full)?;
    Ok(OpaqueBox::new(
        crate::init_segment::SamplingRateBox::FOURCC,
        full[8..n].to_vec(),
    ))
}

/// Build the `pasp`/`colr` child boxes (ISO/IEC 14496-12:2015 §12.1.4–5)
/// that `build_trak` appends to an AVC/HEVC sample entry from the track's
/// [`DisplayGeometry`] (Phase 4a-1). `pasp` is emitted only for a non-1:1
/// sample aspect ratio; `colr` (`nclx`) only when a colour description was
/// signalled. The track's HDR10 static metadata adds `mdcv` then `clli`
/// (`private/specs-verified/mpeg/hdr/isobmff-mdcv-clli-boxes.md`). Order:
/// `pasp`, `colr`, `mdcv`, `clli`, all after the codec config box.
fn avc_hevc_display_extra_boxes(
    display: Option<&crate::ir::DisplayGeometry>,
    hdr: Option<&crate::hdr::HdrStaticMetadata>,
    dovi: Option<&crate::dovi::DoviDecoderConfigurationRecord>,
) -> Result<Vec<crate::sample_entries::OpaqueBox>> {
    let mut extra_boxes = Vec::new();
    if let Some(display) = display {
        push_display_boxes(display, &mut extra_boxes)?;
    }
    if let Some(hdr) = hdr {
        for (box_type, data) in hdr.sample_entry_boxes()? {
            extra_boxes.push(crate::sample_entries::OpaqueBox { box_type, data });
        }
    }
    if let Some(dovi) = dovi {
        extra_boxes.push(crate::sample_entries::OpaqueBox {
            box_type: dovi.box_fourcc(),
            data: dovi.try_to_bytes()?,
        });
    }
    Ok(extra_boxes)
}

fn push_display_boxes(
    display: &crate::ir::DisplayGeometry,
    extra_boxes: &mut Vec<crate::sample_entries::OpaqueBox>,
) -> Result<()> {
    // pasp — only when the SAR is specified and not square (§12.1.4.1: an
    // absent pasp means square pixels).
    if let Some((w, h)) = display.sample_aspect_ratio()
        && w != h
    {
        let pasp = PixelAspectRatioBox {
            h_spacing: u32::from(w),
            v_spacing: u32::from(h),
        };
        let mut body = alloc::vec![0u8; pasp.serialized_len()];
        pasp.serialize_into(&mut body)?;
        extra_boxes.push(crate::sample_entries::OpaqueBox {
            box_type: *b"pasp",
            data: body,
        });
    }

    // colr (nclx) — only when a colour description was actually signalled.
    if let Some(vs) = display.video_signal
        && let Some(colour) = vs.colour
    {
        let nclx = NclxColourInfo {
            colour_primaries: u16::from(colour.primaries.code()),
            transfer_characteristics: u16::from(colour.transfer.code()),
            matrix_coefficients: u16::from(colour.matrix.code()),
            full_range_flag: vs.full_range,
            reserved: 0,
        };
        let colr = ColourInformationBox {
            colour_type: *b"nclx",
            nclx: Some(nclx),
            icc_profile: alloc::vec::Vec::new(),
        };
        let mut body = alloc::vec![0u8; colr.serialized_len()];
        colr.serialize_into(&mut body)?;
        extra_boxes.push(crate::sample_entries::OpaqueBox {
            box_type: *b"colr",
            data: body,
        });
    }

    Ok(())
}

/// Whole-pixel presentation size for an AVC/HEVC video track (square-pixel
/// presentation size per display-geometry Q8): cropped output size, then
/// rescaled for the sample aspect ratio (horizontal only), rounded to the
/// nearest whole pixel. `display = None` (or an unspecified SAR) falls back to
/// the coded size. Shared core with `tkhd_presentation_size` so the two
/// cannot drift.
pub fn presentation_size(
    display: Option<&crate::ir::DisplayGeometry>,
    coded_width: u16,
    coded_height: u16,
) -> (u32, u32) {
    let (w, h) = presentation_size_fixed(display, coded_width, coded_height);
    (
        u32::try_from((w + (1 << 15)) >> 16).unwrap_or(u32::MAX),
        u32::try_from((h + (1 << 15)) >> 16).unwrap_or(u32::MAX),
    )
}

/// Basis on which a nominal frame rate was derived (issue: HEVC VUI timing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameRateBasis {
    /// Normative: derived from a fixed-rate signalling field
    /// (`fixed_frame_rate_flag` for H.264, `fixed_pic_rate_within_cvs_flag`
    /// for H.265).
    Normative,
    /// Convention (implementation-defined): no fixed-rate signalling; the
    /// value is `time_scale / num_units_in_tick` (H.264) or
    /// `vui_time_scale / vui_num_units_in_tick` (H.265). Owner-approved
    /// 2026-10-21.
    Convention,
}

impl FrameRateBasis {
    pub fn name(&self) -> &'static str {
        match self {
            FrameRateBasis::Normative => "normative",
            FrameRateBasis::Convention => "convention",
        }
    }
}

broadcast_common::impl_spec_display!(FrameRateBasis);

/// Exact integer frame rate in millihertz from VUI timing fields (H.264):
/// `round_half_up(1000 * time_scale / (2 * num_units_in_tick))`.
fn frame_rate_millihertz_from_timing(time_scale: u32, num_units_in_tick: u32) -> Option<u32> {
    if num_units_in_tick == 0 {
        return None;
    }
    let denominator = u128::from(num_units_in_tick).saturating_mul(2);
    if denominator == 0 {
        return None;
    }
    let numerator = u128::from(time_scale).saturating_mul(1000);
    let millihertz = numerator
        .checked_add(u128::from(num_units_in_tick))?
        .checked_div(denominator)?;
    u32::try_from(millihertz).ok()
}

/// Exact integer frame rate in millihertz from VUI timing fields (H.265
/// convention tier): `round_half_up(1000 * time_scale / num_units_in_tick)`.
fn frame_rate_millihertz_hevc_convention(time_scale: u32, num_units_in_tick: u32) -> Option<u32> {
    if num_units_in_tick == 0 {
        return None;
    }
    let denominator = u128::from(num_units_in_tick);
    let numerator = u128::from(time_scale).saturating_mul(1000);
    // round half up: add denominator/2
    let millihertz = numerator
        .checked_add(denominator / 2)?
        .checked_div(denominator)?;
    u32::try_from(millihertz).ok()
}

/// Normative tier (H.265 §E.3.2, `private/specs-verified/mpeg/transmux/codec/vui-timing-frame-rate.md` §2.7): the
/// picture interval is `ClockTick * elemental_duration`, `ClockTick = num_units / time_scale`,
/// so the picture rate is `time_scale / (num_units * elemental_duration)` Hz and
/// `millihertz = round_half_up(1000 * time_scale / (num_units * elemental_duration))`.
/// With `field_seq` the pictures are fields; the frame rate is half the picture rate
/// (assuming complementary field pairs), i.e. the denominator doubles.
fn frame_rate_millihertz_hevc_normative(
    time_scale: u32,
    num_units_in_tick: u32,
    elemental_duration: u32, // elemental_duration_in_tc_minus1 + 1
    field_seq: bool,
) -> Option<u32> {
    if num_units_in_tick == 0 || elemental_duration == 0 {
        return None;
    }
    // denominator ticks = num_units * elemental_duration, times 2 if field_seq
    let ticks = u128::from(num_units_in_tick).checked_mul(u128::from(elemental_duration))?;
    let ticks = if field_seq {
        ticks.checked_mul(2)?
    } else {
        ticks
    };
    if ticks == 0 {
        return None;
    }
    let numerator = u128::from(time_scale).saturating_mul(1000);
    // round half up
    let millihertz = numerator.checked_add(ticks / 2)?.checked_div(ticks)?;
    u32::try_from(millihertz).ok()
}

/// H.265 frame-rate derivation from the parsed VUI timing facts, per
/// `private/specs-verified/mpeg/transmux/codec/vui-timing-frame-rate.md` §2.7.
///
/// * Tier A (normative): HRD fixed picture rate present. `field_seq_flag = 1` halves the
///   picture rate (assumes complementary field pairs); `frame_field_info_present_flag = 1`
///   with `field_seq_flag = 0` leaves the cadence to the per-picture `pic_struct` SEI, so no
///   value.
/// * Tier B (convention, implementation-defined): no HRD fixed rate, `field_seq_flag = 0`
///   and `frame_field_info_present_flag = 0`: `vui_time_scale / vui_num_units_in_tick`.
/// * Otherwise `None`.
fn hevc_frame_rate_from_timing(
    timing: &crate::sps::HevcVuiTiming,
) -> Option<(u32, FrameRateBasis)> {
    let (num_units, time_scale) = (timing.num_units_in_tick, timing.time_scale);
    if num_units == 0 {
        return None;
    }
    match timing.hrd_fixed_pic_rate {
        Some(ed) => {
            if !timing.field_seq && timing.frame_field_info_present {
                return None;
            }
            let value =
                frame_rate_millihertz_hevc_normative(time_scale, num_units, ed, timing.field_seq)?;
            Some((value, FrameRateBasis::Normative))
        }
        None => {
            if timing.field_seq || timing.frame_field_info_present {
                return None;
            }
            let value = frame_rate_millihertz_hevc_convention(time_scale, num_units)?;
            Some((value, FrameRateBasis::Convention))
        }
    }
}

/// Combine an SPS VUI timing (if any) with a VPS timing fallback, per
/// `private/specs-verified/mpeg/transmux/codec/hevc-vps-timing.md` §3-§4. The SPS timing wins when
/// present (the spec requires the two to be equal when both are present; the
/// VPS is then ignored). Otherwise the VPS timing is used together with the
/// SPS interlace facts (`field_seq` / `frame_field_info_present` from
/// `decode_hevc_interlace_sps`, which already applies the §E.3.1 inferences),
/// with `hrd_fixed_pic_rate = None` (VPS HRD is not used, so the result is at
/// most tier B). Returns `None` when neither carries timing, or when the VPS
/// timing is unusable (either value 0).
///
/// Applying the inheritance when the SPS has no VUI at all is an inference of
/// this derivation (doc §4), since §3 phrases the inference in terms of "when
/// not present" inside the VUI.
fn hevc_timing_with_vps_fallback(
    sps: Option<crate::sps::HevcVuiTiming>,
    interlace: crate::sps::HevcInterlaceSps,
    vps: Option<crate::sps::HevcVpsTiming>,
) -> Option<crate::sps::HevcVuiTiming> {
    if let Some(t) = sps {
        return Some(t);
    }
    let v = vps?;
    if v.num_units_in_tick == 0 || v.time_scale == 0 {
        return None;
    }
    Some(crate::sps::HevcVuiTiming {
        num_units_in_tick: v.num_units_in_tick,
        time_scale: v.time_scale,
        field_seq: interlace.field_seq,
        frame_field_info_present: interlace.frame_field_info_present,
        hrd_fixed_pic_rate: None,
    })
}

/// Nominal frame rate of a video track, in millihertz (1/1000 Hz), plus the
/// basis (normative vs. convention) on which it was derived.
///
/// H.264 (ITU-T H.264 §E.2.1): when VUI `timing_info_present_flag` is 1,
/// `round_half_up(1000 * time_scale / (2 * num_units_in_tick))`. Tier
/// "normative" when `fixed_frame_rate_flag == 1`, "convention" otherwise
/// (owner-approved: both emit the same value).
///
/// H.265 (ITU-T H.265 §E.2.1 + hrd_parameters): see
/// `private/specs-verified/mpeg/transmux/codec/vui-timing-frame-rate.md` §2.7.
pub fn nominal_frame_rate(config: &CodecConfig) -> Option<(u32, FrameRateBasis)> {
    match config {
        CodecConfig::Avc { config, .. } => {
            let sps = config.config.sps.first()?;
            let timing = crate::sps::decode_avc_vui_timing(&sps.0).ok()??;
            let num_units = timing.num_units_in_tick;
            let time_scale = timing.time_scale;
            if num_units == 0 {
                return None;
            }
            let value = frame_rate_millihertz_from_timing(time_scale, num_units)?;
            let basis = if timing.fixed_frame_rate {
                FrameRateBasis::Normative
            } else {
                FrameRateBasis::Convention
            };
            Some((value, basis))
        }
        CodecConfig::Hevc { config, .. } => {
            let sps_nal = config
                .config
                .arrays
                .iter()
                .find(|a| a.nal_unit_type == 33)?
                .nalus
                .first()?
                .0
                .as_slice();
            let sps_timing = crate::sps::decode_hevc_vui_timing(sps_nal).ok().flatten();
            let interlace = crate::sps::decode_hevc_interlace_sps(sps_nal)
                .ok()
                .unwrap_or_default();
            // VPS fallback: only when the SPS carries no VUI timing. A missing
            // VPS array means no fallback (None as before).
            let vps_timing = config
                .config
                .arrays
                .iter()
                .find(|a| a.nal_unit_type == 32)
                .and_then(|a| a.nalus.first())
                .and_then(|n| {
                    crate::sps::decode_hevc_vps_timing(n.0.as_slice())
                        .ok()
                        .flatten()
                });
            let timing = hevc_timing_with_vps_fallback(sps_timing, interlace, vps_timing)?;
            hevc_frame_rate_from_timing(&timing)
        }
        _ => None,
    }
}

/// Nominal frame rate of a video track, in millihertz (1/1000 Hz), derived
/// solely from the SPS VUI timing information.
///
/// Thin wrapper around [`nominal_frame_rate`] that discards the basis; kept
/// for `hls-runtime` and other callers. Returns `None` whenever
/// [`nominal_frame_rate`] returns `None`.
pub fn nominal_frame_rate_millihertz(config: &CodecConfig) -> Option<u32> {
    nominal_frame_rate(config).map(|(v, _)| v)
}

/// 16.16 fixed-point presentation size (the value written into `tkhd`).
fn presentation_size_fixed(
    display: Option<&crate::ir::DisplayGeometry>,
    coded_width: u16,
    coded_height: u16,
) -> (u64, u64) {
    let Some(display) = display else {
        return (
            (u64::from(coded_width)) << 16,
            (u64::from(coded_height)) << 16,
        );
    };
    let out = display.output_rect;
    let height = (u64::from(out.height)) << 16;
    let width = match display.typed_sample_aspect_ratio() {
        Some(sar) => {
            let sar_w = u64::from(sar.width());
            let sar_h = u64::from(sar.height());
            if sar_h == 0 {
                (u64::from(out.width)) << 16
            } else {
                let num = u64::from(out.width) * sar_w * 65536;
                (num + sar_h / 2)
                    .checked_div(sar_h)
                    .unwrap_or((u64::from(out.width)) << 16)
            }
        }
        None => (u64::from(out.width)) << 16,
    };
    (width, height)
}

/// tkhd presentation size for an AVC/HEVC video track per ISO/IEC 14496-12:2012
/// §8.3.2 and ETSI TS 103 285 §5.1.2: cropped output size, then rescaled for
/// the sample aspect ratio (horizontal only), as 16.16 fixed point.
fn tkhd_presentation_size(
    display: Option<&crate::ir::DisplayGeometry>,
    coded_width: u16,
    coded_height: u16,
) -> (u32, u32) {
    let (width, height) = presentation_size_fixed(display, coded_width, coded_height);
    (
        u32::try_from(width).unwrap_or(u32::MAX),
        u32::try_from(height).unwrap_or(u32::MAX),
    )
}

/// Build one fragmented-init `trak` (empty sample tables + a single `stsd` entry).
fn build_trak(t: &TrackSpec) -> Result<TrackBox> {
    let audio = t.config.is_audio();
    let stsd::TrackMedia {
        entry: stsd_entry,
        kind,
        tkhd_w,
        tkhd_h,
    } = stsd::track_media(t)?;
    let (vmhd, smhd) = (kind.vmhd(), kind.smhd());
    let minf_header = kind.minf_header();
    let (handler_type, handler_name) = (kind.handler_type(), kind.handler_name());

    let stbl = SampleTableBox {
        children: vec![
            StblChild::Stsd(SampleDescriptionBox {
                version: stsd_entry.required_stsd_version(),
                flags: 0,
                entries: vec![stsd_entry],
            }),
            StblChild::Stts(TimeToSampleBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsc(SampleToChunkBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsz(SampleSizeBox {
                version: 0,
                flags: 0,
                sample_size: 0,
                sample_count: 0,
                entries: vec![],
            }),
            StblChild::Stco(ChunkOffsetBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
        ],
    };

    let dinf = DataInformationBox::new(
        Some(DataReferenceBox {
            version: 0,
            flags: 0,
            // A single self-contained URL entry (flags bit 0 = media in this file).
            entries: vec![DataEntry::Url(DataEntryUrlBox {
                version: 0,
                flags: 1,
                location: vec![],
            })],
        }),
        vec![],
    );

    let mut minf = MediaInformationBox::new(vmhd, smhd, Some(dinf), Some(stbl), vec![]);
    if let Some(h) = minf_header {
        minf.set_leading_opaque_header(h);
    }

    let mdhd = MediaHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: t.timescale,
        duration: 0,
        language: LANG_UND,
    };

    let hdlr = HandlerBox {
        version: 0,
        flags: 0,
        handler_type,
        name: handler_name,
    };

    let mdia = MediaBox::new(Some(mdhd), Some(hdlr), Some(minf), vec![]);

    let tkhd = TrackHeaderBox {
        version: 0,
        flags: TKHD_ENABLED_IN_MOVIE,
        creation_time: 0,
        modification_time: 0,
        track_id: t.track_id,
        duration: 0,
        layer: 0,
        alternate_group: 0,
        volume: if audio { 0x0100 } else { 0 },
        matrix: IDENTITY_MATRIX,
        width: tkhd_w,
        height: tkhd_h,
        // Anamorphic tkhd presentation size (ISO/IEC 14496-12 §8.3.2) is a follow-up; coded size retained.
    };

    Ok(TrackBox::new(tkhd, None, Some(mdia), vec![]))
}

/// Build one CMAF media segment (`styp` + `moof` + `mdat`) carrying the given
/// per-track samples. Each `trun.data_offset` is computed from the finished
/// `moof` size (the tracks' sample data are concatenated into a single `mdat`
/// in track order, resolved against the `default-base-is-moof` base).
///
/// Delegates to [`build_media_segment_with_events`] with an empty `emsgs`
/// slice (identical output).
pub fn build_media_segment(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
) -> Result<Vec<u8>> {
    build_media_segment_with_events(sequence_number, tracks, &[])
}

/// Build one CMAF media segment (`styp` + one or more `emsg` boxes + `moof` +
/// `mdat`), optionally carrying inband timed-metadata events.
///
/// # Box order
///
/// Per **ISO/IEC 14496-12 §8.8** (movie fragments) and the DASH-IF Interoperability
/// Point Part 10 §6.1 (Events and Timed Metadata in MPEG-DASH / CMAF), each
/// [`EmsgBox`] must appear **at the start of the segment**, after the `styp`
/// segment-type box but *before* the `moof`.  The boxes in `emsgs` are emitted
/// in the order given.  A common consumer is SCTE 35 in-band ad-insertion
/// signalling (`scheme_id_uri = "urn:scte:scte35:2013:bin"`,
/// ANSI/SCTE 214-3 §8.3.3 —
/// DASH-IF Part 10 §7.3).
///
/// # Timing
///
/// [`PresentationTime::Delta`] (version 0) expresses the event time relative to
/// the segment's earliest presentation time in `timescale` ticks.
/// [`PresentationTime::Absolute`] (version 1) expresses an absolute
/// representation-timeline position, per ISO/IEC 23009-1 §5.10.3.3.
///
/// # Empty slice
///
/// When `emsgs` is empty this function produces byte-identical output to
/// [`build_media_segment`].
pub fn build_media_segment_with_events(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
    emsgs: &[EmsgBox<'_>],
) -> Result<Vec<u8>> {
    build_chunk(sequence_number, tracks, true, emsgs)
}

/// `styp` major brand of a CMAF segment or low-latency chunk (ISO/IEC 14496-12
/// §8.16.4); one brand set so chunk concatenation equals the whole segment.
const STYP_MAJOR_BRAND: [u8; 4] = *b"msdh";
const STYP_COMPATIBLE_BRANDS: [[u8; 4]; 2] = [*b"msdh", *b"msix"];

/// Build one CMAF fragment (`[styp] [emsg]* moof mdat`) over a run of samples.
///
/// The single builder behind [`build_media_segment_with_events`] and the LL-DASH
/// and LL-HLS chunk/part builders: same `trun` flags and `default-base-is-moof`
/// offset resolution everywhere, so a segment's chunks concatenate to the
/// whole-segment coded-sample stream. `with_styp` gates the leading `styp`
/// (first chunk of a segment only); `emsgs` follow it and precede the `moof`
/// (ISO/IEC 14496-12 §8.8, DASH-IF IOP Part 10 §6.1).
pub(crate) fn build_chunk(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
    with_styp: bool,
    emsgs: &[EmsgBox<'_>],
) -> Result<Vec<u8>> {
    let styp = with_styp.then(|| SegmentTypeBox {
        major_brand: STYP_MAJOR_BRAND,
        minor_version: 0,
        compatible_brands: STYP_COMPATIBLE_BRANDS.to_vec(),
    });

    let mut traf_boxes = Vec::with_capacity(tracks.len());
    for ft in tracks {
        let (tfhd, trun) = media_fragment_run(
            ft.track_id,
            ft.samples.iter().map(|s| MediaRunSample {
                duration: s.duration.unwrap_or(0),
                size: s.data.len(),
                is_sync: s.flags.is_sync,
                composition_offset: s.composition_offset(),
            }),
        )?;
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(ft.base_media_decode_time);
        traf_boxes.push(TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]));
    }

    let mut moof = MovieFragmentBox::new(MovieFragmentHeaderBox::new(sequence_number), traf_boxes);

    let moof_size = moof.serialized_len();
    let mut cursor = moof_size + crate::box_types::BOX_HEADER_MIN_SIZE;
    let mut mdat_data = Vec::new();
    for (i, ft) in tracks.iter().enumerate() {
        moof.traf[i].trun[0].data_offset = Some(i32::try_from(cursor).map_err(|_| {
            Error::InvalidValue(InvalidValue {
                field: "trun data_offset",
                value: cursor as u64,
                reason: "fragment too large for the signed 32-bit trun data_offset",
            })
        })?);
        for s in ft.samples {
            mdat_data.extend_from_slice(&s.data);
            cursor += s.data.len();
        }
    }
    let mdat = MediaDataBox { data: mdat_data };

    let emsg_vecs: Vec<Vec<u8>> = emsgs
        .iter()
        .map(|e| Ok(e.to_vec()?))
        .collect::<Result<_>>()?;
    let emsg_total: usize = emsg_vecs.iter().map(|v| v.len()).sum();

    let styp_len = styp.as_ref().map_or(0, |s| s.serialized_len());
    let total = styp_len + emsg_total + moof.serialized_len() + mdat.serialized_len();
    let mut out = vec![0u8; total];
    let mut c = 0usize;
    if let Some(styp) = &styp {
        c += styp.serialize_into(&mut out[c..])?;
    }
    for ev in &emsg_vecs {
        out[c..c + ev.len()].copy_from_slice(ev);
        c += ev.len();
    }
    c += moof.serialize_into(&mut out[c..])?;
    c += mdat.serialize_into(&mut out[c..])?;
    out.truncate(c);
    Ok(out)
}

#[cfg(test)]
mod frame_rate_tests {
    use super::frame_rate_millihertz_from_timing;

    #[test]
    fn table() {
        assert_eq!(frame_rate_millihertz_from_timing(60000, 1001), Some(29970));
        assert_eq!(frame_rate_millihertz_from_timing(120000, 1001), Some(59940));
        assert_eq!(frame_rate_millihertz_from_timing(50, 1), Some(25000));
        assert_eq!(frame_rate_millihertz_from_timing(1, 0), None);
        // round half up
        assert_eq!(frame_rate_millihertz_from_timing(1, 3), Some(167));
    }
}

#[cfg(test)]
mod hevc_rate_rules_tests {
    use super::{FrameRateBasis, hevc_frame_rate_from_timing, hevc_timing_with_vps_fallback};
    use crate::sps::{HevcInterlaceSps, HevcVpsTiming, HevcVuiTiming};

    fn t(
        num: u32,
        scale: u32,
        field_seq: bool,
        ffip: bool,
        ed: Option<u32>,
    ) -> Option<(u32, FrameRateBasis)> {
        hevc_frame_rate_from_timing(&HevcVuiTiming {
            num_units_in_tick: num,
            time_scale: scale,
            field_seq,
            frame_field_info_present: ffip,
            hrd_fixed_pic_rate: ed,
        })
    }

    const N: FrameRateBasis = FrameRateBasis::Normative;
    const C: FrameRateBasis = FrameRateBasis::Convention;

    #[test]
    fn tier_a_uses_the_elemental_duration() {
        assert_eq!(t(1, 25, false, false, Some(1)), Some((25_000, N)));
        // elemental_duration_in_tc_minus1 = 1 => interval doubles => half the rate
        assert_eq!(t(1, 25, false, false, Some(2)), Some((12_500, N)));
        // 1001/60000 tick, 2 ticks per picture => 29.970
        assert_eq!(t(1001, 60_000, false, false, Some(2)), Some((29_970, N)));
    }

    #[test]
    fn tier_a_field_sequence_halves_the_picture_rate() {
        assert_eq!(t(1, 50, true, true, Some(1)), Some((25_000, N)));
        assert_eq!(t(1, 50, true, true, Some(2)), Some((12_500, N)));
    }

    #[test]
    fn frame_field_info_without_field_seq_is_not_derivable() {
        // pic_struct (picture timing SEI) decides the cadence: no value, with or without HRD.
        assert_eq!(t(1, 25, false, true, Some(1)), None);
        assert_eq!(t(1, 25, false, true, None), None);
    }

    #[test]
    fn tier_b_is_the_clock_tick_convention_only_for_plain_frames() {
        assert_eq!(t(1, 25, false, false, None), Some((25_000, C)));
        assert_eq!(t(1001, 30_000, false, false, None), Some((29_970, C)));
        // field sequence without HRD fixed rate: not derivable
        assert_eq!(t(1, 25, true, true, None), None);
        assert_eq!(t(1, 25, true, false, None), None);
    }

    #[test]
    fn degenerate_inputs_give_none() {
        assert_eq!(t(0, 25, false, false, None), None);
        assert_eq!(t(0, 25, false, false, Some(1)), None);
        assert_eq!(t(1, 25, false, false, Some(0)), None);
        // 1000 * u32::MAX millihertz does not fit u32: overflow => None.
        assert_eq!(t(1, u32::MAX, false, false, None), None);
    }

    fn vps(num: u32, scale: u32) -> Option<HevcVpsTiming> {
        Some(HevcVpsTiming {
            num_units_in_tick: num,
            time_scale: scale,
        })
    }

    #[test]
    fn vps_fallback_sps_wins() {
        let sps = Some(HevcVuiTiming {
            num_units_in_tick: 1,
            time_scale: 25,
            field_seq: false,
            frame_field_info_present: false,
            hrd_fixed_pic_rate: None,
        });
        let inter = HevcInterlaceSps::default();
        let vps = vps(1001, 30000);
        let t = hevc_timing_with_vps_fallback(sps, inter, vps).unwrap();
        let rate = hevc_frame_rate_from_timing(&t).unwrap();
        assert_eq!(rate, (25_000, FrameRateBasis::Convention));
    }

    #[test]
    fn vps_fallback_vps_only_convention() {
        let inter = HevcInterlaceSps::default();
        let vps = vps(1001, 30000);
        let t = hevc_timing_with_vps_fallback(None, inter, vps).unwrap();
        let rate = hevc_frame_rate_from_timing(&t).unwrap();
        assert_eq!(rate, (29_970, FrameRateBasis::Convention));
    }

    #[test]
    fn vps_fallback_none_none() {
        let inter = HevcInterlaceSps::default();
        assert!(hevc_timing_with_vps_fallback(None, inter, None).is_none());
    }

    #[test]
    fn vps_fallback_field_seq_none_rate() {
        let inter = HevcInterlaceSps {
            field_seq: true,
            frame_field_info_present: false,
        };
        let vps = vps(1001, 30000);
        let t = hevc_timing_with_vps_fallback(None, inter, vps).unwrap();
        assert!(hevc_frame_rate_from_timing(&t).is_none());
    }

    #[test]
    fn vps_fallback_frame_field_info_none_rate() {
        let inter = HevcInterlaceSps {
            field_seq: false,
            frame_field_info_present: true,
        };
        let vps = vps(1001, 30000);
        let t = hevc_timing_with_vps_fallback(None, inter, vps).unwrap();
        assert!(hevc_frame_rate_from_timing(&t).is_none());
    }

    #[test]
    fn vps_fallback_zero_units_none() {
        let inter = HevcInterlaceSps::default();
        let v0 = Some(HevcVpsTiming {
            num_units_in_tick: 0,
            time_scale: 30000,
        });
        assert!(hevc_timing_with_vps_fallback(None, inter, v0).is_none());
        let v1 = Some(HevcVpsTiming {
            num_units_in_tick: 1001,
            time_scale: 0,
        });
        assert!(hevc_timing_with_vps_fallback(None, inter, v1).is_none());
    }
}
