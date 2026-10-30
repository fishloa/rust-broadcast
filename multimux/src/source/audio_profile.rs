//! Operator-declared HE-AAC profiles (issue #1428).
//!
//! An ADTS header carries only the AAC-LC core, and the SBR/PS data sits
//! inside the raw data block, which this crate never decodes. A route that
//! knows its audio is HE-AAC says so with `audio_profiles`; the matching
//! tracks then carry explicit hierarchical signalling
//! ([`transmux::TrackSpec::with_explicit_he_aac`], ISO/IEC 14496-3 §1.6.2), so
//! the fMP4 init's `AudioSpecificConfig` and the HLS `CODECS` agree with the
//! stream. The declaration is applied wherever a track set leaves the `Trunk`
//! for an init segment or a master playlist, and wins over the PMT
//! `AAC_descriptor`.

use transmux::TrackSpec;

use crate::config::{AudioProfileOverride, AudioTrackSelector};

pub(crate) fn apply(
    route: &str,
    overrides: &[AudioProfileOverride],
    mut tracks: Vec<TrackSpec>,
) -> Vec<TrackSpec> {
    for declared in overrides {
        let audio: Vec<usize> = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                !crate::push::egress::is_video(&t.config) && t.config.is_muxable_in_bmff()
            })
            .map(|(i, _)| i)
            .collect();
        let found = match declared.track {
            AudioTrackSelector::Index(i) => audio.get(i as usize).copied(),
            AudioTrackSelector::TrackId(id) => audio
                .iter()
                .copied()
                .find(|&i| tracks[i].track_id == transmux::TrackId::new(id)),
        };
        let Some(at) = found else {
            tracing::warn!(
                route,
                selector = %declared.track,
                "audio_profiles entry matches no audio track; ignoring until one appears"
            );
            continue;
        };
        match tracks[at]
            .clone()
            .with_explicit_he_aac(declared.audio_profile.he_aac())
        {
            Ok(spec) => tracks[at] = spec,
            Err(error) => tracing::warn!(
                route,
                selector = %declared.track,
                profile = %declared.audio_profile,
                %error,
                "audio_profiles entry cannot be applied to this track; leaving it as the source states it"
            ),
        }
    }
    tracks
}
