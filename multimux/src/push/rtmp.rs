//! RTMP push transport — client publish to a remote RTMP server.
//!
//! Adobe RTMP 1.0 §7.2 (NetConnection/NetStream commands).

use crate::push::PushTransport;
use rtmp_runtime::client::{ClientConfig, ClientSession};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
