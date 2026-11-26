//! The per-track view of [`RtpPacketiserStage`](transmux::RtpPacketiserStage)'s
//! output that the RTP tests inspect.
//!
//! The stage emits one `MuxOutput::Bytes` per RTP datagram: whole packets, in
//! emission order, carrying no track. A test that wants "the video stream's
//! packets" drives one track's samples at a time through a single stage (so
//! every packet emitted during that stretch belongs to that track) and rebuilds
//! the per-track streams here, plus each track's SDP media section from
//! [`sdp_media`](transmux::sdp_media).

use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;
use test_harness::collect::Collected as Media;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{CodecConfig, RtpMediaKind, RtpPacket, RtpPacketiserStage, TrackSpec};

/// One track's packetised RTP stream.
#[derive(Debug)]
pub struct RtpStream {
    pub pt: u8,
    pub kind: RtpMediaKind,
    pub packets: Vec<RtpPacket>,
}

/// Per-track RTP streams plus the SDP media descriptions that describe them.
#[derive(Debug)]
pub struct RtpOutput {
    pub streams: Vec<RtpStream>,
    pub medias: Vec<sdp_types::Media>,
}

/// The stage configuration the RTP tests use.
pub fn stage(mtu: usize, ssrc: u32) -> RtpPacketiserStage {
    RtpPacketiserStage::new().with_mtu(mtu).with_ssrc(ssrc)
}

/// Drive `stage` over `media`, one track's samples at a time, and rebuild the
/// per-track streams and media sections.
pub fn packetise(media: &Media, stage: RtpPacketiserStage) -> RtpOutput {
    packetise_result(media, stage).expect("packetise")
}

/// [`packetise`] leaving the stage's error to the caller.
pub fn packetise_result(
    media: &Media,
    mut stage: RtpPacketiserStage,
) -> Result<RtpOutput, transmux::Error> {
    let mut streams: Vec<RtpStream> = Vec::new();
    let mut sections: Vec<Option<sdp_types::Media>> = Vec::new();
    for track in &media.tracks {
        let kind = match &track.spec.config {
            CodecConfig::Avc { .. } => RtpMediaKind::H264,
            CodecConfig::Aac { .. } => RtpMediaKind::Aac,
            _ => continue,
        };
        stage.feed(
            MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )?;
        let mut packets: Vec<RtpPacket> = drain(&mut stage)
            .filter_map(RtpPacket::from_contiguous)
            .collect();
        for sample in &track.samples {
            stage.feed(
                MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )?;
            packets.extend(drain(&mut stage).filter_map(RtpPacket::from_contiguous));
        }
        let pt = match packets.first() {
            Some(packet) if packet.header.len() > 1 => packet.header[1] & 0x7F,
            _ => stage.video_pt,
        };
        sections.push(media_for(&track.spec, pt));
        streams.push(RtpStream { pt, kind, packets });
    }
    let medias = sections.into_iter().flatten().collect();
    Ok(RtpOutput { streams, medias })
}

fn media_for(spec: &TrackSpec, pt: u8) -> Option<sdp_types::Media> {
    transmux::sdp_media(spec, pt).ok()
}

fn drain(stage: &mut RtpPacketiserStage) -> impl Iterator<Item = Bytes> + '_ {
    core::iter::from_fn(move || match stage.poll() {
        Some(MuxOutput::Bytes(bytes)) => Some(bytes),
        _ => None,
    })
}
