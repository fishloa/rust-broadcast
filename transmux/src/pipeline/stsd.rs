//! The per-codec `stsd` entry and track kind of a fragmented-init `trak`.

use super::*;

pub(super) enum MediaKind {
    Video,
    Audio,
    Subtitle {
        media_header: [u8; 4],
        handler: [u8; 4],
        name: &'static [u8],
    },
}

impl MediaKind {
    pub(super) fn vmhd(&self) -> Option<VideoMediaHeaderBox> {
        matches!(self, Self::Video).then_some(VideoMediaHeaderBox {
            version: 0,
            flags: 1,
            graphicsmode: 0,
            opcolor: [0, 0, 0],
        })
    }

    pub(super) fn smhd(&self) -> Option<SoundMediaHeaderBox> {
        matches!(self, Self::Audio).then_some(SoundMediaHeaderBox {
            version: 0,
            flags: 0,
            balance: 0,
        })
    }

    pub(super) fn handler_type(&self) -> [u8; 4] {
        match self {
            Self::Video => *b"vide",
            Self::Audio => *b"soun",
            Self::Subtitle { handler, .. } => *handler,
        }
    }

    pub(super) fn handler_name(&self) -> Vec<u8> {
        match self {
            Self::Video => b"VideoHandler\0".to_vec(),
            Self::Audio => b"SoundHandler\0".to_vec(),
            Self::Subtitle { name, .. } => name.to_vec(),
        }
    }

    /// The headerless FullBox (version(8)+flags(24)) kept as the first `minf`
    /// child of a subtitle track: `sthd` for `stpp`/`subt` (ISO/IEC 14496-12
    /// §12.6.2), `nmhd` for `wvtt`/`text` (§8.4.5.5), see §6.2.3.
    pub(super) fn minf_header(&self) -> Option<OpaqueBox> {
        match self {
            Self::Subtitle { media_header, .. } => {
                Some(OpaqueBox::new(*media_header, vec![0u8; 4]))
            }
            _ => None,
        }
    }
}

pub(super) struct TrackMedia {
    pub(super) entry: SampleEntryVariant,
    pub(super) kind: MediaKind,
    pub(super) tkhd_w: u32,
    pub(super) tkhd_h: u32,
}

impl TrackMedia {
    fn video(entry: SampleEntryVariant, tkhd_w: u32, tkhd_h: u32) -> Self {
        Self {
            entry,
            kind: MediaKind::Video,
            tkhd_w,
            tkhd_h,
        }
    }

    fn audio(entry: SampleEntryVariant) -> Self {
        Self {
            entry,
            kind: MediaKind::Audio,
            tkhd_w: 0,
            tkhd_h: 0,
        }
    }
}

/// The sample entry, media kind and `tkhd` size of one track's codec.
pub(super) fn track_media(t: &TrackSpec) -> Result<TrackMedia> {
    let tkhd_video_dims = match &t.config {
        CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
        CodecConfig::Hevc { width, height, .. } => Some((*width, *height)),
        _ => None,
    };
    let (tkhd_w_pre, tkhd_h_pre) = match tkhd_video_dims {
        Some((w, h)) => tkhd_presentation_size(t.display.as_ref(), w, h),
        None => (0u32, 0u32),
    };

    let media = match &t.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Avc1(AVCSampleEntry {
                codec_type: *b"avc1",
                visual,
                config: config.clone(),
                extra_boxes: avc_hevc_display_extra_boxes(
                    t.display.as_ref(),
                    t.hdr.as_ref(),
                    None,
                )?,
            });
            TrackMedia::video(entry, tkhd_w_pre, tkhd_h_pre)
        }
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // `hvc1` (parameter sets in the sample entry) — ISO/IEC
            // 14496-15:2017 §8.4.1; `dvh1` for a Dolby Vision base layer that
            // is not SDR/HDR compliant
            // (private/specs-verified/dolby/vision/dolby-vision-streams-in-isobmff-v2.0-2017-12.md 8.1.2).
            let dovi = t.dovi.as_ref().filter(|d| d.is_single_layer());
            let codec_type = if dovi.is_some_and(|d| d.uses_dolby_hevc_entry()) {
                crate::dovi::DVH1_FOURCC
            } else {
                *b"hvc1"
            };
            let entry = SampleEntryVariant::Hevc1(HEVCSampleEntry {
                codec_type,
                visual,
                config: config.clone(),
                extra_boxes: avc_hevc_display_extra_boxes(
                    t.display.as_ref(),
                    t.hdr.as_ref(),
                    dovi,
                )?,
            });
            TrackMedia::video(entry, tkhd_w_pre, tkhd_h_pre)
        }
        CodecConfig::Vvc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // Always emit `vvc1` (parameter sets in the sample entry) —
            // ISO/IEC 14496-15:2022 §11.3.3.
            let entry = SampleEntryVariant::Vvc(Box::new(VVCSampleEntry {
                codec_type: *b"vvc1",
                visual,
                config: config.clone(),
                extra_boxes: vec![],
            }));
            TrackMedia::video(entry, (*width as u32) << 16, (*height as u32) << 16)
        }
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![esds_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry {
                codec_type: *b"mp4a",
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Ac3 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut dac3_full = vec![0u8; config.serialized_len() + 8];
            dac3_full[4..8].copy_from_slice(b"dac3");
            let n = config.serialize_into(&mut dac3_full[8..])?;
            let dac3_opaque = OpaqueBox::new(*b"dac3", dac3_full[8..8 + n].to_vec());
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![dac3_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Ac3(Box::new(Ac3SampleEntry {
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Eac3 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut dec3_full = vec![0u8; config.serialized_len() + 8];
            dec3_full[4..8].copy_from_slice(b"dec3");
            let n = config.serialize_into(&mut dec3_full[8..])?;
            let dec3_opaque = OpaqueBox::new(*b"dec3", dec3_full[8..8 + n].to_vec());
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![dec3_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Ec3(Box::new(Ec3SampleEntry {
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Av1 {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Av01(Box::new(Av1SampleEntry {
                visual,
                config: config.clone(),
                children: alloc::vec![crate::sample_entries::SampleEntryChild::Config],
            }));
            TrackMedia::video(entry, (*width as u32) << 16, (*height as u32) << 16)
        }
        CodecConfig::Vp9 {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Vp09(Box::new(Vp9SampleEntry {
                visual,
                config: config.clone(),
                children: alloc::vec![crate::sample_entries::SampleEntryChild::Config],
            }));
            TrackMedia::video(entry, (*width as u32) << 16, (*height as u32) << 16)
        }
        CodecConfig::Opus {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dops_opaque = config_box(&DOPS_FOURCC, config)?;
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![dops_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Opus(Box::new(OpusSampleEntry {
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Flac {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dfla_opaque = config_box(&DFLA_FOURCC, config)?;
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![dfla_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Flac(Box::new(FlacSampleEntry {
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Ac4 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dac4_opaque = config_box(&DAC4_FOURCC, config)?;
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![dac4_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Ac4(Box::new(Ac4SampleEntry {
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::MpegH {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mhac_opaque = config_box(&MHAC_FOURCC, config)?;
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![mhac_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Mha(Box::new(MhaSampleEntry {
                codec_type: crate::mpegh::MHA1_FOURCC,
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Mpeg2Video {
            esds,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // Carry the esds as the mp4v's first child box (body only, no header).
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let entry = SampleEntryVariant::Mp4v(Box::new(Mp4vSampleEntry {
                visual,
                config_boxes: vec![esds_opaque],
            }));
            TrackMedia::video(entry, (*width as u32) << 16, (*height as u32) << 16)
        }
        CodecConfig::MpegAudio {
            esds,
            layer: _,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![esds_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry {
                codec_type: *b"mp4a",
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        CodecConfig::Dts {
            config,
            codec_fourcc,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let ddts_opaque = config_box(&DDTS_FOURCC, config)?;
            let rate = audio_rate_carriage(*sample_rate);
            let mut config_boxes = vec![ddts_opaque];
            if let Some(srat) = rate.srat {
                config_boxes.push(srat_opaque(srat)?);
            }
            let entry = SampleEntryVariant::Dts(Box::new(DtsSampleEntry {
                codec_type: *codec_fourcc,
                entry_version: rate.entry_version,
                reserved_1: [0u8; 6],
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                compression_id_and_packet_size: [0u8; 4],
                samplerate: rate.samplerate_field,
                config_boxes,
            }));
            TrackMedia::audio(entry)
        }
        // WebM-native codecs: carried in the IR (see [`crate::webm_demux`]) but
        // with no ISOBMFF sample entry in this crate — the fMP4 mux path is out
        // of scope (see `private/specs-verified/mpeg/transmux/codec/vp8-vorbis-webm.md`).
        CodecConfig::Vp8 { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "VP8" });
        }
        CodecConfig::Vorbis { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "Vorbis" });
        }
        // Opaque PES data track: no ISOBMFF sample entry in this crate — the
        // fMP4 mux path is out of scope (mirrors Vp8/Vorbis above).
        CodecConfig::Data { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "Data" });
        }
        // Subtitle track: re-emit the ISOBMFF sample entry carried whole in
        // `sample_entry` (issue #1186, ex-`TODO(#753)`). The media header is
        // `sthd` for `stpp`/`subt` (ISO/IEC 14496-12 §12.6.2, as in the real
        // ffmpeg fixture) and `nmhd` for `wvtt`/`text` (§8.4.5.5); both are
        // headerless FullBoxes, preserved as the first `minf` child (§6.2.3).
        CodecConfig::Subtitle {
            sample_entry: Some(se),
            ..
        } => {
            let (entry, media_header, handler, name): (_, &[u8; 4], &[u8; 4], &[u8]) = match se {
                SubtitleSampleEntry::Ttml(e) => (
                    SampleEntryVariant::Stpp(Box::new(e.clone())),
                    b"sthd",
                    b"subt",
                    b"SubtitleHandler\0",
                ),
                SubtitleSampleEntry::WebVtt(e) => (
                    SampleEntryVariant::Wvtt(Box::new(e.clone())),
                    b"nmhd",
                    b"text",
                    b"TextHandler\0",
                ),
            };
            TrackMedia {
                entry,
                kind: MediaKind::Subtitle {
                    media_header: *media_header,
                    handler: *handler,
                    name,
                },
                tkhd_w: 0,
                tkhd_h: 0,
            }
        }
        CodecConfig::Subtitle {
            sample_entry: None, ..
        } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "Subtitle" });
        }
    };
    Ok(media)
}
