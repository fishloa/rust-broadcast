//! Parse an RTSP DESCRIBE SDP body into per-track init (codec config + clock
//! rate + control URL + assigned interleaved channel).

use crate::error::{MultimuxError, Result};
use crate::source::TrackInit;
use rtsp_runtime::{MAX_TRACKS, PlayPlan};
use sdp_types::{Media, Session};
use transmux::TrackId;
use transmux::sdp_track;
use url::Url;

/// First interleaved RTP channel of an RTSP pull.
const FIRST_CHANNEL: u8 = 0;

/// Interleaved channels are assigned in steps of 2 (RTP even, RTCP = +1).
const CHANNEL_STEP: u8 = 2;

/// Builds one media section's [`TrackInit`] (channel 0; the caller assigns the real one).
fn track_init(media_index: usize, media: &Media) -> Result<TrackInit> {
    let track_id = u32::try_from(media_index).map_or(u32::MAX, |i| i.saturating_add(1));
    let track = sdp_track(media).ok_or_else(|| MultimuxError::Sdp {
        reason: format!("media {:?} has no playable payload type", media.media),
    })?;
    let payload_type = track.payload_type.ok_or_else(|| MultimuxError::Sdp {
        reason: format!(
            "media {:?} has no payload type in its fmt field",
            media.media
        ),
    })?;
    Ok(TrackInit {
        track_id: TrackId::new(track_id),
        kind: track.kind,
        config: track.config,
        clock_rate: track.clock_rate,
        control: track.control,
        channel: 0,
        payload_type,
    })
}

/// Media sections that can be played, in media order, with the track cap applied.
/// Skipped sections are logged here with their reason, exactly once.
fn playable_media(session: &Session) -> Vec<(usize, TrackInit)> {
    let mut out = Vec::new();
    for (media_index, media) in session.medias.iter().enumerate() {
        if out.len() >= MAX_TRACKS {
            tracing::warn!(
                media_index,
                "rtsp: skipped a media section beyond the track cap"
            );
            continue;
        }
        match track_init(media_index, media) {
            Ok(track) => out.push((media_index, track)),
            Err(e) => tracing::warn!(
                error = %e,
                media_index,
                "rtsp: skipped an unplayable media section"
            ),
        }
    }
    out
}

/// Parse the DESCRIBE SDP body into per-track init, assigning interleaved
/// channels 0,2,4,… in media order.
pub fn parse_tracks_multimux(sdp: &[u8]) -> Result<Vec<TrackInit>> {
    let session = Session::parse(sdp).map_err(|e| MultimuxError::Sdp {
        reason: format!("parse: {e}"),
    })?;
    let playable = playable_media(&session);
    if playable.is_empty() {
        return Err(MultimuxError::Sdp {
            reason: "SDP has no supported media".into(),
        });
    }
    let mut tracks = Vec::new();
    let mut channel = 0u8;
    for (_, mut track) in playable {
        track.channel = channel;
        tracks.push(track);
        channel = channel.saturating_add(CHANNEL_STEP);
    }
    Ok(tracks)
}

/// Plans the RTSP `SETUP`/`PLAY` for a DESCRIBE body: every media section
/// must be a supported codec (any failure is the whole call's error), and a
/// control URL on another authority is an error rather than a skipped track.
/// Relative controls resolve against `content_base` (the DESCRIBE reply's
/// `Content-Base`, else `Content-Location`), else `request_url` (RFC 2326
/// Appendix C.1.1).
pub(crate) fn plan_tracks(
    request_url: &Url,
    content_base: Option<&str>,
    sdp: &[u8],
) -> Result<(PlayPlan, Vec<TrackInit>)> {
    let text = std::str::from_utf8(sdp).map_err(|e| MultimuxError::Sdp {
        reason: format!("parse: {e}"),
    })?;
    struct Candidate {
        media_index: usize,
        track: TrackInit,
    }
    let playable =
        playable_media(
            &Session::parse(text.as_bytes()).map_err(|e| MultimuxError::Sdp {
                reason: format!("parse: {e}"),
            })?,
        );
    let playable_indices: std::collections::BTreeSet<usize> =
        playable.iter().map(|(i, _)| *i).collect();
    let candidates: Vec<Candidate> = playable
        .into_iter()
        .map(|(media_index, track)| Candidate { media_index, track })
        .collect();
    let plan = PlayPlan::from_describe_selecting(
        request_url,
        content_base,
        None,
        text,
        FIRST_CHANNEL,
        usize::MAX,
        |media_index, _| playable_indices.contains(&media_index),
    )
    .map_err(|e| MultimuxError::Sdp {
        reason: match e {
            rtsp_runtime::Error::InvalidSdp(reason) => format!("parse: {reason}"),
            other => other.to_string(),
        },
    })?;
    for skipped in plan.skipped() {
        tracing::warn!(
            media_index = skipped.media_index,
            control = %skipped.control,
            "rtsp: skipped a media section whose control is on another authority"
        );
    }
    for media_index in plan.over_limit() {
        tracing::warn!(
            media_index,
            "rtsp: skipped a media section beyond the track cap"
        );
    }
    let mut tracks: Vec<TrackInit> = Vec::new();
    for (index, setup) in plan.setups().iter().enumerate() {
        let Some(candidate) = candidates
            .iter()
            .find(|c| c.media_index == setup.media_index)
        else {
            continue;
        };
        let Some((channel, _)) = plan.channels(index) else {
            continue;
        };
        let mut track = candidate.track.clone();
        track.channel = channel;
        tracks.push(track);
    }
    if tracks.is_empty() {
        return Err(MultimuxError::Sdp {
            reason: "SDP has no supported media".into(),
        });
    }
    Ok((plan, tracks))
}

/// Loads an SDP body from `spec`: either the literal inline text, or —
/// prefixed with `@` — a file path read fresh on every call, so an on-disk
/// SDP can be updated between reconnects without a process restart. Used by
/// [`crate::source::rtp_udp::RtpUdpDialer`]'s `dial()`, the raw-RTP-over-UDP
/// ingest source that has no RTSP DESCRIBE to fetch its SDP from.
pub fn load_sdp(spec: &str) -> Result<Vec<u8>> {
    match spec.strip_prefix('@') {
        Some(path) => std::fs::read(path).map_err(|e| MultimuxError::Sdp {
            reason: format!("failed to read SDP file {path:?}: {e}"),
        }),
        None => Ok(spec.as_bytes().to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::rtp::RtpMediaKind;

    // Minimal H.264 SDP (one video media) with a real sprop-parameter-sets
    // (SPS: nal_unit_type 7, 18 bytes; PPS: nal_unit_type 8, 4 bytes).
    const SDP: &[u8] = b"v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 0 RTP/AVP 96\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\n\
a=control:streamid=0\r\n";

    #[test]
    fn parses_h264_video_track() {
        let tracks = parse_tracks_multimux(SDP).unwrap();
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert!(matches!(t.kind, RtpMediaKind::H264));
        assert_eq!(t.clock_rate, 90_000);
        assert_eq!(t.channel, 0, "first media gets RTP channel 0");
        assert_eq!(t.control.as_deref(), Some("streamid=0"));
        assert_eq!(t.track_id, TrackId::new(1));
        assert_eq!(
            t.payload_type, 96,
            "the m=video line's fmt payload type must be captured"
        );
    }

    #[test]
    fn accepted_media_indices_match_shared_golden() {
        let mut expected = std::collections::BTreeMap::new();
        let raw = std::fs::read_to_string(test_fixtures::path(
            "golden/sdp-tracks/accepted-indices.txt",
        ))
        .unwrap();
        for line in raw.lines() {
            if let Some((name, idx)) = line.split_once(": ") {
                expected.insert(name.to_string(), idx.to_string());
            }
        }
        let url = url::Url::parse("rtsp://cam.test/live/").unwrap();
        for (name, body) in test_fixtures::sdp_track_fixtures() {
            let got = match plan_tracks(&url, None, &body) {
                Ok((plan, _)) => plan
                    .setups()
                    .iter()
                    .map(|s| s.media_index.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                Err(_) => "NONE".to_string(),
            };
            let exp = expected.get(&name).cloned().unwrap_or_default();
            assert_eq!(got, exp, "{name}: accepted indices differ");
        }
    }

    #[test]
    fn load_sdp_returns_inline_text_verbatim() {
        let loaded = load_sdp(std::str::from_utf8(SDP).unwrap()).unwrap();
        assert_eq!(loaded, SDP);
    }

    /// Biting test: `@path` must read the file's bytes, not treat the whole
    /// `@path` string as literal SDP text (which would then fail to parse).
    #[test]
    fn load_sdp_at_prefix_reads_file() {
        let dir = std::env::temp_dir().join(format!("multimux-sdp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sdp");
        std::fs::write(&path, SDP).unwrap();
        let loaded = load_sdp(&format!("@{}", path.display())).unwrap();
        assert_eq!(loaded, SDP);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_sdp_at_prefix_missing_file_errors() {
        let err = load_sdp("@/no/such/path/multimux-missing.sdp");
        assert!(matches!(err, Err(crate::error::MultimuxError::Sdp { .. })));
    }

    #[test]
    fn rejects_sdp_without_media() {
        let sdp = b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
        assert!(matches!(
            parse_tracks_multimux(sdp),
            Err(crate::error::MultimuxError::Sdp { .. })
        ));
    }

    fn plan_for(sdp: &[u8]) -> Result<(PlayPlan, Vec<TrackInit>)> {
        plan_tracks(&Url::parse("rtsp://cam.example/test").unwrap(), None, sdp)
    }

    #[test]
    fn plan_resolves_controls_beneath_the_request_url() {
        let (plan, tracks) = plan_for(SDP).unwrap();
        assert_eq!(
            plan.setups()[0].control,
            "rtsp://cam.example/test/streamid=0"
        );
        assert_eq!(plan.play_uri(), "rtsp://cam.example/test");
        assert_eq!(tracks[0].channel, 0);
    }

    #[test]
    fn plan_skips_a_control_on_another_authority() {
        let sdp = String::from_utf8(SDP.to_vec())
            .unwrap()
            .replace("streamid=0", "rtsp://other.example/x");
        assert!(matches!(
            plan_for(sdp.as_bytes()),
            Err(crate::error::MultimuxError::Sdp { .. })
        ));
    }

    #[test]
    fn plan_skips_an_unsupported_media_instead_of_failing_the_call() {
        let sdp = String::from_utf8(SDP.to_vec()).unwrap();
        let sdp = sdp.replace("m=video", "m=text");
        assert!(matches!(
            plan_for(sdp.as_bytes()),
            Err(crate::error::MultimuxError::Sdp { .. })
        ));
    }

    #[test]
    fn plan_skips_leading_unsupported_then_plans_h264() {
        let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=text 0 RTP/AVP 96\r\na=control:streamid=meta\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1; sprop-parameter-sets=Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==\r\na=control:streamid=0\r\n";
        let (plan, tracks) = plan_for(sdp.as_bytes()).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].channel, 0);
        assert_eq!(plan.setups().len(), 1);
    }

    #[test]
    fn plan_resolves_controls_against_the_content_base() {
        let (plan, _) = plan_tracks(
            &Url::parse("rtsp://cam.example/test").unwrap(),
            Some("rtsp://cam.example/base/"),
            SDP,
        )
        .unwrap();
        assert_eq!(
            plan.setups()[0].control,
            "rtsp://cam.example/base/streamid=0"
        );
    }

    mod golden {
        use super::super::*;
        use std::fmt::Write;

        const GOLDEN: &str = "golden/sdp-tracks/multimux.txt";
        const REGENERATE_ENV: &str = "REGENERATE_SDP_TRACKS_GOLDEN";
        const REQUEST_URL: &str = "rtsp://cam.test/live/";

        fn describe<T: std::fmt::Debug>(result: &Result<T>) -> String {
            match result {
                Ok(value) => format!("{value:?}"),
                Err(e) => format!("error {e}"),
            }
        }

        fn render(name: &str, sdp: &[u8]) -> String {
            let mut out = format!("== {name}\n");
            let tracks = parse_tracks_multimux(sdp);
            writeln!(out, "udp {}", describe(&tracks)).unwrap();
            let request_url = Url::parse(REQUEST_URL).unwrap();
            let planned = plan_tracks(&request_url, None, sdp).map(|(_, tracks)| tracks);
            writeln!(out, "rtsp {}", describe(&planned)).unwrap();
            out
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
}
