//! `check_container_codec` — ISOBMFF/CMAF codec-level checks (issue #567).
//!
//! Companion to the TS-input codec checks in `diagnostics::{codec_signalling,
//! interlace, param_sets, fps_cadence}`, which operate on the
//! [`crate::Diagnostic`] trait's raw TS byte-stream. A fragmented or
//! progressive MP4 file has no PMT to cross-validate; instead the
//! sample-entry config box (`avcC`/`hvcC`) and its `width`/`height` fields
//! *are* the container's claim, and the SPS embedded in that very config box
//! is the bitstream truth. [`check_container_codec`] demuxes a whole image
//! through [`transmux::mp4_demux::Mp4Demux`] (or [`ContainerCodecChecker`]
//! takes the same stage's events one at a time, so a file on disk can be
//! checked in bounded memory) and, for every AVC/HEVC video track,
//! cross-checks:
//!
//! - **`avcc-sps-mismatch`** / **`hvcc-sps-mismatch`** (Error): the config
//!   record's `profile_indication`/`level_indication` (AVC) or
//!   `general_profile_idc`/`general_level_idc` (HEVC) disagrees with the
//!   same fields re-decoded from the record's own embedded SPS (a level
//!   at or above the SPS level is conformant)
//!   ([`transmux::decode_avc_sps`]/[`transmux::decode_hevc_sps`] — reused,
//!   never re-implemented) — ISO/IEC 14496-15:2017 §5.3.3.1/§8.3.3.1.
//! - **`container-sps-dimension-mismatch`** (Error): the sample entry's
//!   `width`/`height` disagrees with the SPS-decoded coded dimensions
//!   (ITU-T H.264 §7.4.2.1.1 / H.265 §7.4.3.2.1 cropped geometry).
//! - **`avc-interlaced-content`** (Info): the AVC SPS has
//!   `frame_mbs_only_flag == 0` — mirrors the TS-side `InterlaceCheck`
//!   (`diagnostics::interlace`) verbatim, including its `Info`-not-mismatch
//!   severity: neither MPEG-2 TS PMT signalling nor ISOBMFF (there is no
//!   `tkhd`/`stsd` progressive/interlace flag) carries a container-level
//!   claim to cross-validate against, so this surfaces the bitstream fact
//!   plainly rather than as a mismatch. HEVC is out of scope for the same
//!   reason as the TS-side check: `HevcSpsInfo` carries no comparable field.
//! - **`length-prefix-violation`** (Error): a sample's coded bytes do not
//!   parse as a well-formed 4-byte length-prefixed NAL sequence
//!   ([`broadcast_bitstream::iter_length_prefixed_nals`]) — the structural signature of
//!   an Annex B start code smuggled into an MP4 sample in place of a length
//!   prefix (ISO/IEC 14496-15:2017 §4.3.2 "no start codes", §4.3.3).
//!
//! - **`hvcc-array-nal-type`** (Warning): an `hvcC` array whose `NAL_unit_type` is
//!   not VPS/SPS/PPS/prefix SEI/suffix SEI. The 2017 text restricts the arrays to
//!   those types but tells readers to ignore the others, so this is a warning
//!   (ISO/IEC 14496-15:2017 §8.3.3.1.1).
//! - **`hvc1-array-completeness`** (Warning): an `hvc1` entry whose
//!   `array_completeness` is not 1 on a parameter-set array and 0 on every other
//!   array (§8.4.1.1.1 "default and mandatory"). A warning, not an error: GPAC
//!   writes `hvc1` with 0 on the parameter sets and the file plays everywhere.
//!   `hev1` only defaults to 0, so it is never reported.
//!
//! Non-ISOBMFF input, or input that fails to demux (e.g. a bare `sidx`-only
//! stub with no `moov`), yields no findings — this function has nothing to
//! say about a container it cannot parse, which must never surface as a
//! false positive.
//!
//! Follows the same free-function `&mut Report` out-parameter convention as
//! [`crate::check_playlist`]; [`Location::packet`] holds the sample index (0
//! for track-level findings) and [`Location::stream`] holds the MP4 track_id
//! as [`StreamLocator::Track`] (never a TS PID — there is none for this
//! input).

use alloc::collections::BTreeMap;

use broadcast_bitstream::iter_length_prefixed_nals_with;
use broadcast_common::{Stage, Timestamp};
use broadcast_core::Parse;
use isobmff::init_segment::{MovieBox, SampleEntryVariant, StblChild};
#[cfg(all(test, feature = "std"))]
use test_harness::collect::{Collected as Media, CollectedTrack as Track};
use transmux::TrackId;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::{CodecConfig, DemuxEvent, Sample, TrackSpec};

use crate::report::{Finding, Location, Report, Severity};
use transmux::nalu_ext::{AvcSpsCodecInfo, HevcNalUnitCodecInfo};

/// H.265 `NAL_unit_type` for a sequence parameter set (ITU-T H.265 §7.3.1.2
/// Table 7-1) — used to pick the SPS array out of `hvcC`'s `numOfArrays`
/// loop (ISO/IEC 14496-15:2017 §8.3.3).
const HEVC_NAL_SPS: u8 = 33;

/// Run every ISOBMFF/CMAF codec-level check against a whole `bytes` image,
/// appending findings to `report`.
pub fn check_container_codec(bytes: &[u8], report: &mut Report) {
    if !looks_like_isobmff(bytes) {
        return;
    }
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let config = Mp4DemuxConfig::default().with_total_len(Some(total_len));
    let mut checker = ContainerCodecChecker::new();
    let _ = drive_mp4(bytes, config, &mut checker, report);
    if let Some(moov) = isobmff::box_types::find_box(bytes, &isobmff::init_segment::MOOV) {
        check_hevc_sample_entries(moov, report);
    }
}

/// The incremental half of [`check_container_codec`]: feed it a demux stage's
/// events one at a time (a streamed file reads a bounded range at a time), and
/// the `moov` box once, and it appends the same findings the whole-image
/// entry point does.
pub struct ContainerCodecChecker {
    specs: BTreeMap<TrackId, TrackSpec>,
    sample_index: BTreeMap<TrackId, usize>,
    framing_reported: BTreeMap<TrackId, bool>,
}

impl ContainerCodecChecker {
    /// A checker that has seen no events.
    #[must_use]
    pub fn new() -> Self {
        Self {
            specs: BTreeMap::new(),
            sample_index: BTreeMap::new(),
            framing_reported: BTreeMap::new(),
        }
    }

    /// Apply one demux event. Track additions (and updates) run the
    /// track-level checks; a sample runs the length-prefix check once per
    /// track, exactly as the whole-image path does.
    pub fn event(&mut self, event: &DemuxEvent, report: &mut Report) {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                check_track_spec(spec, report);
                self.specs.insert(spec.track_id, spec.clone());
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                let index = self.sample_index.entry(*track_id).or_insert(0);
                let this = *index;
                *index = index.saturating_add(1);
                if self
                    .framing_reported
                    .get(track_id)
                    .copied()
                    .unwrap_or(false)
                {
                    return;
                }
                let Some(spec) = self.specs.get(track_id) else {
                    return;
                };
                if check_sample_framing(spec, this, sample, report) {
                    self.framing_reported.insert(*track_id, true);
                }
            }
            _ => {}
        }
    }

    /// The `hvc1`/`hev1` sample-entry warnings, from the `moov` box.
    pub fn sample_entries(&self, moov: &[u8], report: &mut Report) {
        check_hevc_sample_entries(moov, report);
    }
}

impl Default for ContainerCodecChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Drive a seekable [`Mp4Demux`] over an in-memory image, handing every event
/// to `checker`. Reads no more than the range the stage asks for.
fn drive_mp4(
    bytes: &[u8],
    config: Mp4DemuxConfig,
    checker: &mut ContainerCodecChecker,
    report: &mut Report,
) -> Result<(), transmux::Error> {
    let mut demux = Mp4Demux::seekable(config);
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]);
        demux.feed(
            Mp4Input::At {
                offset,
                bytes: slice.to_vec().into(),
            },
            Timestamp::ZERO,
        )?;
        while let Some(event) = demux.poll() {
            checker.event(&event, report);
        }
    }
    demux.finish()?;
    while let Some(event) = demux.poll() {
        checker.event(&event, report);
    }
    Ok(())
}

fn check_hevc_sample_entries(moov: &[u8], report: &mut Report) {
    let Ok(movie) = MovieBox::parse(moov) else {
        return;
    };
    for track in &movie.tracks {
        let track_id = track.tkhd.track_id;
        let Some(stbl) = track
            .mdia
            .as_ref()
            .and_then(|m| m.minf.as_ref())
            .and_then(|m| m.stbl.as_ref())
        else {
            continue;
        };
        for child in &stbl.children {
            let StblChild::Stsd(stsd) = child else {
                continue;
            };
            for entry in &stsd.entries {
                let SampleEntryVariant::Hevc1(entry) = entry else {
                    continue;
                };
                let name = alloc::string::String::from_utf8_lossy(&entry.codec_type).into_owned();
                for array in entry.config.config.unpermitted_arrays() {
                    report.push(Finding::new(
                        Severity::Warning,
                        Location::at_track(0, track_id.into()),
                        "hvcc-array-nal-type",
                        alloc::format!(
                            "{name} hvcC on track {track_id} carries an array of NAL_unit_type {}, \
                             which is not VPS, SPS, PPS, prefix SEI or suffix SEI; readers ignore \
                             it — ISO/IEC 14496-15:2017 §8.3.3.1.1",
                            array.nal_unit_type,
                        ),
                    ));
                }
                for array in entry.array_completeness_violations() {
                    report.push(Finding::new(
                        Severity::Warning,
                        Location::at_track(0, track_id.into()),
                        "hvc1-array-completeness",
                        alloc::format!(
                            "hvc1 hvcC on track {track_id} has array_completeness {} on the \
                             NAL_unit_type {} array; hvc1 requires {} — ISO/IEC 14496-15:2017 \
                             §8.4.1.1.1",
                            u8::from(array.array_completeness),
                            array.nal_unit_type,
                            u8::from(array.is_parameter_set()),
                        ),
                    ));
                }
            }
        }
    }
}

/// A minimal ISOBMFF sniff: the first box is a `size(4)+type(4)` header
/// naming a top-level box type every ISOBMFF/CMAF/DASH file starts with,
/// including the compressed `!mov` / `!mof` (ISO/IEC 14496-12:2026 §8.16).
pub fn looks_like_isobmff(bytes: &[u8]) -> bool {
    if bytes.len() < isobmff::box_types::BOX_HEADER_MIN_SIZE {
        return false;
    }
    let four_cc = &bytes[4..8];
    four_cc == isobmff::segments::FTYP.as_slice()
        || four_cc == isobmff::segments::STYP.as_slice()
        || four_cc == isobmff::init_segment::MOOV.as_slice()
        || four_cc == isobmff::segments::MOOF.as_slice()
        || four_cc == b"!mov"
        || four_cc == b"!mof"
        || four_cc == isobmff::segments::FREE.as_slice()
        || four_cc == isobmff::segments::SKIP.as_slice()
}

/// The NAL length-prefix width a track's codec configuration declares, in
/// bytes — `lengthSizeMinusOne + 1` (ISO/IEC 14496-15:2017 §5.3.3.1.3 `avcC`,
/// §8.3.3.1.3 `hvcC`). `None` for a track that is not AVC/HEVC.
///
/// `Sample::data` is length-prefixed NAL data only for those two codecs, so
/// this is also what gates the length-prefix check.
fn nal_length_size(config: &CodecConfig) -> Option<usize> {
    let length_size_minus_one = match config {
        CodecConfig::Avc { config, .. } => config.config.length_size_minus_one,
        CodecConfig::Hevc { config, .. } => config.config.length_size_minus_one,
        _ => return None,
    };
    // The field is 2 bits; the crate's parsers already reject the reserved
    // value 2 (a 3-byte prefix), so this cannot be out of range.
    let width = usize::from(length_size_minus_one) + 1;
    AVCC_LENGTH_SIZES.contains(&width).then_some(width)
}

/// `lengthSizeMinusOne` values a well-formed avcC/hvcC may declare, as byte
/// prefixes (ISO/IEC 14496-15:2017 §5.3.3.1.1).
const AVCC_LENGTH_SIZES: [usize; 3] = [1, 2, 4];

#[cfg(all(test, feature = "std"))]
fn check_media(media: &Media, report: &mut Report) {
    for track in &media.tracks {
        check_track_spec(&track.spec, report);
        for (idx, sample) in track.samples.iter().enumerate() {
            if check_sample_framing(&track.spec, idx, sample, report) {
                break;
            }
        }
    }
}

/// Run the track-level codec checks for one track.
fn check_track_spec(spec: &TrackSpec, report: &mut Report) {
    // `track_ID` is a full 32-bit field (ISO/IEC 14496-12:2026 §8.3.2) — never
    // narrowed, or every id above 65535 aliases onto a lower one.
    let track_id = spec.track_id;
    match &spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => check_avc_track(track_id, &config.config, *width, *height, report),
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => check_hevc_track(track_id, &config.config, *width, *height, report),
        _ => {}
    }
}

/// Run the per-sample length-prefix check for one sample. Returns `true` when
/// a violation was reported, so the caller can stop after one finding per
/// track.
fn check_sample_framing(
    spec: &TrackSpec,
    idx: usize,
    sample: &Sample,
    report: &mut Report,
) -> bool {
    // Length-prefixed NAL framing (ISO/IEC 14496-15:2017 §4.3.2, §4.3.3) only
    // applies to AVC/HEVC video samples — `transmux::pipeline::Sample`
    // documents its `data` field as "length-prefixed NAL data for
    // AVC/HEVC, or the raw frame for AAC [and every other codec]".
    // Running this check on a non-NAL track (e.g. a raw AAC frame, which
    // is not length-prefixed at all) is not a structural fault — it
    // would misread arbitrary audio bytes as a bogus NAL length and
    // false-positive on every clean AAC track (confirmed against
    // `fixtures/mp4/h264-aac-frag.mp4`'s AAC track).
    if !matches!(
        spec.config,
        CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
    ) {
        return false;
    }
    // The record's own `lengthSizeMinusOne` (ISO/IEC 14496-15:2017 §5.3.3.1.3
    // for `avcC`, §8.3.3.1.3 for `hvcC`) declares the prefix width —
    // 1-, 2- and 4-byte prefixes are all conformant. Assuming 4 bytes
    // reported a false `length-prefix-violation` on every conformant
    // AVC/HEVC track that uses a narrower prefix.
    let Some(length_size) = nal_length_size(&spec.config) else {
        return false;
    };
    if iter_length_prefixed_nals_with(&sample.data, length_size).is_err() {
        let track_id = spec.track_id;
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(idx, track_id),
            "length-prefix-violation",
            alloc::format!(
                "Sample {idx} on track {track_id} is not a well-formed {length_size}-byte \
                 length-prefixed NAL sequence (declared length runs past the sample's \
                 own end) — a hallmark of an Annex B start code left in place of a \
                 length prefix (ISO/IEC 14496-15:2017 §4.3.2)",
            ),
        ));
        return true;
    }
    false
}

fn check_avc_track(
    track_id: TrackId,
    record: &isobmff::AVCDecoderConfigurationRecord,
    width: u16,
    height: u16,
    report: &mut Report,
) {
    let Some(sps) = record.sps.first() else {
        return;
    };
    let Ok(info) = sps.decode() else {
        return;
    };

    if record.profile_indication != info.profile_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC AVCProfileIndication 0x{:02X} on track {track_id} disagrees with the \
                 embedded SPS profile_idc 0x{:02X} — ISO/IEC 14496-15:2017 §5.3.3.1.1",
                record.profile_indication,
                info.profile_idc,
            ),
        ));
    }
    if record.level_indication < info.level_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC AVCLevelIndication 0x{:02X} on track {track_id} is below the \
                 embedded SPS level_idc 0x{:02X} — ISO/IEC 14496-15:2017 §5.3.3.1.1",
                record.level_indication,
                info.level_idc,
            ),
        ));
    }
    if let Some(chroma) = record.chroma_format
        && chroma != info.chroma_format_idc
    {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC chroma_format {chroma} on track {track_id} disagrees with the \
                 embedded SPS chroma_format_idc {} — ISO/IEC 14496-15:2017 §5.3.3.1.1",
                info.chroma_format_idc,
            ),
        ));
    }
    if width != 0
        && height != 0
        && (u32::from(width), u32::from(height)) != (info.width, info.height)
    {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "container-sps-dimension-mismatch",
            alloc::format!(
                "Sample entry declares {width}x{height} on track {track_id} but the embedded \
                 SPS decodes to {}x{} — ITU-T H.264 §7.4.2.1.1 cropped coded geometry",
                info.width,
                info.height,
            ),
        ));
    }
    if !info.frame_mbs_only {
        report.push(Finding::new(
            Severity::Info,
            Location::at_track(0, track_id),
            "avc-interlaced-content",
            alloc::format!(
                "AVC SPS on track {track_id} has frame_mbs_only_flag=0 (interlaced coding \
                 tools enabled) — ITU-T H.264 §7.3.2.1.1",
            ),
        ));
    }
}

fn check_hevc_track(
    track_id: TrackId,
    record: &isobmff::HEVCDecoderConfigurationRecord,
    width: u16,
    height: u16,
    report: &mut Report,
) {
    let Some(info) = record
        .arrays
        .iter()
        .find(|a| a.nal_unit_type == HEVC_NAL_SPS)
        .and_then(|a| a.nalus.first())
        .and_then(|n| n.decode_sps().ok().flatten())
    else {
        return;
    };

    if record.general_profile_idc != info.general_profile_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC general_profile_idc {} on track {track_id} disagrees with the embedded \
                 SPS general_profile_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1.1",
                record.general_profile_idc,
                info.general_profile_idc,
            ),
        ));
    }
    if record.general_level_idc < info.general_level_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC general_level_idc {} on track {track_id} is below the embedded SPS \
                 general_level_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1.1",
                record.general_level_idc,
                info.general_level_idc,
            ),
        ));
    }
    if record.chroma_format_idc != info.chroma_format_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC chroma_format_idc {} on track {track_id} disagrees with the embedded SPS \
                 chroma_format_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1.1",
                record.chroma_format_idc,
                info.chroma_format_idc,
            ),
        ));
    }
    if width != 0
        && height != 0
        && (u32::from(width), u32::from(height)) != (info.width, info.height)
    {
        report.push(Finding::new(
            Severity::Error,
            Location::at_track(0, track_id),
            "container-sps-dimension-mismatch",
            alloc::format!(
                "Sample entry declares {width}x{height} on track {track_id} but the embedded \
                 SPS decodes to {}x{} — ITU-T H.265 §7.4.3.2.1 cropped coded geometry",
                info.width,
                info.height,
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "std")]
    use alloc::vec::Vec;

    /// Demux a whole in-memory MP4 image into a [`Media`] via the streaming
    /// stage, for the checks that need a `Media` to mutate.
    #[cfg(feature = "std")]
    fn test_media(bytes: &[u8]) -> Media {
        let total = u64::try_from(bytes.len()).expect("image fits u64");
        let mut demux = Mp4Demux::seekable(Mp4DemuxConfig::default().with_total_len(Some(total)));
        let mut events = Vec::new();
        loop {
            let demand = demux.demand();
            let Some(offset) = demand.offset else {
                break;
            };
            let start = usize::try_from(offset).expect("offset fits");
            let end = start.saturating_add(demand.want_bytes).min(bytes.len());
            demux
                .feed(
                    Mp4Input::At {
                        offset,
                        bytes: bytes[start..end].to_vec().into(),
                    },
                    Timestamp::ZERO,
                )
                .expect("range feed");
            while let Some(event) = demux.poll() {
                events.push(event);
            }
        }
        demux.finish().expect("finish");
        while let Some(event) = demux.poll() {
            events.push(event);
        }
        let movie_timescale = demux
            .movie_timescale()
            .unwrap_or(transmux::VIDEO_CLOCK_TIMESCALE);
        let mut tracks: Vec<Track> = Vec::new();
        let mut index: BTreeMap<TrackId, usize> = BTreeMap::new();
        for event in events {
            match event {
                DemuxEvent::TrackAdded(spec) => {
                    index.insert(spec.track_id, tracks.len());
                    tracks.push(Track::new(spec, Vec::new()));
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if let Some(&at) = index.get(&track_id) {
                        tracks[at].samples.push(sample);
                    }
                }
                _ => {}
            }
        }
        Media::new(tracks, movie_timescale)
    }

    /// Non-ISOBMFF bytes (e.g. a raw TS packet) must yield no findings, never
    /// a panic.
    #[test]
    fn compressed_top_level_boxes_are_isobmff() {
        for kind in [b"!mov", b"!mof"] {
            let mut bytes = 16u32.to_be_bytes().to_vec();
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(&[0u8; 8]);
            assert!(looks_like_isobmff(&bytes), "{kind:?}");
        }
    }

    #[test]
    fn non_isobmff_input_yields_no_findings() {
        let mut report = Report::new();
        check_container_codec(&[0x47, 0x00, 0x00, 0x10], &mut report);
        assert!(report.is_empty());
    }

    /// A truncated/garbage ISOBMFF-looking buffer must degrade to "nothing to
    /// say", never panic.
    #[test]
    fn garbage_isobmff_yields_no_findings() {
        let mut report = Report::new();
        check_container_codec(b"\x00\x00\x00\x08ftyp", &mut report);
        assert!(report.is_empty());
    }

    /// Audit MD-W6: the NAL length-prefix width comes from the track's own
    /// `avcC`/`hvcC` `lengthSizeMinusOne` (ISO/IEC 14496-15:2017 §5.3.3.1.3), not
    /// an assumed 4. A conformant stream with 2-byte prefixes must produce no
    /// `length-prefix-violation`.
    ///
    /// Built from the committed real capture
    /// `fixtures/mp4/h264-aac-prog.mp4`: demuxed through `transmux`, its
    /// `avcC` record set to `lengthSizeMinusOne = 1`, and every sample
    /// re-prefixed from 4 bytes to 2 — a conformant file our own muxer never
    /// produces (it always writes 4-byte prefixes) and therefore could not
    /// previously be exercised.
    #[test]
    #[cfg(feature = "std")]
    fn two_byte_nal_length_prefix_is_not_a_violation() {
        let path = test_fixtures::path("mp4/h264-aac-prog.mp4");
        let bytes = std::fs::read(path).expect("read h264-aac-prog.mp4");
        let mut media = test_media(&bytes);

        let mut video_tracks = 0usize;
        for track in &mut media.tracks {
            let CodecConfig::Avc { config, .. } = &mut track.spec.config else {
                continue;
            };
            video_tracks += 1;
            config.config.length_size_minus_one = 1;
            for sample in &mut track.samples {
                let nals = iter_length_prefixed_nals_with(&sample.data, 4)
                    .expect("fixture samples are 4-byte length-prefixed");
                let mut reprefixed = alloc::vec::Vec::new();
                for nal in nals {
                    let len = u16::try_from(nal.len()).expect("NAL fits in 2 bytes");
                    reprefixed.extend_from_slice(&len.to_be_bytes());
                    reprefixed.extend_from_slice(nal);
                }
                sample.data = reprefixed.into();
            }
        }
        assert!(video_tracks > 0, "fixture must carry an AVC track");

        let mut report = Report::new();
        check_media(&media, &mut report);
        assert!(
            report
                .findings()
                .iter()
                .all(|f| f.rule_id != "length-prefix-violation"),
            "a conformant 2-byte length-prefixed AVC track must not be reported as malformed; got {:?}",
            report.findings(),
        );
    }

    /// The width the check uses is read from the record, not assumed:
    /// `nal_length_size` maps each `lengthSizeMinusOne` to its byte count
    /// (ISO/IEC 14496-15:2017 §5.3.3.1.3) and rejects a non-AVC/HEVC config.
    #[test]
    #[cfg(feature = "std")]
    fn nal_length_size_comes_from_the_record() {
        let path = test_fixtures::path("mp4/h264-aac-prog.mp4");
        let bytes = std::fs::read(path).expect("read h264-aac-prog.mp4");
        let mut media = test_media(&bytes);

        let mut checked = 0usize;
        for track in &mut media.tracks {
            let CodecConfig::Avc { config, .. } = &mut track.spec.config else {
                continue;
            };
            checked += 1;
            for (minus_one, expected) in [(0u8, 1usize), (1, 2), (3, 4)] {
                // Mutate the record in place, then build the same config value
                // the check sees rather than re-borrowing `track.spec.config`.
                config.config.length_size_minus_one = minus_one;
                let cfg = CodecConfig::Avc {
                    config: config.clone(),
                    width: 0,
                    height: 0,
                };
                assert_eq!(
                    nal_length_size(&cfg),
                    Some(expected),
                    "lengthSizeMinusOne {minus_one} means a {expected}-byte prefix",
                );
            }
            // The reserved value 2 (a 3-byte prefix) is not a width this
            // check may interpret.
            config.config.length_size_minus_one = 2;
            let cfg = CodecConfig::Avc {
                config: config.clone(),
                width: 0,
                height: 0,
            };
            assert_eq!(nal_length_size(&cfg), None);
        }
        assert_eq!(checked, 1, "fixture carries one AVC track");
    }
}
