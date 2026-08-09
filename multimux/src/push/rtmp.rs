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

use crate::push::{PushTransport, SendMediaError};
use rtmp_runtime::amf0::Amf0Value;
use rtmp_runtime::client::{ClientConfig, ClientSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use transmux::CodecConfig;
use transmux::ir::{Media, Track, TrackSpec};

/// FLV `videocodecid` metadata value for AVC (`CodecID` 7, Adobe FLV v10.1 §E.4.3).
const META_VIDEOCODECID_AVC: f64 = 7.0;
/// FLV `audiocodecid` metadata value for AAC (`SoundFormat` 10, Adobe FLV v10.1 §E.4.2).
const META_AUDIOCODECID_AAC: f64 = 10.0;

/// Whether `config` is a codec RTMP/FLV can carry (issue #934: FLV's
/// mainstream is AVC video + AAC audio only — `transmux::flv`'s module doc).
/// Any other track present in the trunk (e.g. a private/section stream) is
/// silently excluded from the RTMP push rather than failing the whole batch.
fn is_flv_codec(config: &CodecConfig) -> bool {
    matches!(config, CodecConfig::Avc { .. } | CodecConfig::Aac { .. })
}

/// Build the `onMetaData` key/value list for `tracks`' (at most one) AVC and
/// (at most one) AAC track — informational only; a decoder needs the
/// sequence headers (`avcC`/ASC), not this, to actually decode.
fn build_metadata(tracks: &[TrackSpec]) -> Vec<(String, Amf0Value)> {
    let mut meta = Vec::new();
    for t in tracks {
        match &t.config {
            CodecConfig::Avc { width, height, .. } => {
                meta.push(("width".to_string(), Amf0Value::Number(*width as f64)));
                meta.push(("height".to_string(), Amf0Value::Number(*height as f64)));
                meta.push((
                    "videocodecid".to_string(),
                    Amf0Value::Number(META_VIDEOCODECID_AVC),
                ));
            }
            CodecConfig::Aac {
                sample_rate,
                channel_count,
                ..
            } => {
                meta.push((
                    "audiocodecid".to_string(),
                    Amf0Value::Number(META_AUDIOCODECID_AAC),
                ));
                meta.push((
                    "audiosamplerate".to_string(),
                    Amf0Value::Number(*sample_rate as f64),
                ));
                meta.push((
                    "audiochannels".to_string(),
                    Amf0Value::Number(*channel_count as f64),
                ));
            }
            _ => {}
        }
    }
    meta
}

/// Per-connection configuration for the RTMP push transport.
#[derive(Debug, Clone)]
pub struct RtmpTransportConfig {
    /// RTMP `app` name.
    pub app: String,
    /// Publishing name (stream key).
    pub stream_key: String,
}

impl Default for RtmpTransportConfig {
    fn default() -> Self {
        Self {
            app: "live".to_string(),
            stream_key: String::new(),
        }
    }
}

/// The RTMP push transport: an owned TCP connection + [`ClientSession`].
pub struct RtmpTransport {
    stream: Option<TcpStream>,
    client: ClientSession,
    /// Whether a track-refusal warning has already been emitted. FLV carries
    /// only AVC video and AAC audio, so any other codec is dropped — but
    /// dropping it *silently*, once per batch, is both a data-loss hazard and
    /// log spam. Warn once per connection instead.
    warned_refused_tracks: bool,
}

impl std::fmt::Debug for RtmpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpTransport")
            .field("connected", &self.stream.is_some())
            .finish()
    }
}

#[async_trait::async_trait]
impl PushTransport for RtmpTransport {
    type Config = RtmpTransportConfig;
    type Error = RtmpPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let parsed = url::Url::parse(url).map_err(|e| RtmpPushError::Connect(e.to_string()))?;
        let host = parsed.host_str().unwrap_or("127.0.0.1");
        let port = parsed.port().unwrap_or(1935);
        let addr = format!("{host}:{port}");

        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| RtmpPushError::Connect(e.to_string()))?;

        let tc_url = format!("rtmp://{host}:{port}/{}", config.app);
        let mut client_config = ClientConfig::default();
        client_config.app = config.app.clone();
        client_config.stream_key = config.stream_key.clone();
        client_config.tc_url = Some(tc_url);
        let mut client = ClientSession::new(client_config);
        let c0_c1 = client.start();
        stream.write_all(&c0_c1).await.map_err(RtmpPushError::Io)?;

        let mut buf = vec![0u8; 8192];
        loop {
            let n = stream.read(&mut buf).await.map_err(RtmpPushError::Io)?;
            if n == 0 {
                return Err(RtmpPushError::Connect(
                    "connection closed during handshake".into(),
                ));
            }
            let (reply, events) = client
                .handle_data(&buf[..n])
                .map_err(|e| RtmpPushError::Protocol(e.to_string()))?;
            if !reply.is_empty() {
                stream.write_all(&reply).await.map_err(RtmpPushError::Io)?;
            }
            if client.is_publishing() {
                return Ok(Self {
                    stream: Some(stream),
                    client,
                    warned_refused_tracks: false,
                });
            }
            if events
                .iter()
                .any(|e| matches!(e, rtmp_runtime::client::ClientEvent::Error { .. }))
            {
                return Err(RtmpPushError::Protocol("server rejected connection".into()));
            }
        }
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| RtmpPushError::Connect("not connected".into()))?;
        let bytes = self
            .client
            .send_video(0, data)
            .map_err(|e| RtmpPushError::Protocol(e.to_string()))?;
        stream.write_all(&bytes).await.map_err(RtmpPushError::Io)
    }

    /// Writes `message` to the socket verbatim (issue #942) — unlike
    /// [`send`](Self::send) above, this does **not** call `send_video`:
    /// `message` is already a complete chunk-stream-framed RTMP message
    /// (produced by [`encode_media`](Self::encode_media)'s
    /// `send_video`/`send_audio` calls), so framing it again here would
    /// nest one RTMP message inside the body of another.
    async fn write_message(&mut self, message: &[u8]) -> Result<(), Self::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| RtmpPushError::Connect("not connected".into()))?;
        stream.write_all(message).await.map_err(RtmpPushError::Io)
    }

    /// FLV's mainstream is AVC video + AAC audio only (issue #942) — see
    /// this module's own `is_flv_codec`.
    fn supports_codec(&self, config: &CodecConfig) -> bool {
        is_flv_codec(config)
    }

    /// Send `onMetaData` plus the AVC/AAC sequence headers (`avcC`/ASC) once,
    /// before any frame data (issue #934) — a decoder cannot initialise
    /// without them. Tracks this transport cannot carry over RTMP (anything
    /// but AVC/AAC — `is_flv_codec`) are silently excluded, matching
    /// `send_media`'s per-batch filtering below.
    async fn setup(&mut self, tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let flv_tracks: Vec<TrackSpec> = tracks
            .iter()
            .filter(|t| is_flv_codec(&t.config))
            .cloned()
            .collect();
        if flv_tracks.is_empty() {
            return Err(RtmpPushError::Protocol(
                "no AVC video or AAC audio track to publish over RTMP".into(),
            ));
        }

        let RtmpTransport { stream, client, .. } = self;
        let stream = stream
            .as_mut()
            .ok_or_else(|| RtmpPushError::Connect("not connected".into()))?;

        let metadata = build_metadata(&flv_tracks);
        let meta_bytes = client
            .send_metadata(&metadata)
            .map_err(|e| RtmpPushError::Protocol(e.to_string()))?;
        stream
            .write_all(&meta_bytes)
            .await
            .map_err(RtmpPushError::Io)?;

        // A zero-sample `Media` is enough to build the sequence-header
        // payloads — they're derived only from `TrackSpec::config`.
        let media = Media::new(
            flv_tracks
                .into_iter()
                .map(|spec| Track::new(spec, Vec::new()))
                .collect(),
            0,
        );
        let headers = transmux::flv_sequence_header_payloads(&media)
            .map_err(|e| RtmpPushError::Protocol(e.to_string()))?;
        for header in &headers {
            // `FlvPayloadKind` is `#[non_exhaustive]`: only `Video`/`Audio`
            // exist today (transmux's FLV mainstream); a future kind is
            // silently skipped here rather than sent as neither.
            let sent = match header.kind {
                transmux::FlvPayloadKind::Video => Some(client.send_video(0, &header.body)),
                transmux::FlvPayloadKind::Audio => Some(client.send_audio(0, &header.body)),
                _ => None,
            };
            let Some(bytes) = sent else { continue };
            let bytes = bytes.map_err(|e| RtmpPushError::Protocol(e.to_string()))?;
            stream.write_all(&bytes).await.map_err(RtmpPushError::Io)?;
        }
        Ok(())
    }

    /// Split `media` into FLV-framed video/audio *messages*, performing no
    /// I/O (issue #942: the sans-IO half of `send_media`, below — see
    /// `PushTransport::encode_media`'s own doc for why this split exists;
    /// `push::egress::PushTransportEgress::send` is the caller that actually
    /// needs it, since its trait is synchronous). `rtmp_runtime::client::
    /// ClientSession::send_video`/`send_audio` are themselves sans-IO
    /// (RTMP chunk-stream framing is pure computation — only the socket
    /// write that follows is I/O), so this is a real factoring, not a
    /// workaround. Tracks this transport cannot carry over RTMP are
    /// excluded (`is_flv_codec`); if that leaves nothing at all, returns
    /// [`SendMediaError::Mux`] (excluding *some* tracks while carrying
    /// others is not an error — see the warn-once note below).
    fn encode_media(&mut self, media: &Media) -> Result<Vec<bytes::Bytes>, SendMediaError> {
        let flv_tracks: Vec<Track> = media
            .tracks
            .iter()
            .filter(|t| is_flv_codec(&t.spec.config))
            .cloned()
            .collect();

        // Refusing a track the wire format cannot carry is legitimate;
        // refusing it *silently* is not. Report it once per connection — a
        // per-batch log would spam, and no report at all is the same
        // data-loss hazard as `transmux`'s FLV demux dropping non-AVC/AAC
        // tracks with no event. `push::egress::PushTransportEgress::negotiate`
        // reports this same fact structurally (via `NegotiationOutcome`) at
        // connect time; this warn covers the direct-`PushTransport` caller
        // that never negotiates at all (e.g. `drive_push`'s pre-#942 shape,
        // or a test driving this transport by hand).
        let refused = media.tracks.len() - flv_tracks.len();
        if refused > 0 && !self.warned_refused_tracks {
            self.warned_refused_tracks = true;
            let refused_track_ids: Vec<u32> = media
                .tracks
                .iter()
                .filter(|t| !is_flv_codec(&t.spec.config))
                .map(|t| t.spec.track_id)
                .collect();
            tracing::warn!(
                refused,
                carried = flv_tracks.len(),
                ?refused_track_ids,
                "RTMP push cannot carry these tracks — FLV carries only AVC video \
                 and AAC audio; they are excluded from this push"
            );
        }

        // Every track refused means this push transmits nothing, for as long
        // as the track set stays this way. Reporting an empty batch would
        // present a permanently useless push as a working one. `Mux` is the
        // right class: it drops the batch without triggering a reconnect,
        // because reconnecting cannot fix a codec mismatch.
        if flv_tracks.is_empty() {
            return Err(SendMediaError::Mux(format!(
                "no RTMP-carriable track: FLV carries only AVC video and AAC audio, \
                 but all {} track(s) in this program are other codecs",
                media.tracks.len()
            )));
        }
        let filtered = Media::new(flv_tracks, media.movie_timescale);
        let payloads = transmux::flv_frame_payloads(&filtered)
            .map_err(|e| SendMediaError::Mux(e.to_string()))?;

        let mut messages = Vec::with_capacity(payloads.len());
        for payload in &payloads {
            // `FlvPayloadKind` is `#[non_exhaustive]`: only `Video`/`Audio`
            // exist today; a future kind is silently skipped here rather
            // than sent as neither.
            let sent = match payload.kind {
                transmux::FlvPayloadKind::Video => {
                    Some(self.client.send_video(payload.timestamp_ms, &payload.body))
                }
                transmux::FlvPayloadKind::Audio => {
                    Some(self.client.send_audio(payload.timestamp_ms, &payload.body))
                }
                _ => None,
            };
            let Some(bytes) = sent else { continue };
            let bytes = bytes.map_err(|e| {
                SendMediaError::Transport(Box::new(RtmpPushError::Protocol(e.to_string())))
            })?;
            messages.push(bytes::Bytes::from(bytes));
        }
        Ok(messages)
    }

    /// Encodes `media` via [`encode_media`](Self::encode_media) then writes
    /// each resulting chunk-framed RTMP message straight to the socket —
    /// bypasses [`send`](Self::send) (which would re-frame the bytes as a
    /// *new* video message) and the trait's default `TsMux` path entirely.
    async fn send_media(&mut self, media: &Media) -> Result<u64, SendMediaError> {
        let messages = self.encode_media(media)?;
        let stream = self.stream.as_mut().ok_or_else(|| {
            SendMediaError::Transport(Box::new(RtmpPushError::Connect("not connected".into())))
        })?;
        let mut total = 0u64;
        for message in &messages {
            stream
                .write_all(message)
                .await
                .map_err(|e| SendMediaError::Transport(Box::new(RtmpPushError::Io(e))))?;
            total += message.len() as u64;
        }
        Ok(total)
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

/// Errors from the RTMP push transport.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtmpPushError {
    /// Connection failed.
    #[error("RTMP connect failed: {0}")]
    Connect(String),
    /// Protocol error.
    #[error("RTMP protocol error: {0}")]
    Protocol(String),
    /// I/O error.
    #[error("RTMP I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avc_config() -> CodecConfig {
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(transmux::AVCDecoderConfigurationRecord {
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
            stream_type: 0x06,
            descriptors: Vec::new(),
            carriage: transmux::ir::DataCarriage::Pes,
        }
    }

    /// `PushTransport::supports_codec` (issue #942) must delegate to the
    /// real `is_flv_codec` predicate — the one place `push::egress::
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
    fn supports_codec_matches_is_flv_codec() {
        let transport = RtmpTransport {
            stream: None,
            client: rtmp_runtime::client::ClientSession::new(ClientConfig::default()),
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
}
