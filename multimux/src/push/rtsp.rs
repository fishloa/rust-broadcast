//! RTSP push transport — client ANNOUNCE/RECORD to a remote RTSP server.
//!
//! RFC 2326 §10.3 (ANNOUNCE), §10.11 (RECORD). Sends TS-muxed data over
//! interleaved TCP framing (§10.12).

use crate::push::PushTransport;
use rtsp_runtime::client::ClientSession;
use rtsp_runtime::interleaved::InterleavedFrame;
use rtsp_runtime::transport::{Transport, TransportSpec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use transmux::ir::TrackSpec;

/// Per-connection configuration for the RTSP push transport.
#[derive(Debug, Clone, Default)]
pub struct RtspTransportConfig {
    /// Optional credentials for RTSP auth.
    pub credentials: Option<(String, String)>,
}

/// The RTSP push transport: an owned TCP connection + [`ClientSession`].
pub struct RtspTransport {
    stream: Option<TcpStream>,
    client: ClientSession,
    channel: u8,
}

impl std::fmt::Debug for RtspTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspTransport")
            .field("connected", &self.stream.is_some())
            .finish()
    }
}

#[async_trait::async_trait]
impl PushTransport for RtspTransport {
    type Config = RtspTransportConfig;
    type Error = RtspPushError;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let parsed = url::Url::parse(url).map_err(|e| RtspPushError::Connect(e.to_string()))?;
        let host = parsed.host_str().unwrap_or("127.0.0.1");
        let port = parsed.port().unwrap_or(554);
        let addr = format!("{host}:{port}");

        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| RtspPushError::Connect(e.to_string()))?;

        let mut client = ClientSession::new();
        if let Some((user, pass)) = &config.credentials {
            client = client.with_credentials(rtsp_runtime::auth::Credentials::new(
                user.clone(),
                pass.clone(),
            ));
        }

        let options_bytes = client
            .options(url)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        stream
            .write_all(&options_bytes)
            .await
            .map_err(RtspPushError::Io)?;

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.map_err(RtspPushError::Io)?;
        let _events = client
            .handle_data(&buf[..n])
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;

        Ok(Self {
            stream: Some(stream),
            client,
            channel: 0,
        })
    }

    async fn setup(&mut self, tracks: &[TrackSpec]) -> Result<(), Self::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;

        let url = "rtsp://localhost/push";
        let sdp = build_sdp(tracks);
        let announce_bytes = self
            .client
            .announce(url, &sdp)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        stream
            .write_all(&announce_bytes)
            .await
            .map_err(RtspPushError::Io)?;

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.map_err(RtspPushError::Io)?;
        let _events = self
            .client
            .handle_data(&buf[..n])
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;

        let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
            self.channel,
            self.channel + 1,
        ));
        let setup_bytes = self
            .client
            .setup(url, &transport)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        stream
            .write_all(&setup_bytes)
            .await
            .map_err(RtspPushError::Io)?;

        let n = stream.read(&mut buf).await.map_err(RtspPushError::Io)?;
        let _events = self
            .client
            .handle_data(&buf[..n])
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;

        let record_bytes = self
            .client
            .record(url)
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        stream
            .write_all(&record_bytes)
            .await
            .map_err(RtspPushError::Io)?;

        let n = stream.read(&mut buf).await.map_err(RtspPushError::Io)?;
        let _events = self
            .client
            .handle_data(&buf[..n])
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;

        Ok(())
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| RtspPushError::Connect("not connected".into()))?;
        let frame = InterleavedFrame::new(self.channel, data);
        let bytes = frame
            .to_bytes()
            .map_err(|e| RtspPushError::Protocol(e.to_string()))?;
        stream.write_all(&bytes).await.map_err(RtspPushError::Io)
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

fn build_sdp(tracks: &[TrackSpec]) -> String {
    use transmux::CodecConfig;
    let mut sdp = String::from("v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Push\r\nt=0 0\r\n");
    for spec in tracks {
        let media = match &spec.config {
            CodecConfig::Avc { .. }
            | CodecConfig::Hevc { .. }
            | CodecConfig::Vvc { .. }
            | CodecConfig::Av1 { .. }
            | CodecConfig::Vp9 { .. }
            | CodecConfig::Vp8 { .. }
            | CodecConfig::Mpeg2Video { .. } => "video",
            _ => "audio",
        };
        sdp.push_str(&format!("m={media} 0 RTP/AVP 96\r\n"));
    }
    sdp
}

/// Errors from the RTSP push transport.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RtspPushError {
    #[error("RTSP connect failed: {0}")]
    Connect(String),
    #[error("RTSP protocol error: {0}")]
    Protocol(String),
    #[error("RTSP I/O error: {0}")]
    Io(#[from] std::io::Error),
}
