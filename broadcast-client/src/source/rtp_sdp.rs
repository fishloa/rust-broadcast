//! SDP-described RTP over datagrams (RFC 3550 §5.1, RFC 4566): the streams an out-of-band SDP
//! declares are played by RTP payload type, depayloaded with [`RtpStreamDepacketiser`]
//! (RFC 6184 H.264, RFC 3640 AAC) and announced as one [`Event::Tracks`] before any packet.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::Parse;
use broadcast_common::timescale::Timescale;
use rtp_packet::RtpPacket;
use rtsp_runtime::MAX_TRACKS;
use sdp_types::Session as SdpSession;
use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::{RtpStreamDepacketiser, RtpStreamTrack, TrackId, sdp_track};

use crate::{Error, Event};

#[derive(Clone)]
struct Declared {
    track_id: TrackId,
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: Timescale,
    payload_type: u8,
}

pub(crate) struct SdpRtp {
    declared: Vec<Declared>,
    depa: RtpStreamDepacketiser,
    by_payload_type: BTreeMap<u8, TrackId>,
    announced: bool,
    events: Vec<Event>,
    accepted_bytes: u64,
    datagrams: u64,
    dropped: u64,
}

fn declared(sdp: &str) -> Result<Vec<Declared>, Error> {
    let invalid = Error::InvalidConfig { field: "udp.sdp" };
    let session = SdpSession::parse(sdp.as_bytes()).map_err(|_| invalid.clone())?;
    let mut out = Vec::new();
    for (media_index, media) in session.medias.iter().enumerate() {
        if out.len() >= MAX_TRACKS {
            break;
        }
        let Some(track) = sdp_track(media) else {
            continue;
        };
        let Some(payload_type) = track.payload_type else {
            continue;
        };
        let position = u32::try_from(media_index).map_or(u32::MAX, |i| i.saturating_add(1));
        out.push(Declared {
            track_id: TrackId::new(position),
            kind: track.kind,
            config: track.config,
            clock_rate: track.clock_rate,
            payload_type,
        });
    }
    if out.is_empty() {
        return Err(invalid);
    }
    Ok(out)
}

fn depacketiser(tracks: &[Declared]) -> RtpStreamDepacketiser {
    RtpStreamDepacketiser::new(
        tracks
            .iter()
            .map(|t| RtpStreamTrack::new(t.track_id, t.kind, t.config.clone(), t.clock_rate))
            .collect(),
    )
}

impl SdpRtp {
    pub(crate) fn new(sdp: &str) -> Result<Self, Error> {
        let tracks = declared(sdp)?;
        Ok(Self {
            depa: depacketiser(&tracks),
            by_payload_type: tracks
                .iter()
                .map(|t| (t.payload_type, t.track_id))
                .collect(),
            declared: tracks,
            announced: false,
            events: Vec::new(),
            accepted_bytes: 0,
            datagrams: 0,
            dropped: 0,
        })
    }

    pub(crate) fn announce(&mut self) {
        if !self.announced {
            self.announced = true;
            self.events.push(Event::Tracks(self.depa.track_specs()));
        }
    }

    pub(crate) fn push(&mut self, datagram: &[u8]) {
        self.announce();
        self.datagrams = self.datagrams.saturating_add(1);
        let Ok(packet) = RtpPacket::parse(datagram) else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        let Some(&track) = self.by_payload_type.get(&packet.payload_type) else {
            return;
        };
        self.accepted_bytes = self.accepted_bytes.saturating_add(datagram.len() as u64);
        match self.depa.push(track, datagram) {
            Ok(samples) => self.emit(track, samples),
            Err(_) => self.dropped = self.dropped.saturating_add(1),
        }
    }

    /// Emits the access units still held in every track's depacketiser: a partially assembled
    /// unit and the one held back to measure its duration are complete when the stream ends.
    pub(crate) fn flush(&mut self) {
        let tracks: Vec<TrackId> = self.depa.track_specs().iter().map(|s| s.track_id).collect();
        for track in tracks {
            match self.depa.flush(track) {
                Ok(samples) => self.emit(track, samples),
                Err(_) => self.dropped = self.dropped.saturating_add(1),
            }
        }
    }

    fn emit(&mut self, track: TrackId, samples: Vec<transmux::Sample>) {
        for sample in samples {
            self.events.push(Event::Sample {
                track,
                sample,
                join: None,
            });
        }
    }

    pub(crate) fn reset(&mut self) {
        self.flush();
        self.depa = depacketiser(&self.declared);
        self.announced = false;
    }

    pub(crate) fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn take_accepted_bytes(&mut self) -> u64 {
        core::mem::take(&mut self.accepted_bytes)
    }

    pub(crate) fn stats(&self) -> crate::udp::DatagramStats {
        crate::udp::DatagramStats {
            datagrams: self.datagrams,
            malformed: self.dropped,
            ..crate::udp::DatagramStats::default()
        }
    }
}
