//! Smooth Streaming `CodecPrivateData` glue — the part of the client Manifest
//! that needs `transmux`'s own codec types (init-segment synthesis).
//!
//! Microsoft Smooth Streaming (MS-SSTR,
//! `private/specs-verified/mpeg/transmux/smooth/ms-sstr.md`). The manifest
//! itself — its typed model, parse and serialize — lives in `broadcast-smooth`
//! ([`broadcast_smooth::SmoothManifest`]); this module turns one of its
//! [`QualityLevel`]s into the [`TrackSpec`] a caller feeds to
//! [`crate::pipeline::build_init_segment`].
//!
//! # Init-segment synthesis
//!
//! Smooth has no bootstrapping init segment — a `QualityLevel`'s
//! `CodecPrivateData` IS the codec config. [`track_spec_from_quality_level`]
//! builds the `TrackSpec` so [`crate::media::Fmp4Demux`] (which hard-requires a
//! `moov`) can then absorb the Smooth fragment stream:
//! - `FourCC="H264"`: the Annex-B-framed `CodecPrivateData` (start-code
//!   delimited SPS+PPS) is split with [`crate::annexb::iter_annexb_nals`] and
//!   classified/assembled into an `avcC` via
//!   [`crate::rtp_sdp::avc_config_from_sps_pps`].
//! - `FourCC="AACL"`/`"AACH"`: the `CodecPrivateData` bytes ARE the
//!   `AudioSpecificConfig`, carried straight into `CodecConfig::Aac` via
//!   [`crate::rtp_sdp::aac_config_from_asc_bytes`]; when absent, an ASC is
//!   synthesised from `SamplingRate`/`Channels`.
//! - `Type="text"` with `FourCC="stpp"`/`"wvtt"`: a `CodecConfig::Subtitle`
//!   with a synthesised ISOBMFF sample entry.
//!
//! [`track_spec_from_quality_level`] dispatches on `QualityLevel@FourCC`
//! (case-insensitively) and rejects any other token (a Dolby `EC-3`/`AC-3`
//! audio level, an `H265` video level, a private one) with
//! `Error::UnsupportedCodec`: those carry their own CodecPrivateData shape, and
//! parsing Dolby bytes as an AudioSpecificConfig would silently yield a garbage
//! AAC track.

use crate::TrackId;
use alloc::vec::Vec;
use broadcast_common::error::InvalidValue;

use broadcast_smooth::{QualityLevel, StreamType};

use crate::aac_asc::{ChannelConfiguration, SamplingFrequencyIndex};
use crate::annexb::iter_annexb_nals;
use crate::error::{Error as CrateError, Result as CrateResult};
use crate::nal::{NalCodec, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, SubtitleFormat, TrackSpec};
use crate::rtp_sdp::{aac_config_from_asc_bytes, avc_config_from_sps_pps};
use crate::smooth::{FOURCC_AACH, FOURCC_AACL, FOURCC_AVC1, FOURCC_H264};
use crate::subtitle_entries::{WvttSampleEntry, XmlSubtitleSampleEntry};

/// The TTML namespace a synthesised `stpp` sample entry names (W3C TTML2).
const TTML_NAMESPACE: &str = "http://www.w3.org/ns/ttml";
/// The bare WebVTT file-header block a synthesised `wvtt` `vttC` carries
/// (WebVTT §4.1 file signature; ISO/IEC 14496-30 §9.2).
const WEBVTT_HEADER: &str = "WEBVTT";

/// H.264 `nal_unit_type` for a sequence parameter set (SPS) —
/// ITU-T H.264 §7.4.1 Table 7-1.
const AVC_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for a picture parameter set (PPS).
const AVC_NAL_PPS: u8 = 8;

/// Build a [`TrackSpec`] (feed to [`crate::pipeline::build_init_segment`]) from
/// a `QualityLevel`'s `CodecPrivateData`, keyed by the enclosing
/// `StreamIndex`'s [`StreamType`] — the init-segment synthesis Smooth needs
/// in place of a bootstrapping init segment (see the module docs).
///
/// - [`StreamType::Video`]: `FourCC` `H264` (a.k.a. `AVC1`, [MS-SSTR]
///   §2.2.2.5) — splits the Annex-B `CodecPrivateData` into SPS/PPS NAL units
///   and builds an `avcC` via [`crate::rtp_sdp::avc_config_from_sps_pps`];
///   geometry prefers the SPS-decoded coded dimensions (authoritative),
///   falling back to the `QualityLevel`'s `MaxWidth`/`MaxHeight` if the SPS
///   doesn't decode.
/// - [`StreamType::Audio`]: `FourCC` `AACL` or `AACH` — the `CodecPrivateData`
///   bytes ARE the `AudioSpecificConfig`, carried via
///   [`crate::rtp_sdp::aac_config_from_asc_bytes`]. When it is absent (allowed
///   by [MS-SSTR]), the ASC is synthesised from the `QualityLevel`'s
///   `SamplingRate`/`Channels`.
/// - [`StreamType::Text`]: `FourCC` `stpp` (TTML) or `wvtt` (WebVTT) — the
///   `SmoothPackager`'s own text stream — becomes a
///   [`CodecConfig::Subtitle`] carrying a synthesised ISOBMFF sample entry
///   (TTML namespace / `WEBVTT` header; the manifest holds neither). Any other
///   text FourCC is [`crate::Error::UnsupportedCodec`].
///
/// `FourCC` matching is case-insensitive (real manifests use `AACL` and
/// `aacl`); any unrecognised token is [`crate::Error::UnsupportedCodec`].
pub fn track_spec_from_quality_level(
    track_id: TrackId,
    timescale: u32,
    stream_type: StreamType,
    quality: &QualityLevel,
) -> CrateResult<TrackSpec> {
    match stream_type {
        StreamType::Video => {
            // [MS-SSTR] §2.2.2.5 names the video FourCC `H264`, a.k.a. `AVC1`.
            if !is_fourcc(&quality.four_cc, FOURCC_H264)
                && !is_fourcc(&quality.four_cc, FOURCC_AVC1)
            {
                return Err(CrateError::UnsupportedCodec {
                    codec: "Smooth video FourCC (only H264/AVC1 is synthesised)",
                });
            }
            let mut sps: Vec<AvcSps> = Vec::new();
            let mut pps: Vec<AvcPps> = Vec::new();
            for nal in iter_annexb_nals(&quality.codec_private_data) {
                match nal_unit_type(NalCodec::Avc, nal) {
                    Some(AVC_NAL_SPS) => sps.push(AvcSps(nal.to_vec())),
                    Some(AVC_NAL_PPS) => pps.push(AvcPps(nal.to_vec())),
                    _ => {}
                }
            }
            let config = avc_config_from_sps_pps(sps, pps)?;
            let (width, height) = config
                .config
                .sps
                .first()
                .and_then(|s| s.decode().ok())
                .map(|info| {
                    Ok::<_, CrateError>((
                        u16::try_from(info.width).map_err(|_| {
                            CrateError::InvalidValue(InvalidValue {
                                field: "SPS sps_pic_width_max_in_luma_samples",
                                value: u64::from(info.width),
                                reason: "does not fit the IR's 16-bit dimension field",
                            })
                        })?,
                        u16::try_from(info.height).map_err(|_| {
                            CrateError::InvalidValue(InvalidValue {
                                field: "SPS sps_pic_height_max_in_luma_samples",
                                value: u64::from(info.height),
                                reason: "does not fit the IR's 16-bit dimension field",
                            })
                        })?,
                    ))
                })
                .transpose()?
                .unwrap_or((
                    checked_u16(quality.width, "QualityLevel@MaxWidth")?,
                    checked_u16(quality.height, "QualityLevel@MaxHeight")?,
                ));
            Ok(TrackSpec::new(
                track_id,
                timescale,
                CodecConfig::Avc {
                    config,
                    width,
                    height,
                },
            ))
        }
        StreamType::Audio => {
            // [MS-SSTR] §2.2.2.5 names the audio FourCC `AACL` (AAC-LC) and
            // `AACH` (HE-AAC). A Dolby FourCC (`EC-3`/`AC-3`) instead carries
            // Dolby `CodecPrivateData`, which "parses" as an ASC (any ≥2-byte
            // buffer does) and would become a garbage `CodecConfig::Aac` track
            // with a nonsense rate/channel count — reject it.
            // HE-AAC (`AACH`) carries an AAC-LC core too, so both FourCCs
            // synthesise the same core ASC (see `synthesise_asc`).
            if !is_fourcc(&quality.four_cc, FOURCC_AACL)
                && !is_fourcc(&quality.four_cc, FOURCC_AACH)
            {
                return Err(CrateError::UnsupportedCodec {
                    codec: "Smooth audio FourCC (only AACL/AACH is synthesised)",
                });
            }
            // No `CodecPrivateData`: [MS-SSTR] allows the config to be derived
            // from the quality level's own attributes, so build the ASC.
            let asc = if quality.codec_private_data.is_empty() {
                synthesise_asc(AOT_AAC_LC, quality)?
            } else {
                quality.codec_private_data.clone()
            };
            let config = aac_config_from_asc_bytes(asc)?;
            Ok(TrackSpec::new(track_id, timescale, config))
        }
        StreamType::Text => {
            // The manifest names the sample format by its ISOBMFF sample-entry
            // fourcc ([MS-SSTR] §2.2.2.5: a registered MPEG4-RA code) and
            // carries no `CodecPrivateData` for it (what `SmoothPackager`
            // writes), so the entry is rebuilt from the format's own identity:
            // the W3C TTML namespace for `stpp`, the bare `WEBVTT` header
            // block for `wvtt` (ISO/IEC 14496-30 §7.2 / §9.2).
            use crate::pipeline::SubtitleSampleEntry;
            use crate::smooth::{FOURCC_STPP, FOURCC_WVTT};
            let (format, entry) = if is_fourcc(&quality.four_cc, FOURCC_STPP) {
                (
                    SubtitleFormat::Ttml,
                    SubtitleSampleEntry::Ttml(XmlSubtitleSampleEntry::new(TTML_NAMESPACE)),
                )
            } else if is_fourcc(&quality.four_cc, FOURCC_WVTT) {
                (
                    SubtitleFormat::WebVtt,
                    SubtitleSampleEntry::WebVtt(WvttSampleEntry::new(WEBVTT_HEADER)),
                )
            } else {
                return Err(CrateError::UnsupportedCodec {
                    codec: "Smooth text FourCC (only stpp/wvtt is synthesised)",
                });
            };
            Ok(TrackSpec::new(
                track_id,
                timescale,
                CodecConfig::Subtitle {
                    format,
                    sample_entry: Some(entry),
                },
            ))
        }
    }
}

/// Case-insensitive `FourCC` comparison ([MS-SSTR] `QualityLevel@FourCC`).
fn is_fourcc(value: &str, expected: &str) -> bool {
    value.eq_ignore_ascii_case(expected)
}

/// Convert an optional `QualityLevel` dimension to the IR's `u16`, rejecting an
/// over-range value rather than truncating it (the `#997` class).
fn checked_u16(value: Option<u32>, field: &'static str) -> CrateResult<u16> {
    match value {
        Some(v) => u16::try_from(v).map_err(|_| {
            CrateError::InvalidValue(InvalidValue {
                field,
                value: u64::from(v),
                reason: "does not fit the IR's 16-bit dimension field",
            })
        }),
        None => Ok(0),
    }
}

/// Synthesise an `AudioSpecificConfig` from a `QualityLevel`'s
/// `SamplingRate`/`Channels` attributes (no `CodecPrivateData`).
///
/// ISO/IEC 14496-3 §1.6.2.1 `AudioSpecificConfig()` is
/// `audioObjectType(5)` | `samplingFrequencyIndex(4)` | `channelConfiguration(4)`,
/// each packed MSB-first into the leading bytes:
/// - `audioObjectType`: 2 (AAC-LC) — the **core** object type.
/// - `samplingFrequencyIndex`: the shared ISO/IEC 14496-3 Table 1.10 index for
///   `SamplingRate`. A rate that is not a Table 1.10 entry is
///   [`crate::Error::UnsupportedCodec`]: this synthesiser writes only the
///   two-byte core form, so it cannot express the `samplingFrequencyIndex == 0xF`
///   explicit-24-bit-rate escape (that would need the rate appended after the
///   index), and writing the escape index alone would describe a config with no
///   sample rate.
/// - `channelConfiguration`: `Channels` mapped through Table 1.19 (1, 2, 3, 4,
///   5, 6, 8); a count with no defined mapping is
///   [`crate::Error::UnsupportedCodec`].
///
/// **HE-AAC (`AACH`)**: only the core AAC-LC config is written. The hierarchical
/// explicit-signalling form of §1.6.2.1 (`extensionAudioObjectType = 5` +
/// `extensionSamplingFrequencyIndex` + the core AOT) is not synthesised, so the
/// SBR extension stays **implicit** — a decoder derives it from the
/// backward-compatible sync extension in the stream itself. That is a legal
/// carriage (§1.6.2.1 allows implicit signalling) and honest about what these
/// two bytes say; it does mean the `esds` reports the core rate, not twice it.
fn synthesise_asc(aot: u8, quality: &QualityLevel) -> CrateResult<Vec<u8>> {
    let rate = quality.sampling_rate.ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC QualityLevel with no SamplingRate and no CodecPrivateData",
    })?;
    let channels = quality.channels.ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC QualityLevel with no Channels and no CodecPrivateData",
    })?;
    let sf_index = frequency_index_for(rate).ok_or(CrateError::UnsupportedCodec {
        codec: "Smooth AAC SamplingRate is not an ISO/IEC 14496-3 Table 1.10 entry",
    })?;
    let channel_raw = channel_configuration_for(channels)?;

    // audioObjectType(5) | samplingFrequencyIndex(4) | channelConfiguration(4)
    // → 13 bits → 2 bytes, the remaining 3 bits zero. Only the core AAC-LC
    // object type is written (see the doc above).
    let mut out = alloc::vec![0u8; 2];
    out[0] = (aot << 3) | (sf_index >> 1);
    out[1] = ((sf_index & 0x01) << 7) | (channel_raw << 3);
    Ok(out)
}

/// The `samplingFrequencyIndex` for a rate, from the crate's single copy of
/// ISO/IEC 14496-3 Table 1.10 (`SamplingFrequencyIndex::raw`).
///
/// A rate absent from Table 1.10 is `None`: this synthesiser writes only the
/// two-byte core ASC (`audioObjectType`/`samplingFrequencyIndex`/
/// `channelConfiguration`), so it cannot express the 24-bit explicit-rate
/// escape (`samplingFrequencyIndex == 0xF`) without also appending that value.
/// Returning `None` and rejecting is honest; silently writing the escape index
/// with no rate would describe a config with no sample rate at all.
fn frequency_index_for(rate: u32) -> Option<u8> {
    SAMPLING_FREQUENCY_INDICES
        .iter()
        .find(|c| c.table_hz() == Some(rate))
        .map(|c| c.raw())
}
/// The `audioObjectType` for AAC-LC (ISO/IEC 14496-3 Table 1.18).
const AOT_AAC_LC: u8 = 2;

/// Every non-reserved `SamplingFrequencyIndex`, in Table 1.10 order.
const SAMPLING_FREQUENCY_INDICES: [SamplingFrequencyIndex; 12] = [
    SamplingFrequencyIndex::Fs96000,
    SamplingFrequencyIndex::Fs88200,
    SamplingFrequencyIndex::Fs64000,
    SamplingFrequencyIndex::Fs48000,
    SamplingFrequencyIndex::Fs44100,
    SamplingFrequencyIndex::Fs32000,
    SamplingFrequencyIndex::Fs24000,
    SamplingFrequencyIndex::Fs22050,
    SamplingFrequencyIndex::Fs16000,
    SamplingFrequencyIndex::Fs12000,
    SamplingFrequencyIndex::Fs8000,
    SamplingFrequencyIndex::Fs7350,
];

/// Map a channel count to `channelConfiguration` (ISO/IEC 14496-3 Table 1.19):
/// 1→mono, 2→stereo, 3→`Ch3`, 4→`Ch4`, 5→`Ch5`, 6→`Ch5_1` (5.1), 8→`Ch7_1`
/// (7.1: 3 front + 2 side + 2 back + LFE). Table 1.19 has no 7-channel
/// configuration — `Ch7_1` is eight — so 7 is rejected, not mis-mapped. Any
/// other count is [`crate::Error::UnsupportedCodec`] rather than a wrong
/// mapping.
fn channel_configuration_for(channels: u16) -> CrateResult<u8> {
    match channels {
        1 => Ok(ChannelConfiguration::Mono.raw()),
        2 => Ok(ChannelConfiguration::Stereo.raw()),
        3 => Ok(ChannelConfiguration::Ch3.raw()),
        4 => Ok(ChannelConfiguration::Ch4.raw()),
        5 => Ok(ChannelConfiguration::Ch5.raw()),
        6 => Ok(ChannelConfiguration::Ch5_1.raw()),
        8 => Ok(ChannelConfiguration::Ch7_1.raw()),
        _ => Err(CrateError::UnsupportedCodec {
            codec: "Smooth AAC channel count with no Table 1.19 configuration",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::pipeline::CodecConfig;

    fn video_quality(four_cc: &str, cpd: Vec<u8>) -> QualityLevel {
        QualityLevel::new(four_cc, VIDEO_BITRATE, cpd).with_size(WIDTH, HEIGHT)
    }

    const VIDEO_BITRATE: u64 = 500_000;
    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 360;

    // -- codec glue ------------------------------------------------------------

    #[test]
    fn track_spec_from_quality_level_video_avc() {
        // Real-shaped (if tiny) SPS/PPS: type 7 (SPS) then type 8 (PPS),
        // Annex-B start-code delimited, matching the writer's CodecPrivateData shape.
        let cpd = alloc::vec![
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB, 0x00, 0x00, 0x00, 0x01, 0x68,
            0xCE, 0x3C, 0x80,
        ];
        let quality = QualityLevel::new("H264".to_string(), 500_000, cpd);
        let spec =
            track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality)
                .expect("build video TrackSpec");
        match spec.config {
            CodecConfig::Avc { config, .. } => {
                assert_eq!(config.config.sps.len(), 1);
                assert_eq!(config.config.pps.len(), 1);
                assert_eq!(
                    config.config.sps[0].0,
                    alloc::vec![0x67, 0x42, 0xC0, 0x1E, 0xAB]
                );
                assert_eq!(config.config.pps[0].0, alloc::vec![0x68, 0xCE, 0x3C, 0x80]);
            }
            _ => panic!("expected CodecConfig::Avc"),
        }
    }

    #[test]
    fn track_spec_from_quality_level_audio_aac() {
        let quality = QualityLevel::new("AACL".to_string(), 128_000, alloc::vec![0x12, 0x10]);
        let spec =
            track_spec_from_quality_level(TrackId::new(2), 44_100, StreamType::Audio, &quality)
                .expect("build audio TrackSpec");
        match spec.config {
            CodecConfig::Aac {
                sample_rate,
                channel_count,
                ..
            } => {
                assert_eq!(sample_rate, 44100);
                assert_eq!(channel_count, 2);
            }
            _ => panic!("expected CodecConfig::Aac"),
        }
    }

    #[test]
    fn track_spec_from_quality_level_audio_dolby_fourcc_rejected() {
        // `FourCC="EC-3"` carries Dolby (E-)AC-3 CodecPrivateData, not an
        // AudioSpecificConfig — this must be rejected, not silently parsed as
        // an AAC track. The gate is the FourCC itself (the bytes are
        // deliberately a plausible ASC, so only the FourCC check stops it).
        let quality = QualityLevel::new("EC-3".to_string(), 128_000, alloc::vec![0x10, 0x3D]);
        match track_spec_from_quality_level(TrackId::new(2), 48_000, StreamType::Audio, &quality) {
            Err(CrateError::UnsupportedCodec { codec }) => {
                assert_eq!(codec, "Smooth audio FourCC (only AACL/AACH is synthesised)");
            }
            other => panic!("expected UnsupportedCodec for EC-3, got {other:?}"),
        }
        let quality = QualityLevel::new(
            "AC-3".to_string(),
            quality.bitrate,
            quality.codec_private_data.clone(),
        );
        assert!(matches!(
            track_spec_from_quality_level(TrackId::new(2), 48_000, StreamType::Audio, &quality),
            Err(CrateError::UnsupportedCodec { .. })
        ));
    }

    #[test]
    fn track_spec_from_quality_level_video_non_h264_fourcc_rejected() {
        // The video path builds an `avcC` from Annex-B SPS/PPS; a non-H264
        // FourCC (HEVC `H265`) must not take that path.
        let quality = QualityLevel::new(
            "H265".to_string(),
            1_000_000,
            alloc::vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB],
        );
        match track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality) {
            Err(CrateError::UnsupportedCodec { codec }) => {
                assert_eq!(codec, "Smooth video FourCC (only H264/AVC1 is synthesised)");
            }
            other => panic!("expected UnsupportedCodec for H265, got {other:?}"),
        }
    }

    /// `FourCC` matching is case-insensitive ([MS-SSTR] §2.2.2.5 names `H264`
    /// a.k.a. `AVC1`, and manifest authors are inconsistent about case).
    #[test]
    fn four_cc_matching_is_case_insensitive() {
        // Video `H264` / `AVC1` / lowercase.
        for cc in ["H264", "h264", "AVC1", "avc1"] {
            let quality = video_quality(
                cc,
                alloc::vec![
                    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1E, 0xAB, 0x00, 0x00, 0x00, 0x01,
                    0x68, 0xCE, 0x3C, 0x80,
                ],
            );
            assert!(
                track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality)
                    .is_ok(),
                "video FourCC {cc} must be accepted"
            );
        }
        // Audio `AACL` / `AACH` / lowercase.
        for cc in ["AACL", "aacl", "AACH", "aach"] {
            let quality = QualityLevel::new(cc.to_string(), 128_000, alloc::vec![0x12, 0x10]);
            assert!(
                track_spec_from_quality_level(TrackId::new(2), 44_100, StreamType::Audio, &quality)
                    .is_ok(),
                "audio FourCC {cc} must be accepted"
            );
        }
    }

    /// An `AACL` `QualityLevel` with no `CodecPrivateData` synthesises an ASC
    /// from `SamplingRate`/`Channels` (ISO/IEC 14496-3 §1.6.2.1). For AAC-LC
    /// 44 100 Hz stereo the ASC is the literal `12 10`
    /// (`audioObjectType` 2, `samplingFrequencyIndex` 4, `channelConfiguration`
    /// 2), cross-checked against the crate's own ASC parser.
    #[test]
    fn empty_codec_private_data_synthesises_aac_lc_asc() {
        let quality = QualityLevel::new("AACL".to_string(), 128_000, Vec::new())
            .with_audio(44_100, 2, 16, 0, 0);
        // The synthesiser's literals.
        assert_eq!(
            synthesise_asc(AOT_AAC_LC, &quality).unwrap(),
            alloc::vec![0x12, 0x10]
        );
        // And the crate's own parser agrees about what those bytes mean.
        let asc =
            <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&[0x12, 0x10])
                .unwrap();
        assert_eq!(asc.sampling_frequency_index.raw(), 4);
        assert_eq!(asc.channel_configuration.raw(), 2);

        let spec =
            track_spec_from_quality_level(TrackId::new(2), 44_100, StreamType::Audio, &quality)
                .expect("empty CodecPrivateData must synthesise an ASC");
        let CodecConfig::Aac {
            sample_rate,
            channel_count,
            esds,
            ..
        } = spec.config
        else {
            panic!("expected CodecConfig::Aac");
        };
        assert_eq!((sample_rate, channel_count), (44_100, 2));
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|d| d.decoder_specific_info.as_ref())
            .expect("esds carries a DecoderSpecificInfo");
        assert_eq!(
            dsi.data.as_ref(),
            [0x12u8, 0x10],
            "the esds DecoderSpecificInfo must be the literal AAC-LC ASC 12 10"
        );
    }

    /// `AACH` (HE-AAC) with no `CodecPrivateData` synthesises the plain AAC-LC
    /// **core** ASC — the SBR extension is implicit, derived from the stream's
    /// backward-compatible signalling rather than the two config bytes — so the
    /// bytes equal the `AACL` case and `heaac_signaling()` reports no explicit
    /// SBR.
    #[test]
    fn empty_codec_private_data_synthesises_core_asc_for_aach() {
        let quality = QualityLevel::new("AACH".to_string(), 64_000, Vec::new())
            .with_audio(44_100, 2, 16, 0, 0);
        // Through the real entry point, so the AOT the caller picks is what
        // actually lands in the esds.
        let spec =
            track_spec_from_quality_level(TrackId::new(3), 44_100, StreamType::Audio, &quality)
                .expect("AACH must synthesise a core ASC");
        let CodecConfig::Aac {
            sample_rate,
            channel_count,
            esds,
            ..
        } = spec.config
        else {
            panic!("expected CodecConfig::Aac");
        };
        assert_eq!((sample_rate, channel_count), (44_100, 2));
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|d| d.decoder_specific_info.as_ref())
            .expect("esds carries a DecoderSpecificInfo");
        assert_eq!(
            dsi.data.as_ref(),
            [0x12u8, 0x10],
            "the esds DecoderSpecificInfo must be the core AAC-LC ASC"
        );
        let asc =
            <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&dsi.data)
                .unwrap();
        assert_eq!(asc.audio_object_type.raw(), AOT_AAC_LC);
        let he = asc.heaac_signaling();
        assert!(!he.sbr_present, "SBR is implicit, not explicitly signalled");
        assert!(!he.ps_present);
    }

    /// Every Table 1.19 channel count synthesises its literal ASC bytes (48 kHz
    /// AAC-LC: `audioObjectType` 2, `samplingFrequencyIndex` 3): 1 → `11 88`,
    /// 2 → `11 90`, 6 → `11 B0`, 8 → `11 B8`. `Ch7_1` is **eight** channels, so
    /// a 7-channel level is rejected, not mapped onto it.
    #[test]
    fn channel_configurations_synthesise_literal_asc_bytes() {
        let base = QualityLevel::new("AACL".to_string(), 128_000, Vec::new());
        for (channels, expected) in [
            (1u16, [0x11u8, 0x88]),
            (2, [0x11, 0x90]),
            (6, [0x11, 0xB0]),
            (8, [0x11, 0xB8]),
        ] {
            let q = base.clone().with_audio(48_000, channels, 16, 0, 0);
            assert_eq!(
                synthesise_asc(AOT_AAC_LC, &q).unwrap(),
                expected,
                "channels={channels}"
            );
            // The crate's own parser agrees about the channel count.
            let asc =
                <crate::aac_asc::AudioSpecificConfig as broadcast_common::Parse>::parse(&expected)
                    .unwrap();
            assert_eq!(asc.channel_configuration.channel_count(), Some(channels));
        }
        // 7 has no Table 1.19 configuration (Ch7_1 is 8).
        let seven = base.with_audio(48_000, 7, 16, 0, 0);
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &seven),
            Err(CrateError::UnsupportedCodec { .. })
        ));
    }

    /// A rate that is not an ISO/IEC 14496-3 Table 1.10 entry (e.g. 44 000 Hz)
    /// and a channel count with no Table 1.19 mapping are `UnsupportedCodec`,
    /// never a wrong rate/count.
    #[test]
    fn unsupported_rate_or_channels_are_rejected_not_guessed() {
        let base = QualityLevel::new("AACL".to_string(), 128_000, Vec::new());
        let odd_rate = base.clone().with_audio(44_000, 2, 16, 0, 0);
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &odd_rate),
            Err(CrateError::UnsupportedCodec { .. })
        ));
        let odd_channels = base.clone().with_audio(48_000, 9, 16, 0, 0);
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &odd_channels),
            Err(CrateError::UnsupportedCodec { .. })
        ));
        // Missing attributes are also errors, not defaults.
        let mut no_rate = base.clone();
        no_rate.sampling_rate = None;
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &no_rate),
            Err(crate::Error::UnsupportedCodec { .. })
        ));
        let mut no_channels = base;
        no_channels.channels = None;
        assert!(matches!(
            synthesise_asc(AOT_AAC_LC, &no_channels),
            Err(crate::Error::UnsupportedCodec { .. })
        ));
    }

    /// The SPS-decoded geometry is authoritative, so an SPS whose coded width
    /// exceeds `u16::MAX` is `Error::InvalidValue` (not `InvalidInput`, and not
    /// a wrapped dimension) — the same class the `MaxWidth` fallback rejects.
    ///
    /// The SPS bytes are the one from `fixtures/flv/oversize-dims.flv`
    /// (`pic_width_in_mbs_minus1 = 4095` → width 65 536, height 48).
    #[test]
    fn oversized_sps_dimensions_are_invalid_value() {
        let quality = QualityLevel::new(
            "H264",
            1,
            alloc::vec![
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0xF4, 0x00, 0x08, 0x00, 0x38, 0x80,
            ],
        );
        // The SPS really does decode to 65 536 (so the error below is the
        // over-range check, not a decode failure falling back to `None`).
        let decoded = crate::sps::decode_avc_sps(&quality.codec_private_data[4..])
            .expect("the oversize SPS decodes");
        assert_eq!(decoded.width, 65_536);
        match track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality) {
            Err(CrateError::InvalidValue(InvalidValue {
                field: "SPS sps_pic_width_max_in_luma_samples",
                value: 65_536,
                ..
            })) => {}
            other => panic!("expected an SPS width overflow InvalidValue, got {other:?}"),
        }
    }

    /// A `QualityLevel` dimension above `u16::MAX` is rejected, not truncated
    /// (the `#997` class), and this holds when the SPS does not decode (the
    /// attribute fallback path).
    #[test]
    fn oversized_quality_level_dimensions_are_rejected() {
        let quality = QualityLevel::new(
            "H264".to_string(),
            1,
            alloc::vec![0x00, 0x00, 0x00, 0x01, 0x67, 0xFF, 0xFF, 0xFF, 0xFF,],
        )
        .with_size(70_000, 360);
        match track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality) {
            Err(CrateError::InvalidValue(InvalidValue {
                field: "QualityLevel@MaxWidth",
                value: 70_000,
                ..
            })) => {}
            other => panic!("expected a MaxWidth overflow error, got {other:?}"),
        }
    }

    #[test]
    fn track_spec_from_quality_level_text_unsupported() {
        let quality = QualityLevel::new("TTML".to_string(), 1, Vec::new());
        assert!(matches!(
            track_spec_from_quality_level(TrackId::new(3), 1000, StreamType::Text, &quality),
            Err(crate::error::Error::UnsupportedCodec { .. })
        ));
    }

    #[test]
    fn track_spec_from_quality_level_video_no_sps_errors_not_panics() {
        let quality = QualityLevel::new("H264".to_string(), 1, Vec::new());
        assert!(matches!(
            track_spec_from_quality_level(TrackId::new(1), 90_000, StreamType::Video, &quality),
            Err(crate::error::Error::InvalidInput(..))
        ));
    }
}
