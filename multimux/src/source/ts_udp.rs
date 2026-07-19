//! MPEG-2 Transport Stream over UDP ingest source (issue #663 P3a): a UDP
//! socket (unicast or multicast) feeding transmux's incremental
//! [`transmux::StreamingTsDemux`] — multimux owns only the socket; all PAT/
//! PMT/PES demuxing and codec-config recovery is transmux's, the same
//! streaming demux core `ts-fix` and every other TS consumer in this
//! workspace drives.
//!
//! Since UDP is connectionless there is no DESCRIBE-equivalent step to learn
//! the track set before segmentation starts. [`TsUdpSource::connect`]
//! instead reads datagrams (bounded by a fixed `CONNECT_TIMEOUT`) until the PMT
//! resolves ([`transmux::DemuxEvent::TracksResolved`]) — the TS-over-UDP
//! analogue of RTSP's "DESCRIBE before PLAY" ordering — so
//! [`TsUdpSession::track_specs`] is always populated before the pipeline
//! builds its segmenter.

use std::collections::BTreeSet;
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};
use crate::source::Source;
use crate::source::udp::bind_udp;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingTsDemux};

/// Max UDP datagram this source reads in one `recv` — comfortably above a
/// typical 7×188-byte (1316-byte) TS-over-UDP payload and any legal UDP
/// datagram (65 507 bytes over IPv4).
const MAX_UDP_DATAGRAM: usize = 65_536;

/// Bound on how long [`TsUdpSource::connect`] waits for the PMT to resolve
/// (every currently-declared track known) before giving up — a source that
/// never sends usable PSI would otherwise hang `connect` forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// An MPEG-2 TS-over-UDP stream to pull: bind address (+ optional multicast
/// group) — no control plane, no out-of-band SDP (the PMT carries the track
/// set in-band, unlike raw RTP/UDP).
#[derive(Clone)]
pub struct TsUdpSource {
    name: String,
    addr: String,
    multicast_group: Option<String>,
}

impl std::fmt::Debug for TsUdpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsUdpSource")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("multicast_group", &self.multicast_group)
            .finish()
    }
}

impl TsUdpSource {
    /// Build a source descriptor.
    pub fn new(
        name: impl Into<String>,
        addr: impl Into<String>,
        multicast_group: Option<String>,
    ) -> Self {
        TsUdpSource {
            name: name.into(),
            addr: addr.into(),
            multicast_group,
        }
    }

    /// Binds the UDP socket (joining `multicast_group` if configured), then
    /// reads datagrams into a [`StreamingTsDemux`] until every currently
    /// PMT-declared track has resolved (or `CONNECT_TIMEOUT` elapses) —
    /// the streaming-demux analogue of RTSP's DESCRIBE step, so
    /// [`TsUdpSession::track_specs`] is populated before segmentation starts.
    pub async fn connect(&self) -> Result<TsUdpSession> {
        let socket = bind_udp(&self.addr, self.multicast_group.as_deref()).await?;
        let mut demux = StreamingTsDemux::new();
        let mut specs: Vec<TrackSpec> = Vec::new();
        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];

        let wait_for_tracks = async {
            loop {
                let n = socket
                    .recv(&mut buf)
                    .await
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("udp recv: {e}"),
                    })?;
                demux.feed(&buf[..n]);
                let mut resolved = false;
                while let Some(event) = demux.poll_event() {
                    match event {
                        DemuxEvent::TrackAdded(track) => specs.push(track.spec.clone()),
                        DemuxEvent::TracksResolved => resolved = true,
                        _ => {}
                    }
                }
                if resolved && !specs.is_empty() {
                    return Ok::<(), MultimuxError>(());
                }
            }
        };

        match tokio::time::timeout(CONNECT_TIMEOUT, wait_for_tracks).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "ts/udp: no PMT-declared track resolved within {CONNECT_TIMEOUT:?}"
                    ),
                });
            }
        }

        let known_track_ids: BTreeSet<u32> = specs.iter().map(|s| s.track_id).collect();
        Ok(TsUdpSession {
            socket,
            demux,
            specs,
            known_track_ids,
            buf,
        })
    }
}

impl Source for TsUdpSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live TS-over-UDP session: bound socket, feeding a [`StreamingTsDemux`].
pub struct TsUdpSession {
    socket: UdpSocket,
    demux: StreamingTsDemux,
    specs: Vec<TrackSpec>,
    /// Track ids known at connect time — a `Sample` for any later-discovered
    /// track (e.g. a PMT version bump after `connect` returned) is dropped
    /// rather than surfaced for a track the segmenter was never built with,
    /// mirroring `RtspSession::next_samples`'s "unrouted channel -> ignored"
    /// handling.
    known_track_ids: BTreeSet<u32>,
    buf: Vec<u8>,
}

impl TsUdpSession {
    /// The `TrackSpec`s resolved during [`TsUdpSource::connect`]'s PMT wait.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Receives one UDP datagram (one or more 188-byte TS packets) and feeds
    /// it to the demuxer, returning every completed sample it yields for a
    /// track known at connect time.
    ///
    /// Never returns `Ok(None)`: like
    /// [`crate::source::rtp_udp::RtpUdpSession`], UDP is connectionless, so
    /// there is no transport-level end-of-stream signal.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let n = self
            .socket
            .recv(&mut self.buf)
            .await
            .map_err(|e| MultimuxError::Connect {
                reason: format!("udp recv: {e}"),
            })?;
        self.demux.feed(&self.buf[..n]);
        let mut out = Vec::new();
        while let Some(event) = self.demux.poll_event() {
            if let DemuxEvent::Sample { track_id, sample } = event {
                if self.known_track_ids.contains(&track_id) {
                    out.push((track_id, sample));
                }
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::TsMux;
    use transmux::media::Track;
    use transmux::pipeline::CodecConfig;

    /// Builds a tiny, real (not hand-faked) MPEG-2 TS byte stream carrying
    /// one H.264 video track with a handful of access units, by round-
    /// tripping through the workspace's own `transmux::TsMux` packager —
    /// exactly the kind of "real fixture, not inline bytes" the project's
    /// spec-grounding discipline calls for, since a hand-built TS risks
    /// missing real PSI/PES framing quirks a muxed stream actually has.
    fn build_ts_bytes() -> Vec<u8> {
        use broadcast_common::Package;
        let avc = transmux::avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let spec = TrackSpec::new(
            1,
            90_000,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        let frame_dur = 90_000 / 30;
        // `TsMux` expects length-prefixed NAL data (the fMP4/CMAF `avcC`
        // sample convention: a 4-byte big-endian length prefix + the NAL
        // bytes) — it converts to Annex B internally for TS/PES transport.
        let samples: Vec<Sample> = (0..10u32)
            .map(|i| {
                let nal = [0x65u8, 0xAA, i as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                Sample::new(data, frame_dur, i == 0, 0)
            })
            .collect();
        let track = Track::new(spec, samples);
        let media = transmux::media::Media::new(vec![track], 90_000);
        TsMux::default().package(&media).expect("mux to TS")
    }

    #[tokio::test]
    async fn loopback_udp_ts_yields_samples_after_pmt_resolves() {
        // `TsUdpSource::connect` needs to know its bind address before the
        // sender can target it, but the bound socket isn't observable until
        // `connect()` returns — so reserve a real ephemeral port up front via
        // a throwaway socket, drop it, and have both the source and the
        // sender use that address. UDP has no `TIME_WAIT` (unlike TCP), so
        // the port is immediately reusable once dropped.
        let reserved = UdpSocket::bind("127.0.0.1:0").await.expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source = TsUdpSource::new("cam-ts", addr.to_string(), None);
        let ts_bytes = build_ts_bytes();
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        let send_task = tokio::spawn(async move {
            // Give connect() a moment to bind + start reading before the
            // first datagram is sent (UDP has no connect-then-accept
            // handshake to synchronize on).
            tokio::time::sleep(Duration::from_millis(20)).await;
            for chunk in ts_bytes.chunks(7 * 188) {
                sender.send_to(chunk, addr).await.expect("send TS datagram");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");
        send_task.await.expect("sender task");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 1, "one video track from the muxed TS");
        assert_eq!(specs[0].timescale, 90_000);

        // Drain whatever samples are already in flight, plus poll a little
        // more in case the last datagram(s) hadn't been read by the demuxer
        // when connect() returned (TracksResolved can fire before every
        // sample has arrived).
        let mut samples = Vec::new();
        for _ in 0..20 {
            match tokio::time::timeout(Duration::from_millis(200), session.next_samples()).await {
                Ok(Ok(Some(batch))) => samples.extend(batch),
                _ => break,
            }
        }
        assert!(
            !samples.is_empty(),
            "expected at least one sample from the muxed TS stream"
        );
    }
}
