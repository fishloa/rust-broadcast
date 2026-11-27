//! Operator-declared HE-AAC profiles (issue #1428).
//!
//! An ADTS header carries only the AAC-LC core, and the SBR/PS data sits
//! inside the raw data block, which this crate never decodes. A route that
//! knows its audio is HE-AAC says so with [`AudioProfileOverride`]s; the
//! matching tracks then carry explicit hierarchical signalling
//! ([`TrackSpec::with_explicit_he_aac`], ISO/IEC 14496-3 §1.6.2), so the fMP4
//! init's `AudioSpecificConfig` and the HLS `CODECS` agree with the stream.
//! A declaration wins over the PMT `AAC_descriptor`.

use alloc::vec::Vec;

use broadcast_bitstream::aac_asc::HeAacProfile;

use crate::{Error, TrackId, TrackSpec};

/// Which source audio track a declaration or a separate audio rendition picks
/// (issue #1428 slice 2b). `Index` is the zero-based ordinal among the
/// program's fMP4-muxable audio tracks in the order the source announced them;
/// `TrackId` is the source track id itself.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AudioTrackSelector {
    /// Ordinal among the program's audio tracks.
    Index(u32),
    /// Source track id.
    TrackId(u32),
}

impl AudioTrackSelector {
    /// The position this selector picks among `audio`, the ids of a program's
    /// fMP4-muxable audio tracks in the order the source announced them.
    pub fn position(&self, audio: impl IntoIterator<Item = TrackId>) -> Option<usize> {
        let mut audio = audio.into_iter();
        match self {
            AudioTrackSelector::Index(i) => audio.nth(*i as usize).map(|_| *i as usize),
            AudioTrackSelector::TrackId(id) => audio.position(|t| t == TrackId::new(*id)),
        }
    }

    /// The wire/label string for this selector's kind.
    pub fn name(&self) -> &'static str {
        match self {
            AudioTrackSelector::Index(_) => "index",
            AudioTrackSelector::TrackId(_) => "track_id",
        }
    }
}

broadcast_core::impl_spec_display!(AudioTrackSelector);

/// HE-AAC flavour an operator declares for one audio track of an ADTS source.
/// The ADTS header states only the AAC-LC core, so this crate cannot tell
/// HE-AAC from plain AAC-LC without decoding; this states it instead, and the
/// track's `AudioSpecificConfig` and `CODECS` then carry explicit SBR
/// (`mp4a.40.5`) or SBR+PS (`mp4a.40.29`) signalling, as Apple's HLS authoring
/// rules require.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum AudioProfile {
    /// HE-AAC v1: AAC-LC core + SBR.
    HeAac,
    /// HE-AAC v2: AAC-LC core + SBR + Parametric Stereo.
    HeAacV2,
}

impl AudioProfile {
    /// The wire/label string (the config value).
    pub fn name(&self) -> &'static str {
        match self {
            AudioProfile::HeAac => "he-aac",
            AudioProfile::HeAacV2 => "he-aac-v2",
        }
    }

    /// The explicit-signalling profile this declaration selects.
    pub fn he_aac(self) -> HeAacProfile {
        match self {
            AudioProfile::HeAac => HeAacProfile::V1,
            AudioProfile::HeAacV2 => HeAacProfile::V2,
        }
    }
}

broadcast_core::impl_spec_display!(AudioProfile);

/// One per-track [`AudioProfile`] declaration: `track` picks the audio track
/// exactly as an audio-rendition source does. An override beats whatever the
/// PMT `AAC_descriptor` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct AudioProfileOverride {
    /// The audio track the profile applies to.
    pub track: AudioTrackSelector,
    /// The declared profile.
    pub audio_profile: AudioProfile,
}

/// What [`apply_overrides`] did with a track set.
#[derive(Debug)]
#[non_exhaustive]
pub struct AppliedOverrides {
    /// The tracks, with every applicable declaration applied.
    pub tracks: Vec<TrackSpec>,
    /// Declarations that matched no audio track (yet).
    pub unmatched: Vec<AudioProfileOverride>,
    /// Declarations that matched a track they cannot apply to, with why.
    pub refused: Vec<(AudioProfileOverride, Error)>,
}

/// Apply `overrides` to `tracks`: each declaration selects an fMP4-muxable
/// audio track and gives it explicit HE-AAC signalling.
pub fn apply_overrides(
    overrides: &[AudioProfileOverride],
    mut tracks: Vec<TrackSpec>,
) -> AppliedOverrides {
    let mut unmatched = Vec::new();
    let mut refused = Vec::new();
    for declared in overrides {
        let audio: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.config.is_video() && t.config.is_muxable_in_bmff())
            .map(|(i, _)| i)
            .collect();
        let found = declared
            .track
            .position(audio.iter().map(|&i| tracks[i].track_id))
            .map(|n| audio[n]);
        let Some(at) = found else {
            unmatched.push(*declared);
            continue;
        };
        match tracks[at]
            .clone()
            .with_explicit_he_aac(declared.audio_profile.he_aac())
        {
            Ok(spec) => tracks[at] = spec,
            Err(error) => refused.push((*declared, error)),
        }
    }
    AppliedOverrides {
        tracks,
        unmatched,
        refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CodecConfig, DataCarriage, DataIdentity};
    use broadcast_bitstream::MpegAudioLayer;
    use broadcast_bitstream::codec_config_ext::EsdsBoxCodecInfo;
    use broadcast_core::timescale::Timescale;
    use isobmff::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };

    fn ids(ids: &[u32]) -> impl Iterator<Item = TrackId> + '_ {
        ids.iter().map(|&i| TrackId::new(i))
    }

    fn timescale() -> Timescale {
        Timescale::try_new(44_100).expect("a non-zero timescale")
    }

    fn esds_with_asc(object_type_indication: u8, asc: &[u8]) -> EsdsBox {
        EsdsBox::new(ESDescriptor::new(
            2,
            0,
            Some(DecoderConfigDescriptor::new(
                object_type_indication,
                5,
                false,
                0,
                0,
                0,
                Some(DecoderSpecificInfo::new(asc.to_vec())),
            )),
            Some(SLConfigDescriptor::predefined_two()),
        ))
    }

    fn aac_lc(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(track_id),
            timescale(),
            CodecConfig::Aac {
                esds: esds_with_asc(0x40, &[0x12, 0x10]),
                channel_count: 2,
                sample_rate: 44_100,
                sample_size: 16,
            },
        )
    }

    fn codec_string(track: &TrackSpec) -> alloc::string::String {
        match &track.config {
            CodecConfig::Aac { esds, .. } => esds
                .audio_specific_config()
                .expect("AAC-LC config")
                .rfc6381(),
            other => panic!("expected AAC, got {}", other.name()),
        }
    }

    /// An index picks by ordinal, a track id by identity, and a miss is `None`.
    #[test]
    fn a_selector_picks_by_ordinal_or_by_track_id() {
        let audio = [258, 259, 260];
        assert_eq!(AudioTrackSelector::Index(1).position(ids(&audio)), Some(1));
        assert_eq!(AudioTrackSelector::Index(3).position(ids(&audio)), None);
        assert_eq!(
            AudioTrackSelector::TrackId(260).position(ids(&audio)),
            Some(2)
        );
        assert_eq!(AudioTrackSelector::TrackId(7).position(ids(&audio)), None);
    }

    /// With no audio track, a declaration is reported unmatched and the tracks
    /// come back unchanged.
    #[test]
    fn a_declaration_with_no_audio_track_is_unmatched() {
        let declared = AudioProfileOverride {
            track: AudioTrackSelector::Index(0),
            audio_profile: AudioProfile::HeAac,
        };
        let applied = apply_overrides(&[declared], Vec::new());
        assert!(applied.tracks.is_empty());
        assert_eq!(applied.unmatched, [declared]);
        assert!(applied.refused.is_empty());
    }

    /// `name()` is the config/wire string and the label `Display` renders.
    #[test]
    fn selector_and_profile_labels() {
        for &(value, name) in &[
            (AudioTrackSelector::Index(0), "index"),
            (AudioTrackSelector::TrackId(1), "track_id"),
        ] {
            assert_eq!(value.name(), name);
            assert_eq!(alloc::format!("{value}"), name);
        }
        for &(value, name) in &[
            (AudioProfile::HeAac, "he-aac"),
            (AudioProfile::HeAacV2, "he-aac-v2"),
        ] {
            assert_eq!(value.name(), name);
            assert_eq!(alloc::format!("{value}"), name);
        }
    }

    fn video() -> TrackSpec {
        TrackSpec::new(
            TrackId::new(256),
            timescale(),
            CodecConfig::Vp8 {
                width: 1920,
                height: 1080,
            },
        )
    }

    fn opaque_data() -> TrackSpec {
        TrackSpec::new(
            TrackId::new(259),
            timescale(),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 0x06,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                },
            },
        )
    }

    /// `Index` counts only the fMP4-muxable audio tracks: a video track at
    /// ordinal 0 must not be targeted, and a non-muxable data track is not
    /// audio at all.
    #[test]
    fn overrides_target_muxable_audio_tracks_only() {
        let tracks = || vec![video(), aac_lc(258), opaque_data()];
        let declared = AudioProfileOverride {
            track: AudioTrackSelector::Index(0),
            audio_profile: AudioProfile::HeAac,
        };
        let applied = apply_overrides(&[declared], tracks());
        assert!(applied.unmatched.is_empty());
        assert!(applied.refused.is_empty());
        assert_eq!(applied.tracks[0], video(), "video track untouched");
        assert_eq!(applied.tracks[2], opaque_data(), "data track untouched");
        assert_eq!(codec_string(&applied.tracks[1]), "mp4a.40.5");

        let by_id = AudioProfileOverride {
            track: AudioTrackSelector::TrackId(258),
            audio_profile: AudioProfile::HeAac,
        };
        let applied = apply_overrides(&[by_id], tracks());
        assert!(applied.unmatched.is_empty());
        assert_eq!(applied.tracks[0], video());
        assert_eq!(codec_string(&applied.tracks[1]), "mp4a.40.5");

        let second = AudioProfileOverride {
            track: AudioTrackSelector::Index(1),
            audio_profile: AudioProfile::HeAac,
        };
        let applied = apply_overrides(&[second], tracks());
        assert_eq!(
            applied.unmatched,
            [second],
            "there is only one muxable audio track"
        );
        assert!(applied.refused.is_empty());
    }

    /// A declaration on a muxable audio track that is not AAC-LC is refused,
    /// never silently retargeted at another track.
    #[test]
    fn an_override_on_a_non_aac_audio_track_is_refused() {
        let mpeg = TrackSpec::new(
            TrackId::new(260),
            timescale(),
            CodecConfig::MpegAudio {
                esds: esds_with_asc(0x69, &[]),
                layer: MpegAudioLayer::LayerII,
                channel_count: 2,
                sample_rate: 48_000,
                sample_size: 16,
            },
        );
        let declared = AudioProfileOverride {
            track: AudioTrackSelector::Index(0),
            audio_profile: AudioProfile::HeAac,
        };
        let applied = apply_overrides(&[declared], vec![mpeg.clone()]);
        assert!(applied.unmatched.is_empty());
        assert_eq!(applied.refused.len(), 1, "non-AAC audio is refused");
        assert_eq!(applied.refused[0].0, declared);
        assert_eq!(applied.tracks[0], mpeg, "config unchanged");
    }
}
