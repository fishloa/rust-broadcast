//! RTMP push transport — client publish to a remote RTMP server.
//!
//! Adobe RTMP 1.0 §7.2 (NetConnection/NetStream commands). RTMP Audio (type
//! 8) / Video (type 9) messages carry FLV `AudioTagHeader`+`AACAUDIODATA` /
//! `VideoTagHeader`+`AVCVIDEOPACKET` bodies (Adobe FLV v10.1 Annex E
//! §E.4.2/§E.4.3) — **not** MPEG-2 TS (issue #934: the pre-fix defect muxed
//! every push output, RTMP included, with `TsMux` and shipped the TS bytes
//! as a video message, which no RTMP server can decode). `PushTransport::setup`
//! sends `onMetaData` plus the AVC/AAC sequence headers (`avcC`/ASC) once, and
//! `send_media` is overridden to split each batch into FLV-framed payloads
//! (`transmux::flv_frame_payloads`) dispatched through `send_video`/`send_audio`
//! — see `transmux::flv` for the tag-body layout this reuses.

use crate::push::{MuxError, PushTransport, SendMediaError};
use broadcast_common::stage::{Stage, Timestamp as StageTimestamp};
use rtmp_runtime::ids::Timestamp;
use rtmp_runtime::target::RtmpTarget;
use tokio::net::TcpStream;
use transmux::CodecConfig;
use transmux::RtmpMuxStage;
use transmux::TrackId;
use transmux::flv_track_kind;
#[cfg(test)]
use transmux::ir::DataIdentity;
use transmux::ir::{Media, Track, TrackSpec};
use transmux::mux_stage::{MuxInput, MuxOutput};

/// The metadata pair the publish client frames (`@setDataFrame`/`onMetaData`,
/// Adobe RTMP 1.0 §7.1): the handler name, then the property object
/// `transmux::on_metadata_body` writes after it. The client writes
/// `@setDataFrame` and the name itself, so the body's own name is read back
/// rather than sent twice.
fn on_metadata_values(
    body: &[u8],
) -> Result<Vec<(String, rtmp_runtime::amf0::Amf0Value)>, rtmp_runtime::RtmpError> {
    use broadcast_common::{Parse, Serialize};
    use rtmp_runtime::amf0::Amf0Value;
    let name = Amf0Value::parse(body)?;
    let rest = &body[name.serialized_len()..];
    let Amf0Value::String(handler) = name else {
        return Err(rtmp_runtime::RtmpError::CommandNameNotString { value: name });
    };
    Ok(vec![(handler, Amf0Value::parse(rest)?)])
}

/// Per-connection configuration for the RTMP push transport.
#[derive(Clone)]
#[non_exhaustive]
pub struct RtmpTransportConfig {
    /// RTMP `app` name.
    pub app: String,
    /// Publishing name (stream key).
    pub stream_key: String,
    /// Bounds on every awaited IO the push performs (SP6.1, defect 4; audit run
    /// 7, W17): the connect and handshake, and each outbound write. A test, or
    /// an operator with an unusually slow peer, overrides them here.
    pub timeouts: broadcast_common::timeouts::Timeouts,
}

/// Manual `Debug`: the stream key is a publish secret and never renders
/// (issue #1142).
impl std::fmt::Debug for RtmpTransportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpTransportConfig")
            .field("app", &self.app)
            .field("stream_key", &"<redacted>")
            .field("timeouts", &self.timeouts)
            .finish()
    }
}

impl Default for RtmpTransportConfig {
    fn default() -> Self {
        Self {
            app: "live".to_string(),
            stream_key: String::new(),
            timeouts: broadcast_common::timeouts::Timeouts::default(),
        }
    }
}

impl RtmpTransportConfig {
    /// Set the RTMP `app` name.
    #[must_use]
    pub fn with_app(mut self, app: String) -> Self {
        self.app = app;
        self
    }

    /// Set the publishing name (stream key).
    #[must_use]
    pub fn with_stream_key(mut self, stream_key: String) -> Self {
        self.stream_key = stream_key;
        self
    }

    /// Set the IO timeouts (SP6.1).
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: broadcast_common::timeouts::Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

/// The RTMP push transport: the rtmp-runtime async client adapter (SP6.1).
pub struct RtmpTransport {
    client: Option<rtmp_runtime::io::AsyncRtmpClient<TcpStream>>,
    /// The connection's sample muxer: FLV tag framing, the RTMP chunk stream
    /// and the tag order all live here, so they continue across batches.
    muxer: Option<RtmpMuxStage>,
    /// Whether a track-refusal warning has already been emitted. FLV carries
    /// only AVC video and AAC audio, so any other codec is dropped — but
    /// dropping it *silently*, once per batch, is both a data-loss hazard and
    /// log spam. Warn once per connection instead.
    warned_refused_tracks: bool,
}

impl std::fmt::Debug for RtmpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpTransport")
            .field("connected", &self.client.is_some())
            .finish()
    }
}

/// The chunk size the publishing client's own chunk writer frames at.
///
/// The connection announces this size once at publish (RTMP 1.0 §5.4.1), and a
/// receiver applies it to every later chunk on the connection — including the
/// ones this transport's muxer writes through [`write_message`], so the muxer
/// must frame at exactly the same size.
fn publish_chunk_size() -> usize {
    usize::try_from(rtmp_runtime::client::RtmpClientConfig::default().chunk_size)
        .unwrap_or(usize::MAX)
}

/// Every message the stage has ready, as wire bytes.
fn drain(muxer: &mut RtmpMuxStage) -> Vec<bytes::Bytes> {
    let mut messages = Vec::new();
    while let Some(output) = muxer.poll() {
        if let MuxOutput::Bytes(bytes) = output {
            messages.push(bytes);
        }
    }
    messages
}

/// `carried`'s samples in **decode** order across tracks: every track's own
/// read order preserved, so a batch is framed in the same order the whole
/// media would have been. Decode time is the key because a B-frame track's
/// presentation order is not its decode order.
fn decode_ordered<'a>(carried: &[&'a Track]) -> Vec<(&'a Track, &'a transmux::Sample)> {
    let mut items: Vec<(i128, usize, &Track, &transmux::Sample)> = Vec::new();
    for (index, track) in carried.iter().enumerate() {
        let mut ticks = 0i64;
        for sample in &track.samples {
            let at = sample
                .timing
                .and_then(|t| t.dts())
                .or(sample.timing.and_then(|t| t.pts()))
                .unwrap_or(ticks);
            ticks = at;
            items.push((i128::from(at), index, track, sample));
        }
    }
    // Two tracks' tick counts are only comparable through their own
    // timescales: multiplying each side by the other's timescale compares the
    // real instants without a lossy rescale.
    items.sort_by(|a, b| {
        (a.0 * i128::from(b.2.spec.timescale.get()))
            .cmp(&(b.0 * i128::from(a.2.spec.timescale.get())))
            .then(a.1.cmp(&b.1))
    });
    items
        .into_iter()
        .map(|(_, _, track, sample)| (track, sample))
        .collect()
}

#[async_trait::async_trait]
impl PushTransport for RtmpTransport {
    type Config = RtmpTransportConfig;
    type Error = RtmpPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        // Host, port and the (IPv6-bracketed) `tcUrl` come from `RtmpTarget`,
        // which builds tcUrl through the `url` crate — the hand-rolled
        // `format!("rtmp://{host}:{port}/{app}")` produced
        // `rtmp://::1:1935/live` for an IPv6 address (defect 8). The `app`/
        // `stream_key` come from the config, not the URL path.
        let target = RtmpTarget::from_parts(url, &config.app, &config.stream_key)
            .map_err(|e| RtmpPushError::Connect(e.into()))?;

        // Every IO the adapter performs is bounded (SP6.1, defect 4):
        // connect/handshake and each write by `config.timeouts` (audit run 7, W17).
        let mut client = rtmp_runtime::io::AsyncRtmpClient::connect(
            rtmp_runtime::io::RtmpClientConfig::new(target).with_timeouts(config.timeouts),
        )
        .await
        .map_err(|e| RtmpPushError::Connect(e.into()))?;
        client
            .publish()
            .await
            .map_err(|e| RtmpPushError::Connect(e.into()))?;

        Ok(Self {
            client: Some(client),
            muxer: None,
            warned_refused_tracks: false,
        })
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| RtmpPushError::Connect(RtmpConnectError::NotConnected))?;
        client
            .send_video(Timestamp::ZERO, data)
            .await
            .map_err(RtmpPushError::Io)
    }

    /// Writes `message` to the socket verbatim (issue #942) — unlike
    /// [`send`](Self::send) above, this does **not** call `send_video`:
    /// `message` is already a complete chunk-stream-framed RTMP message
    /// (produced by [`encode_media`](Self::encode_media)'s
    /// `encode_video`/`encode_audio` calls), so framing it again here would
    /// nest one RTMP message inside the body of another.
    async fn write_message(&mut self, message: &[u8]) -> Result<(), Self::Error> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| RtmpPushError::Connect(RtmpConnectError::NotConnected))?;
        client
            .write_frame(message.to_vec())
            .await
            .map_err(RtmpPushError::Io)
    }

    /// FLV's mainstream is AVC video + AAC audio only (issue #942) — see
    /// `transmux::flv_track_kind`.
    fn supports_codec(&self, config: &CodecConfig) -> bool {
        flv_track_kind(config).is_some()
    }

    /// Send `onMetaData` plus the AVC/AAC sequence headers (`avcC`/ASC) once,
    /// before any frame data (issue #934) — a decoder cannot initialise
    /// without them. Tracks this transport cannot carry over RTMP (anything
    /// but AVC/AAC — see `transmux::flv_track_kind`) are excluded here and in
    /// `encode_media`.
    ///
    /// This is also where the connection's [`RtmpMuxStage`] is created and
    /// given its tracks: the stage frames every later message, so its chunk
    /// stream state (and the FLV tag order) continues across batches.
    async fn setup(&mut self, tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let flv_tracks: Vec<TrackSpec> = tracks
            .iter()
            .filter(|t| flv_track_kind(&t.config).is_some())
            .cloned()
            .collect();
        if flv_tracks.is_empty() {
            return Err(RtmpPushError::Protocol(RtmpProtocolError::NoCarriableTrack));
        }
        // The publish client's own chunk streams carry the connection's
        // opening messages: `@setDataFrame`/`onMetaData` and the AVC/AAC
        // sequence headers. The muxer then writes only the frame messages, on
        // its own chunk streams, so one writer never inherits another's
        // chunk-header state.
        let body = transmux::on_metadata_body(&flv_tracks, None)
            .map_err(|e| RtmpPushError::Protocol(e.into()))?;
        let values = on_metadata_values(&body).map_err(|e| RtmpPushError::Protocol(e.into()))?;
        let meta_bytes = {
            let client = self
                .client
                .as_mut()
                .ok_or_else(|| RtmpPushError::Connect(RtmpConnectError::NotConnected))?;
            client
                .encode_metadata(&values)
                .map_err(|e| RtmpPushError::Protocol(e.into()))?
        };
        self.write_message(&meta_bytes).await?;
        for header in transmux::flv_sequence_header_payloads(&flv_tracks)
            .map_err(|e| RtmpPushError::Protocol(e.into()))?
        {
            let bytes = {
                let client = self
                    .client
                    .as_mut()
                    .ok_or_else(|| RtmpPushError::Connect(RtmpConnectError::NotConnected))?;
                match header.kind {
                    transmux::FlvPayloadKind::Video => {
                        client.encode_video(Timestamp::from_millis(0), &header.body)
                    }
                    transmux::FlvPayloadKind::Audio => {
                        client.encode_audio(Timestamp::from_millis(0), &header.body)
                    }
                    _ => continue,
                }
                .map_err(|e| RtmpPushError::Protocol(e.into()))?
            };
            self.write_message(&bytes).await?;
        }

        let mut muxer = RtmpMuxStage::with_chunk_size(publish_chunk_size()).frames_only();
        for spec in &flv_tracks {
            muxer
                .feed(
                    MuxInput::Track(Box::new(spec.clone())),
                    StageTimestamp::ZERO,
                )
                .map_err(|e| RtmpPushError::Protocol(RtmpProtocolError::Mux(e)))?;
        }
        self.muxer = Some(muxer);
        Ok(())
    }

    /// The connection's per-sample muxer, so the push egress feeds it one
    /// sample at a time instead of building a one-sample `Media`.
    fn media_muxer(&mut self) -> Option<&mut RtmpMuxStage> {
        self.muxer.as_mut()
    }

    /// Frame `media`'s samples as RTMP messages through the connection's
    /// [`RtmpMuxStage`], performing no I/O (issue #942: the sans-IO half of
    /// [`send_media`](Self::send_media)).
    ///
    /// Tracks this transport cannot carry are excluded, and reported once per
    /// connection. A batch with no carriable track at all is
    /// [`MuxError::NoCarriableTrack`]: `Mux` is the right class, because
    /// reconnecting cannot fix a codec mismatch.
    ///
    /// Every carried track is announced before the first sample is fed, even
    /// when `setup` already did so — the stage then writes the metadata and
    /// sequence headers as part of this batch, which is what a caller that
    /// skipped `setup` needs.
    fn encode_media(&mut self, media: &Media) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        let carried: Vec<&transmux::Track> = media
            .tracks
            .iter()
            .filter(|t| flv_track_kind(&t.spec.config).is_some())
            .collect();

        // Refusing a track the wire format cannot carry is legitimate;
        // refusing it *silently* is not. Report it once per connection.
        let refused = media.tracks.len() - carried.len();
        if refused > 0 && !self.warned_refused_tracks {
            self.warned_refused_tracks = true;
            let refused_track_ids: Vec<TrackId> = media
                .tracks
                .iter()
                .filter(|t| flv_track_kind(&t.spec.config).is_none())
                .map(|t| t.spec.track_id)
                .collect();
            tracing::warn!(
                refused,
                carried = carried.len(),
                ?refused_track_ids,
                "RTMP push cannot carry these tracks — FLV carries only AVC video                  and AAC audio; they are excluded from this push"
            );
        }
        if carried.is_empty() {
            return Err(SendMediaError::Mux(MuxError::NoCarriableTrack));
        }
        if self.client.is_none() {
            return Err(SendMediaError::Transport(Box::new(RtmpPushError::Connect(
                RtmpConnectError::NotConnected,
            ))));
        }

        // A batch is framed through its own muxer: the batch API is stateless
        // per call (the connection's own muxer numbers the live stream
        // continuously, through `media_muxer`).
        let mut muxer = RtmpMuxStage::with_chunk_size(publish_chunk_size()).frames_only();
        for track in &carried {
            muxer
                .feed(
                    MuxInput::Track(Box::new(track.spec.clone())),
                    StageTimestamp::ZERO,
                )
                .map_err(|e| SendMediaError::Mux(MuxError::Rtmp(e)))?;
        }
        let mut messages = drain(&mut muxer);
        for (track, sample) in decode_ordered(&carried) {
            muxer
                .feed(
                    MuxInput::Sample {
                        track: track.spec.track_id,
                        sample: sample.clone(),
                    },
                    StageTimestamp::ZERO,
                )
                .map_err(|e| SendMediaError::Mux(MuxError::Rtmp(e)))?;
            messages.extend(drain(&mut muxer));
        }
        // A batch boundary is where the stage's one-pending-sample-per-track
        // ordering can be drained: whatever it still holds goes out now, and
        // the stream stays open for the next batch.
        muxer
            .feed(MuxInput::Flush, StageTimestamp::ZERO)
            .map_err(|e| SendMediaError::Mux(MuxError::Rtmp(e)))?;
        messages.extend(drain(&mut muxer));
        Ok(messages)
    }

    /// Encodes `media` via [`encode_media`](Self::encode_media) then writes
    /// each resulting chunk-framed RTMP message straight to the socket —
    /// bypasses [`send`](Self::send) (which would re-frame the bytes as a
    /// *new* video message) and the trait's default `TsMux` path entirely.
    async fn send_media(&mut self, media: &Media) -> Result<u64, SendMediaError> {
        let messages = self.encode_media(media)?;
        let mut total = 0u64;
        for message in &messages {
            self.write_message(message)
                .await
                .map_err(|e| SendMediaError::Transport(Box::new(e)))?;
            total += message.len() as u64;
        }
        Ok(total)
    }

    fn close(&mut self) {
        self.client = None;
        self.muxer = None;
    }
}

/// Errors from the RTMP push transport.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtmpPushError {
    /// Connection failed.
    #[error("RTMP connect failed: {0}")]
    Connect(#[from] RtmpConnectError),
    /// Protocol error.
    #[error("RTMP protocol error: {0}")]
    Protocol(#[from] RtmpProtocolError),
    /// I/O error.
    #[error("RTMP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Why an RTMP push could not connect.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtmpConnectError {
    /// The push URL could not be turned into an RTMP target.
    #[error("bad rtmp URL: {0}")]
    Url(#[from] rtmp_runtime::target::RtmpUrlError),
    /// The client has not connected (or was closed) when a send was attempted.
    #[error("not connected")]
    NotConnected,
    /// The connect or publish handshake failed.
    #[error(transparent)]
    Handshake(#[from] std::io::Error),
}

/// Why an RTMP push failed at the protocol/encoding layer.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtmpProtocolError {
    /// No track of the batch can be published over RTMP (not AVC/AAC).
    #[error("no AVC video or AAC audio track to publish over RTMP")]
    NoCarriableTrack,
    /// Building the FLV/onMetaData payload failed.
    #[error(transparent)]
    Flv(#[from] transmux::FlvError),
    /// Framing the FLV tags as RTMP messages failed.
    #[error(transparent)]
    Mux(#[from] transmux::RtmpFlvError),
    /// Decoding the onMetaData body failed.
    #[error(transparent)]
    Amf(#[from] rtmp_runtime::RtmpError),
    /// Encoding a message failed.
    #[error(transparent)]
    Encode(#[from] std::io::Error),
}

/// `test-seams` hook: build the `tcUrl` through [`RtmpTarget::from_parts`] —
/// the real path the connect uses (production takes `app`/`stream_key` from
/// config and only the host/port from the URL) — so the test exercises the
/// same constructor. The `app` is the URL's first path segment and the stream
/// key the rest.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub fn tc_url_for_test(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    let mut segs = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()))
        .into_iter()
        .flatten();
    let app = segs.next().unwrap_or("").to_string();
    let stream_key = segs.collect::<Vec<_>>().join("/");
    RtmpTarget::from_parts(url, &app, &stream_key)
        .map(|t| t.tc_url().to_owned())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Audit run 7, W17: a server that accepts the TCP connection but never
    /// replies to the RTMP handshake must fail the connect within the
    /// configured bound, not wedge forever. A real loopback listener accepts
    /// and then reads and discards without ever writing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_times_out_against_a_black_hole_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind black-hole listener");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Accept, then never respond — read and discard forever.
                let mut buf = [0u8; 4096];
                while matches!(sock.read(&mut buf).await, Ok(n) if n > 0) {}
            }
        });

        let cfg = RtmpTransportConfig {
            app: "live".to_string(),
            stream_key: "test".to_string(),
            timeouts: broadcast_common::timeouts::Timeouts::default()
                .with_connect(std::time::Duration::from_millis(200))
                .with_handshake(std::time::Duration::from_millis(200)),
        };
        let url = format!("rtmp://{addr}/live/test");
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            RtmpTransport::connect(&url, &cfg),
        )
        .await
        .expect("connect must return, not hang");
        server.abort();
        let err = result.expect_err("black-hole handshake must fail");
        assert!(
            matches!(&err, RtmpPushError::Connect(RtmpConnectError::Handshake(_))),
            "the failure must be the connect timeout: {err:?}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the connect bound must fire promptly, took {:?}",
            started.elapsed()
        );
    }

    fn avc_config() -> CodecConfig {
        CodecConfig::Avc {
            config: isobmff::AVCConfigurationBox::new(isobmff::AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1f,
                length_size_minus_one: 3,
                sps: Vec::new(),
                pps: Vec::new(),
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 0,
            height: 0,
        }
    }

    fn opaque_config() -> CodecConfig {
        CodecConfig::Data {
            identity: DataIdentity::Ts {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: transmux::ir::DataCarriage::Pes,
            },
        }
    }

    /// `PushTransport::supports_codec` (issue #942) must delegate to the
    /// real `transmux::flv_track_kind` — the one place `push::egress::
    /// PushTransportEgress::negotiate`/`renegotiate` decide whether a track
    /// is carriable at all, replacing the transport's own former ad-hoc
    /// check. Exercised against the real `RtmpTransport`, not a test
    /// double, since this is the exact call `drive_push` makes in
    /// production.
    ///
    /// MUTATION-CHECKED: changing this override's body to `true` (always
    /// carriable) makes both assertions fail identically — `supports_codec`
    /// would then never refuse the opaque config, which is exactly what
    /// would have let a `Data`-carried private stream silently reach
    /// `negotiate` as "carriable" and later fail deep inside
    /// `encode_media`'s FLV framing instead of being refused truthfully at
    /// negotiation time. Reverted after confirming the failure.
    #[test]
    fn supports_codec_matches_flv_track_kind() {
        let transport = RtmpTransport {
            client: None,
            muxer: None,
            warned_refused_tracks: false,
        };
        assert!(
            transport.supports_codec(&avc_config()),
            "AVC must be RTMP-carriable"
        );
        assert!(
            !transport.supports_codec(&opaque_config()),
            "opaque PES data must not be RTMP-carriable"
        );
    }

    fn timescale() -> broadcast_common::timescale::Timescale {
        broadcast_common::timescale::Timescale::try_new(90_000).expect("a non-zero timescale")
    }

    fn spec(track_id: u32, config: CodecConfig) -> TrackSpec {
        TrackSpec::new(TrackId::new(track_id), timescale(), config)
    }

    /// The `Debug` body names the type and its connection state.
    #[test]
    fn transport_debug_reports_the_connection_state() {
        let transport = RtmpTransport {
            client: None,
            muxer: None,
            warned_refused_tracks: false,
        };
        let shown = format!("{transport:?}");
        assert!(shown.contains("RtmpTransport"), "{shown}");
        assert!(shown.contains("connected: false"), "{shown}");
    }

    /// The refused-track warning fires once per connection with the exact
    /// count of tracks FLV cannot carry, and never when every track is
    /// carriable.
    #[test]
    fn refused_tracks_are_warned_once_with_the_right_count() {
        #[derive(Clone, Default)]
        struct Capture(std::sync::Arc<std::sync::Mutex<String>>);

        impl tracing::Subscriber for Capture {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                struct Fields<'a>(&'a mut String);
                impl tracing::field::Visit for Fields<'_> {
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        use std::fmt::Write as _;
                        let _ = write!(self.0, "{}={value:?} ", field.name());
                    }
                }
                let mut line = String::new();
                event.record(&mut Fields(&mut line));
                line.push('\n');
                let mut buffer = self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                buffer.push_str(&line);
            }
        }

        let capture = Capture::default();
        let buffer = capture.0.clone();
        tracing::subscriber::with_default(capture, || {
            let mut transport = RtmpTransport {
                client: None,
                muxer: None,
                warned_refused_tracks: false,
            };
            let mixed = Media::new(
                vec![
                    Track::new(spec(1, avc_config()), Vec::new()),
                    Track::new(spec(2, opaque_config()), Vec::new()),
                    Track::new(spec(3, opaque_config()), Vec::new()),
                    Track::new(spec(4, opaque_config()), Vec::new()),
                ],
                transmux::VIDEO_CLOCK_TIMESCALE,
            );
            let all_carriable = Media::new(
                vec![Track::new(spec(1, avc_config()), Vec::new())],
                transmux::VIDEO_CLOCK_TIMESCALE,
            );
            // An all-carriable batch first: `refused == 0` must not warn at
            // all (a `>= 0` boundary would).
            let _ = transport.encode_media(&all_carriable);
            let _ = transport.encode_media(&mixed);
            let _ = transport.encode_media(&mixed);
        });
        let logs = buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            logs.matches("RTMP push cannot carry these tracks").count(),
            1,
            "the refusal is reported once per connection: {logs}"
        );
        assert!(logs.contains("refused=3 "), "{logs}");
        assert!(logs.contains("carried=1 "), "{logs}");
    }
}
