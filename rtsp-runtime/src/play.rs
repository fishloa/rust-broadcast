//! Sans-IO plan for pulling a stream: `DESCRIBE` body to `SETUP` list to `PLAY` URI
//! (RFC 2326 §10.2, §10.4, §10.5, Appendix C.1.1, §12.39).
//!
//! [`PlayPlan`] holds no socket and reads no clock. A driver (the tokio
//! [`AsyncRtspClient::start_play`](crate::AsyncRtspClient::start_play), or a
//! sans-IO host such as `broadcast-client`) plans from the `DESCRIBE` answer,
//! sends each [`PlannedSetup`] in order, hands every `SETUP` answer's
//! negotiated `Transport` to [`PlayPlan::accept_setup`], then sends `PLAY` to
//! [`PlayPlan::play_uri`].

use crate::transport::{LowerTransport, Transport, TransportSpec};
use crate::{Error, Result};

/// SDP attribute naming a track's or the session's control URL (RFC 2326 Appendix C.1.1).
const CONTROL_ATTRIBUTE: &str = "control";

/// Interleaved channels one track consumes: RTP and RTCP (RFC 2326 §12.39).
const CHANNELS_PER_TRACK: u8 = 2;

/// A track negotiated by a play flow.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlayTrack {
    /// The SETUP URI used for this track.
    pub control: String,
    /// The interleaved RTP channel negotiated for this track.
    pub channel: u8,
}

/// Result of a successful play flow.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlaySession {
    /// The SDP body returned by `DESCRIBE` (RFC 2326 §10.2).
    pub sdp: String,
    /// The tracks negotiated by the flow.
    pub tracks: Vec<PlayTrack>,
    /// Control URLs skipped because they crossed authorities.
    pub skipped: Vec<String>,
}

/// One `SETUP` the driver must send, in order.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlannedSetup {
    /// Position of the media section among all `m=` lines of the SDP.
    pub media_index: usize,
    /// The absolute `SETUP` URI (RFC 2326 Appendix C.1.1).
    pub control: String,
    /// The `Transport` to offer: interleaved TCP on this track's channel pair (§12.39).
    pub transport: Transport,
}

/// A media section left out of the plan because its `a=control` resolved to another authority.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SkippedMedia {
    /// Position of the media section among all `m=` lines of the SDP.
    pub media_index: usize,
    /// The `a=control` value as written in the SDP.
    pub control: String,
}

/// The `SETUP`/`PLAY` plan derived from one `DESCRIBE` answer, and the channel
/// bookkeeping of the `SETUP` answers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlayPlan {
    play_uri: String,
    session_control_foreign: bool,
    setups: Vec<PlannedSetup>,
    skipped: Vec<SkippedMedia>,
    channels: Vec<(u8, u8)>,
}

fn control_of(attributes: &[sdp_types::Attribute]) -> Option<&str> {
    attributes
        .iter()
        .find(|a| a.attribute == CONTROL_ATTRIBUTE)
        .and_then(|a| a.value.as_deref())
}

impl PlayPlan {
    /// Plans every media section of `sdp`, refusing a session-level `a=control`
    /// on another authority.
    ///
    /// Relative controls resolve against `content_base`, else
    /// `content_location`, else `request_url` kept with a trailing slash so a
    /// value appends rather than replaces the last path segment (Appendix
    /// C.1.1). Track `n` is offered the interleaved pair `first_channel + 2n` /
    /// `first_channel + 2n + 1`. A media control on another authority is
    /// listed in [`skipped`](Self::skipped) instead of set up. `PLAY` goes to
    /// the session-level control (absent is the request URL, `*` the base).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidSdp`] when `sdp` does not parse,
    /// [`Error::InvalidUrl`] when the session-level control crosses
    /// authorities, and [`Error::InterleavedChannelExhausted`] when the pairs
    /// no longer fit in a `u8`.
    pub fn from_describe(
        request_url: &url::Url,
        content_base: Option<&str>,
        content_location: Option<&str>,
        sdp: &str,
        first_channel: u8,
    ) -> Result<Self> {
        let plan = Self::from_describe_selecting(
            request_url,
            content_base,
            content_location,
            sdp,
            first_channel,
            |_, _| true,
        )?;
        if plan.session_control_foreign {
            return Err(Error::InvalidUrl {
                what: "session control crosses authorities",
            });
        }
        Ok(plan)
    }

    /// Like [`from_describe`](Self::from_describe), but only media sections
    /// `select` accepts (called with the section's position and description,
    /// in order) are planned, and a session-level control on another authority
    /// is not an error: [`play_uri`](Self::play_uri) falls back to
    /// `request_url` and [`session_control_foreign`](Self::session_control_foreign)
    /// reports it. A section `select` rejects appears nowhere in the plan.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidSdp`] and [`Error::InterleavedChannelExhausted`] as for
    /// [`from_describe`](Self::from_describe).
    pub fn from_describe_selecting(
        request_url: &url::Url,
        content_base: Option<&str>,
        content_location: Option<&str>,
        sdp: &str,
        first_channel: u8,
        mut select: impl FnMut(usize, &sdp_types::Media) -> bool,
    ) -> Result<Self> {
        let description = sdp_types::Session::parse(sdp.as_bytes())
            .map_err(|e| Error::InvalidSdp(e.to_string()))?;

        let base = content_base
            .or(content_location)
            .and_then(|b| request_url.join(b).ok())
            .unwrap_or_else(|| {
                let mut base = request_url.clone();
                if !base.path().ends_with('/') {
                    let slashed = format!("{}/", base.path());
                    base.set_path(&slashed);
                }
                base
            });

        let session_play =
            crate::sdp::resolve_control(request_url, &base, control_of(&description.attributes));
        let session_control_foreign = session_play.is_none();
        let play_uri = session_play.unwrap_or_else(|| request_url.to_string());

        let mut setups: Vec<PlannedSetup> = Vec::new();
        let mut skipped: Vec<SkippedMedia> = Vec::new();
        let mut channels: Vec<(u8, u8)> = Vec::new();
        let mut next_low = Some(first_channel);
        for (media_index, media) in description.medias.iter().enumerate() {
            if !select(media_index, media) {
                continue;
            }
            let control = control_of(&media.attributes);
            let Some(resolved) = crate::sdp::resolve_control(request_url, &base, control) else {
                skipped.extend(control.map(|c| SkippedMedia {
                    media_index,
                    control: c.to_string(),
                }));
                continue;
            };
            let low = next_low.ok_or(Error::InterleavedChannelExhausted)?;
            let high = low
                .checked_add(1)
                .ok_or(Error::InterleavedChannelExhausted)?;
            next_low = low.checked_add(CHANNELS_PER_TRACK);
            channels.push((low, high));
            setups.push(PlannedSetup {
                media_index,
                control: resolved,
                transport: Transport::single(TransportSpec::rtp_avp_tcp_interleaved(low, high)),
            });
        }

        Ok(Self {
            play_uri,
            session_control_foreign,
            setups,
            skipped,
            channels,
        })
    }

    /// The `SETUP` requests to send, in order.
    pub fn setups(&self) -> &[PlannedSetup] {
        &self.setups
    }

    /// Media sections left out because their `a=control` crossed authorities.
    pub fn skipped(&self) -> &[SkippedMedia] {
        &self.skipped
    }

    /// The URI `PLAY` is sent to (RFC 2326 §10.5).
    pub fn play_uri(&self) -> &str {
        &self.play_uri
    }

    /// Whether the session-level `a=control` crossed authorities, so
    /// [`play_uri`](Self::play_uri) fell back to the request URL.
    pub fn session_control_foreign(&self) -> bool {
        self.session_control_foreign
    }

    /// The interleaved `(RTP, RTCP)` channels of setup `index`: the offered
    /// pair until [`accept_setup`](Self::accept_setup) records the server's.
    pub fn channels(&self, index: usize) -> Option<(u8, u8)> {
        self.channels.get(index).copied()
    }

    /// Records the answer to setup `index`. `negotiated` is the `Transport`
    /// the server answered with; `None` (no `Transport` header) keeps the
    /// offered pair.
    ///
    /// # Errors
    ///
    /// [`Error::UnusableTransport`] when the answer is not interleaved TCP with
    /// two distinct channels, and [`Error::InterleavedChannelCollision`] when
    /// either channel is already taken by an earlier setup. `index` out of
    /// range is [`Error::UnusableTransport`] too.
    pub fn accept_setup(&mut self, index: usize, negotiated: Option<&Transport>) -> Result<()> {
        let Some(offered) = self.channels.get(index).copied() else {
            return Err(Error::UnusableTransport);
        };
        let (channel, rtcp_channel) = match negotiated {
            None => offered,
            Some(transport) => transport
                .first()
                .filter(|spec| spec.lower_transport == Some(LowerTransport::Tcp))
                .and_then(|spec| spec.interleaved)
                .filter(|(rtp, rtcp)| rtp != rtcp)
                .ok_or(Error::UnusableTransport)?,
        };
        let taken = self.channels.iter().take(index).find(|(rtp, rtcp)| {
            [rtp, rtcp]
                .iter()
                .any(|c| **c == channel || **c == rtcp_channel)
        });
        if taken.is_some() {
            return Err(Error::InterleavedChannelCollision { channel });
        }
        self.channels[index] = (channel, rtcp_channel);
        Ok(())
    }

    /// The finished session: one [`PlayTrack`] per setup on its accepted
    /// channel, plus the `sdp` the plan came from.
    pub fn into_session(self, sdp: String) -> PlaySession {
        let tracks = self
            .setups
            .into_iter()
            .zip(self.channels)
            .map(|(setup, (channel, _))| PlayTrack {
                control: setup.control,
                channel,
            })
            .collect();
        PlaySession {
            sdp,
            tracks,
            skipped: self.skipped.into_iter().map(|m| m.control).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = "rtsp://cam.example/live/key";

    fn sdp(session_control: Option<&str>, controls: &[&str]) -> String {
        let mut sdp = String::from("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n");
        if let Some(control) = session_control {
            sdp.push_str(&format!("a=control:{control}\r\n"));
        }
        for control in controls {
            sdp.push_str(&format!("m=video 0 RTP/AVP 96\r\na=control:{control}\r\n"));
        }
        sdp
    }

    fn plan(
        content_base: Option<&str>,
        session_control: Option<&str>,
        controls: &[&str],
        first_channel: u8,
    ) -> Result<PlayPlan> {
        PlayPlan::from_describe(
            &url::Url::parse(REQUEST).unwrap(),
            content_base,
            None,
            &sdp(session_control, controls),
            first_channel,
        )
    }

    fn interleaved(transport: &str) -> Transport {
        Transport::parse(transport).unwrap()
    }

    #[test]
    fn two_media_get_consecutive_channel_pairs_and_the_aggregate_play_uri() {
        let plan = plan(None, None, &["trackID=0", "trackID=1"], 0).unwrap();
        let setups: Vec<(&str, Option<(u8, u8)>)> = plan
            .setups()
            .iter()
            .map(|s| (s.control.as_str(), s.transport.first().unwrap().interleaved))
            .collect();
        assert_eq!(
            setups,
            vec![
                ("rtsp://cam.example/live/key/trackID=0", Some((0, 1))),
                ("rtsp://cam.example/live/key/trackID=1", Some((2, 3))),
            ]
        );
        assert_eq!(plan.play_uri(), REQUEST);
        let session = plan.into_session(String::new());
        let channels: Vec<u8> = session.tracks.iter().map(|t| t.channel).collect();
        assert_eq!(channels, vec![0, 2]);
    }

    #[test]
    fn a_foreign_media_control_is_skipped_not_planned() {
        let plan = plan(None, None, &["trackID=0", "rtsp://other.example/x"], 0).unwrap();
        assert_eq!(plan.setups().len(), 1);
        assert_eq!(
            plan.skipped(),
            [SkippedMedia {
                media_index: 1,
                control: "rtsp://other.example/x".into()
            }]
        );
        assert_eq!(
            plan.into_session(String::new()).skipped,
            vec!["rtsp://other.example/x".to_string()]
        );
    }

    #[test]
    fn a_repeated_channel_is_a_collision() {
        let mut plan = plan(None, None, &["trackID=0", "trackID=1"], 0).unwrap();
        let answer = interleaved("RTP/AVP/TCP;unicast;interleaved=0-1");
        plan.accept_setup(0, Some(&answer)).unwrap();
        assert!(matches!(
            plan.accept_setup(1, Some(&answer)),
            Err(Error::InterleavedChannelCollision { channel: 0 })
        ));
    }

    #[test]
    fn an_overlapping_rtcp_channel_is_a_collision() {
        let mut plan = plan(None, None, &["trackID=0", "trackID=1"], 0).unwrap();
        plan.accept_setup(0, Some(&interleaved("RTP/AVP/TCP;interleaved=0-1")))
            .unwrap();
        assert!(matches!(
            plan.accept_setup(1, Some(&interleaved("RTP/AVP/TCP;interleaved=1-4"))),
            Err(Error::InterleavedChannelCollision { channel: 1 })
        ));
    }

    #[test]
    fn a_non_interleaved_answer_is_unusable() {
        let mut plan = plan(None, None, &["trackID=0"], 0).unwrap();
        let udp = interleaved("RTP/AVP;unicast;client_port=5000-5001");
        assert!(matches!(
            plan.accept_setup(0, Some(&udp)),
            Err(Error::UnusableTransport)
        ));
    }

    #[test]
    fn content_base_is_the_base_for_relative_controls() {
        let plan = plan(Some("rtsp://cam.example/base/"), None, &["trackID=0"], 0).unwrap();
        assert_eq!(
            plan.setups()[0].control,
            "rtsp://cam.example/base/trackID=0"
        );
    }

    #[test]
    fn content_location_is_used_when_there_is_no_content_base() {
        let request = url::Url::parse(REQUEST).unwrap();
        let plan = PlayPlan::from_describe(
            &request,
            None,
            Some("rtsp://cam.example/loc/"),
            &sdp(None, &["trackID=0"]),
            0,
        )
        .unwrap();
        assert_eq!(plan.setups()[0].control, "rtsp://cam.example/loc/trackID=0");
    }

    #[test]
    fn running_out_of_channels_is_an_error() {
        assert!(matches!(
            plan(None, None, &["trackID=0", "trackID=1"], 254),
            Err(Error::InterleavedChannelExhausted)
        ));
        assert!(matches!(
            plan(None, None, &["trackID=0"], 255),
            Err(Error::InterleavedChannelExhausted)
        ));
    }

    #[test]
    fn the_session_control_names_the_play_uri() {
        let aggregate = plan(None, Some("*"), &["trackID=0"], 0).unwrap();
        assert_eq!(aggregate.play_uri(), "rtsp://cam.example/live/key/");
        let named = plan(None, Some("agg"), &["trackID=0"], 0).unwrap();
        assert_eq!(named.play_uri(), "rtsp://cam.example/live/key/agg");
    }

    #[test]
    fn a_foreign_session_control_is_refused_or_reported() {
        assert!(matches!(
            plan(None, Some("rtsp://other.example/agg"), &["trackID=0"], 0),
            Err(Error::InvalidUrl { .. })
        ));
        let lenient = PlayPlan::from_describe_selecting(
            &url::Url::parse(REQUEST).unwrap(),
            None,
            None,
            &sdp(Some("rtsp://other.example/agg"), &["trackID=0"]),
            0,
            |_, _| true,
        )
        .unwrap();
        assert!(lenient.session_control_foreign());
        assert_eq!(lenient.play_uri(), REQUEST);
    }

    #[test]
    fn a_rejected_media_is_neither_planned_nor_skipped() {
        let request = url::Url::parse(REQUEST).unwrap();
        let plan = PlayPlan::from_describe_selecting(
            &request,
            None,
            None,
            &sdp(None, &["trackID=0", "trackID=1"]),
            0,
            |index, _| index == 1,
        )
        .unwrap();
        assert_eq!(plan.setups().len(), 1);
        assert_eq!(plan.setups()[0].media_index, 1);
        assert_eq!(plan.channels(0), Some((0, 1)));
        assert!(plan.skipped().is_empty());
    }

    #[test]
    fn a_body_that_is_not_an_sdp_is_invalid() {
        let request = url::Url::parse(REQUEST).unwrap();
        assert!(matches!(
            PlayPlan::from_describe(&request, None, None, "not an sdp", 0),
            Err(Error::InvalidSdp(_))
        ));
    }
}
