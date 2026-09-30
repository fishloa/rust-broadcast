//! `check_container_codec` — ISOBMFF/CMAF codec-level checks (issue #567).
//!
//! Companion to the TS-input codec checks in `diagnostics::{codec_signalling,
//! interlace, param_sets, fps_cadence}`, which operate on the
//! [`crate::Diagnostic`] trait's raw TS byte-stream. A fragmented or
//! progressive MP4 file has no PMT to cross-validate; instead the
//! sample-entry config box (`avcC`/`hvcC`) and its `width`/`height` fields
//! *are* the container's claim, and the SPS embedded in that very config box
//! is the bitstream truth. This function demuxes `bytes` into a
//! [`transmux::Media`] (trying [`transmux::Fmp4Demux`] for fragmented/CMAF
//! input, then [`transmux::ProgressiveDemux`] for a single-`moov` progressive
//! file) and, for every AVC/HEVC video track, cross-checks:
//!
//! - **`avcc-sps-mismatch`** / **`hvcc-sps-mismatch`** (Error): the config
//!   record's `profile_indication`/`level_indication` (AVC) or
//!   `general_profile_idc`/`general_level_idc` (HEVC) disagrees with the
//!   same fields re-decoded from the record's own embedded SPS
//!   ([`transmux::decode_avc_sps`]/[`transmux::decode_hevc_sps`] — reused,
//!   never re-implemented) — ISO/IEC 14496-15:2017 §5.3.3/§8.3.3.
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
//!   ([`transmux::iter_length_prefixed_nals`]) — the structural signature of
//!   an Annex B start code smuggled into an MP4 sample in place of a length
//!   prefix (ISO/IEC 14496-15:2017 §5.4.3.2.3 "no Annex B start codes").
//!
//! Non-ISOBMFF input, or input that fails to demux (e.g. a bare `sidx`-only
//! stub with no `moov`), yields no findings — this function has nothing to
//! say about a container it cannot parse, which must never surface as a
//! false positive.
//!
//! Follows the same free-function `&mut Report` out-parameter convention as
//! [`crate::check_playlist`]; [`Location::packet`] holds the sample index (0
//! for track-level findings) and [`Location::pid`] holds the MP4 track_id
//! (not a TS PID — there is none for this input).

use broadcast_common::Unpackage;
use transmux::{CodecConfig, Fmp4Demux, Media, ProgressiveDemux, iter_length_prefixed_nals_with};

use crate::report::{Finding, Location, Report, Severity};

/// H.265 `NAL_unit_type` for a sequence parameter set (ITU-T H.265 §7.3.1.2
/// Table 7-1) — used to pick the SPS array out of `hvcC`'s `numOfArrays`
/// loop (ISO/IEC 14496-15:2017 §8.3.3).
const HEVC_NAL_SPS: u8 = 33;

/// Run every ISOBMFF/CMAF codec-level check against `bytes`, appending
/// findings to `report`.
pub fn check_container_codec(bytes: &[u8], report: &mut Report) {
    if !looks_like_isobmff(bytes) {
        return;
    }
    let media = match Fmp4Demux::new().unpackage(bytes) {
        Ok(m) => Some(m),
        // `bytes` is already the whole file in memory here (the `Unpackage`
        // path, not `Stage::feed`), so the bound just needs to cover it —
        // `bytes.len()` never rejects (transmux issue B7: `ProgressiveDemux`
        // now requires an explicit `max_bytes` bound at construction).
        // `ProgressiveDemux::new` is fallible since it now rejects a zero
        // cap (transmux issue R1); `bytes.len()` is guaranteed non-zero here
        // by `looks_like_isobmff`'s `bytes.len() < 8` guard above, so the
        // only way this `Err` arm is reached is a genuine internal bug, not
        // a real input this function needs to tolerate — treat it the same
        // as "does not demux".
        Err(_) => ProgressiveDemux::new(bytes.len())
            .ok()
            .and_then(|mut demux| demux.unpackage(bytes).ok()),
    };
    if let Some(media) = media {
        check_media(&media, report);
    }
}

/// A minimal ISOBMFF sniff: the first box is a `size(4)+type(4)` header
/// naming a top-level box type every ISOBMFF/CMAF/DASH file starts with.
fn looks_like_isobmff(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    matches!(
        &bytes[4..8],
        b"ftyp" | b"styp" | b"moov" | b"moof" | b"free" | b"skip"
    )
}

/// The NAL length-prefix width a track's codec configuration declares, in
/// bytes — `lengthSizeMinusOne + 1` (ISO/IEC 14496-15 §5.3.3.1.1 `avcC`,
/// §8.3.3.1.2 `hvcC`). `None` for a track that is not AVC/HEVC.
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
    matches!(width, 1 | 2 | 4).then_some(width)
}

fn check_media(media: &Media, report: &mut Report) {
    for track in &media.tracks {
        // `track_ID` is a full 32-bit field (ISO/IEC 14496-12 §8.3.2) — never
        // narrowed, or every id above 65535 aliases onto a lower one.
        let track_id = track.track_id();
        // Length-prefixed NAL framing (ISO/IEC 14496-15:2017 §5.4.3.2.3) only
        // applies to AVC/HEVC video samples — `transmux::pipeline::Sample`
        // documents its `data` field as "length-prefixed NAL data for
        // AVC/HEVC, or the raw frame for AAC [and every other codec]".
        // Running this check on a non-NAL track (e.g. a raw AAC frame, which
        // is not length-prefixed at all) is not a structural fault — it
        // would misread arbitrary audio bytes as a bogus NAL length and
        // false-positive on every clean AAC track (confirmed against
        // `fixtures/transmux/h264_aac_frag.mp4`'s AAC track).
        let is_nal_track = matches!(
            &track.spec.config,
            CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
        );

        match &track.spec.config {
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

        if !is_nal_track {
            continue;
        }

        // The record's own `lengthSizeMinusOne` (ISO/IEC 14496-15 5.3.3.1.1
        // for `avcC`, 8.3.3.1.2 for `hvcC`) declares the prefix width —
        // 1-, 2- and 4-byte prefixes are all conformant. Assuming 4 bytes
        // reported a false `length-prefix-violation` on every conformant
        // AVC/HEVC track that uses a narrower prefix.
        let Some(length_size) = nal_length_size(&track.spec.config) else {
            continue;
        };
        for (idx, sample) in track.samples.iter().enumerate() {
            if iter_length_prefixed_nals_with(&sample.data, length_size).is_err() {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(idx, track_id),
                    "length-prefix-violation",
                    alloc::format!(
                        "Sample {idx} on track {track_id} is not a well-formed {length_size}-byte \
                         length-prefixed NAL sequence (declared length runs past the sample's \
                         own end) — a hallmark of an Annex B start code left in place of a \
                         length prefix (ISO/IEC 14496-15:2017 §5.4.3.2.3)",
                    ),
                ));
                // One finding per track is enough signal; do not spam every
                // subsequent sample of a track that is malformed throughout.
                break;
            }
        }
    }
}

fn check_avc_track(
    track_id: u32,
    record: &transmux::AVCDecoderConfigurationRecord,
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
            Location::new(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC AVCProfileIndication 0x{:02X} on track {track_id} disagrees with the \
                 embedded SPS profile_idc 0x{:02X} — ISO/IEC 14496-15:2017 §5.3.3.1.1",
                record.profile_indication,
                info.profile_idc,
            ),
        ));
    }
    if record.level_indication != info.level_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::new(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC AVCLevelIndication 0x{:02X} on track {track_id} disagrees with the \
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
            Location::new(0, track_id),
            "avcc-sps-mismatch",
            alloc::format!(
                "avcC chroma_format {chroma} on track {track_id} disagrees with the \
                 embedded SPS chroma_format_idc {} — ISO/IEC 14496-15:2017 §5.3.3.1.2",
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
            Location::new(0, track_id),
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
            Location::new(0, track_id),
            "avc-interlaced-content",
            alloc::format!(
                "AVC SPS on track {track_id} has frame_mbs_only_flag=0 (interlaced coding \
                 tools enabled) — ITU-T H.264 §7.3.2.1.1",
            ),
        ));
    }
}

fn check_hevc_track(
    track_id: u32,
    record: &transmux::HEVCDecoderConfigurationRecord,
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
            Location::new(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC general_profile_idc {} on track {track_id} disagrees with the embedded \
                 SPS general_profile_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1",
                record.general_profile_idc,
                info.general_profile_idc,
            ),
        ));
    }
    if record.general_level_idc != info.general_level_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::new(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC general_level_idc {} on track {track_id} disagrees with the embedded SPS \
                 general_level_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1",
                record.general_level_idc,
                info.general_level_idc,
            ),
        ));
    }
    if record.chroma_format_idc != info.chroma_format_idc {
        report.push(Finding::new(
            Severity::Error,
            Location::new(0, track_id),
            "hvcc-sps-mismatch",
            alloc::format!(
                "hvcC chroma_format_idc {} on track {track_id} disagrees with the embedded SPS \
                 chroma_format_idc {} — ISO/IEC 14496-15:2017 §8.3.3.1",
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
            Location::new(0, track_id),
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

    /// Non-ISOBMFF bytes (e.g. a raw TS packet) must yield no findings, never
    /// a panic.
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
    /// `avcC`/`hvcC` `lengthSizeMinusOne` (ISO/IEC 14496-15 5.3.3.1.1), not
    /// an assumed 4. A conformant stream with 2-byte prefixes must produce no
    /// `length-prefix-violation`.
    ///
    /// Built from the committed real capture
    /// `fixtures/transmux/h264_aac_prog.mp4`: demuxed through `transmux`, its
    /// `avcC` record set to `lengthSizeMinusOne = 1`, and every sample
    /// re-prefixed from 4 bytes to 2 — a conformant file our own muxer never
    /// produces (it always writes 4-byte prefixes) and therefore could not
    /// previously be exercised.
    #[test]
    fn two_byte_nal_length_prefix_is_not_a_violation() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/transmux/h264_aac_prog.mp4"
        );
        let bytes = std::fs::read(path).expect("read h264_aac_prog.mp4");
        let mut media = ProgressiveDemux::new(bytes.len())
            .expect("demuxer")
            .unpackage(&bytes)
            .expect("demux");

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
    /// (ISO/IEC 14496-15 5.3.3.1.1) and rejects a non-AVC/HEVC config.
    #[test]
    fn nal_length_size_comes_from_the_record() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/transmux/h264_aac_prog.mp4"
        );
        let bytes = std::fs::read(path).expect("read h264_aac_prog.mp4");
        let mut media = ProgressiveDemux::new(bytes.len())
            .expect("demuxer")
            .unpackage(&bytes)
            .expect("demux");

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
