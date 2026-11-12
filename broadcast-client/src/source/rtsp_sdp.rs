//! DESCRIBE SDP to per-track init (RFC 4566 media sections, RFC 6184 / RFC 3640 fmtp).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rtsp_runtime::{MAX_TRACKS, PlayPlan};
use sdp_types::Media;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::{SdpTrack, TrackId, first_encoding_name, is_played_encoding, sdp_track};
use url::Url;

const FIRST_CHANNEL: u8 = 0;


pub(crate) struct ParsedSdp {
    pub(crate) plan: PlayPlan,
    pub(crate) tracks: Vec<TrackInit>,
    pub(crate) skipped: Vec<SkippedMedia>,
}

/// A media section the client does not play, by its position among all `m=` lines. `codec` is the
/// first listed payload's `a=rtpmap` encoding name; `codec_unsupported` is set when that names a
/// codec other than the ones played (as opposed to a played codec with an unusable `a=fmtp`, or a
/// section that names no codec).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedMedia {
    pub(crate) media_index: usize,
    pub(crate) codec: Option<String>,
    pub(crate) codec_unsupported: bool,
}

pub(crate) enum SdpError {
    Unparsable,
    NothingPlayable { skipped: Vec<SkippedMedia> },
}

pub(crate) struct TrackInit {
    pub(crate) track_id: TrackId,
    pub(crate) kind: RtpMediaKind,
    pub(crate) config: CodecConfig,
    pub(crate) clock_rate: u32,
    pub(crate) setup_uri: String,
    pub(crate) channel: u8,
    pub(crate) rtcp_channel: u8,
}

/// Parses the SDP into the supported (H.264 video, AAC audio) tracks in media order, at most
/// [`MAX_TRACKS`], planned through [`PlayPlan`] (interleaved RTP channels 0, 2, 4, ..., RTCP the
/// RTP channel plus one, and the aggregate control URL of RFC 2326 C.1.1). A media section the
/// client cannot play (another codec or media type, a missing or unusable fmtp), whose
/// `a=control` is on another authority, or that does not fit the track budget is reported in
/// [`ParsedSdp::skipped`]; no playable section left is [`SdpError::NothingPlayable`], and an SDP
/// that does not parse is [`SdpError::Unparsable`]. Of the payload types a media line lists, the
/// first one whose own `a=rtpmap` and `a=fmtp` describe a supported codec is the one played.
pub(crate) fn parse_tracks(
    sdp: &[u8],
    request_url: &Url,
    content_base: Option<&str>,
) -> Result<ParsedSdp, SdpError> {
    let text = core::str::from_utf8(sdp).map_err(|_| SdpError::Unparsable)?;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut described: Vec<SkippedMedia> = Vec::new();
    let mut skipped: Vec<SkippedMedia> = Vec::new();
    let plan = PlayPlan::from_describe_selecting(
        request_url,
        content_base,
        None,
        text,
        FIRST_CHANNEL,
        MAX_TRACKS,
        |media_index, media| {
            let info = skipped_media(media_index, media);
            described.push(info.clone());
            match sdp_track(media) {
                Some(track) if track.payload_type.is_some() => {
                    candidates.push(Candidate { media_index, track });
                    true
                }
                Some(_) => {
                    skipped.push(info);
                    false
                }
                None => {
                    skipped.push(info);
                    false
                }
            }
        },
    )
    .map_err(|_| SdpError::Unparsable)?;
    skipped.extend(
        plan.skipped()
            .iter()
            .map(|m| m.media_index)
            .chain(plan.over_limit().iter().copied())
            .filter_map(|index| described.get(index).cloned()),
    );
    skipped.sort_by_key(|m| m.media_index);
    let mut tracks: Vec<TrackInit> = Vec::new();
    for (index, setup) in plan.setups().iter().enumerate() {
        let (Some(candidate), Some((channel, rtcp_channel))) = (
            candidates
                .iter()
                .find(|c| c.media_index == setup.media_index),
            plan.channels(index),
        ) else {
            continue;
        };
        let ordinal = u32::try_from(tracks.len().saturating_add(1)).unwrap_or(u32::MAX);
        tracks.push(TrackInit {
            track_id: TrackId::new(ordinal),
            kind: candidate.track.kind,
            config: candidate.track.config.clone(),
            clock_rate: candidate.track.clock_rate,
            setup_uri: setup.control.clone(),
            channel,
            rtcp_channel,
        });
    }
    if tracks.is_empty() {
        return Err(SdpError::NothingPlayable { skipped });
    }
    Ok(ParsedSdp {
        plan,
        tracks,
        skipped,
    })
}

struct Candidate {
    media_index: usize,
    track: SdpTrack,
}

fn skipped_media(media_index: usize, media: &Media) -> SkippedMedia {
    let codec = first_encoding_name(media).map(str::to_string);
    let codec_unsupported = codec
        .as_deref()
        .is_some_and(|name| !is_played_encoding(&media.media, name));
    SkippedMedia {
        media_index,
        codec,
        codec_unsupported,
    }
}

#[cfg(test)]
mod golden {
    use super::*;
    use core::fmt::Write;

    const GOLDEN: &str = "golden/sdp-tracks/broadcast-client.txt";
    const REGENERATE_ENV: &str = "REGENERATE_SDP_TRACKS_GOLDEN";
    const REQUEST_URL: &str = "rtsp://cam.test/live/";

    fn render(name: &str, sdp: &[u8]) -> String {
        let mut out = format!("== {name}\n");
        let request_url = Url::parse(REQUEST_URL).unwrap();
        match parse_tracks(sdp, &request_url, None) {
            Err(SdpError::Unparsable) => out.push_str("unparsable\n"),
            Err(SdpError::NothingPlayable { skipped }) => {
                writeln!(out, "nothing-playable skipped={skipped:?}").unwrap();
            }
            Ok(parsed) => {
                for t in &parsed.tracks {
                    writeln!(
                        out,
                        "track id={} kind={:?} clock={} setup={} channels={}-{} config={:?}",
                        t.track_id.value(),
                        t.kind,
                        t.clock_rate,
                        t.setup_uri,
                        t.channel,
                        t.rtcp_channel,
                        t.config
                    )
                    .unwrap();
                }
                writeln!(out, "skipped={:?}", parsed.skipped).unwrap();
            }
        }
        out
    }

    #[test]
    fn accepted_media_indices_match_shared_golden() {
        let mut expected = std::collections::BTreeMap::new();
        let raw = std::fs::read_to_string(test_fixtures::path("golden/sdp-tracks/accepted-indices.txt")).unwrap();
        for line in raw.lines() {
            if let Some((name, idx)) = line.split_once(": ") {
                expected.insert(name.to_string(), idx.to_string());
            }
        }
        let request_url = url::Url::parse("rtsp://cam.test/live/").unwrap();
        for (name, body) in test_fixtures::sdp_track_fixtures() {
            let media_count = sdp_types::Session::parse(&body)
                .map(|s| s.medias.len())
                .unwrap_or(0);
            let mut skipped_indices = std::collections::BTreeSet::new();
            let got = match parse_tracks(&body, &request_url, None) {
                Ok(parsed) => {
                    for s in &parsed.skipped {
                        skipped_indices.insert(s.media_index);
                    }
                    (0..media_count)
                        .filter(|i| !skipped_indices.contains(i))
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                }
                Err(SdpError::NothingPlayable { skipped }) => {
                    for s in &skipped {
                        skipped_indices.insert(s.media_index);
                    }
                    "NONE".to_string()
                }
                Err(_) => "NONE".to_string(),
            };
            // For NothingPlayable all media skipped; accepted set empty -> NONE.
            let got = if got.is_empty() { "NONE".to_string() } else { got };
            let exp = expected.get(&name).cloned().unwrap_or_default();
            assert_eq!(got, exp, "{name}: accepted indices differ");
        }
    }

    #[test]
    fn every_sdp_fixture_matches_the_recorded_interpretation() {
        let rendered: String = test_fixtures::sdp_track_fixtures()
            .iter()
            .map(|(name, body)| render(name, body))
            .collect();
        let path = test_fixtures::path(GOLDEN);
        if std::env::var(REGENERATE_ENV).ok().as_deref() == Some("1") {
            assert!(std::env::var("CI").is_err(), "refuse to regenerate in CI");
            std::fs::write(&path, &rendered).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap();
        assert_eq!(rendered, expected, "{GOLDEN} differs");
    }
}
