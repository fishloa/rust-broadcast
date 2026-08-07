//! The SRT push transport (issue #744 Phase 1) — a thin wrapper over
//! [`srt_runtime::io::SrtSocket`] in **Caller** mode: dials out to a
//! downstream SRT Listener and pushes muxed TS payloads to it.
//!
//! SRT is the first concrete [`PushTransport`] (see `super`): the simplest
//! push wire protocol this crate could start with — a single
//! [`srt_runtime::io::SrtSocket::connect`] handshake and a
//! [`send`](SrtTransport#method.send) per muxed batch, with no session
//! protocol (no RTMP connect/publish, no RTSP ANNOUNCE/RECORD) to run before
//! the first payload.

use crate::push::PushTransport;
use srt_runtime::HandshakeConfig;
use srt_runtime::io::SrtSocket;

/// Per-connection configuration for the SRT push transport — the SRT
/// handshake config dialed out with (latency, MTU, …).
#[derive(Debug, Clone, Default)]
pub struct SrtTransportConfig {
    /// The SRT handshake configuration used for
    /// [`SrtSocket::connect`](srt_runtime::io::SrtSocket::connect).
    pub srt_config: HandshakeConfig,
}

/// The SRT push transport: an owned [`SrtSocket`] in Caller mode.
///
/// The socket is held as `Option` so [`close`](SrtTransport::close) can
/// actually tear the connection down (dropping the owned handle aborts the
/// SRT driver task) rather than only dropping at the enclosing scope.
pub struct SrtTransport {
    socket: Option<SrtSocket>,
}

impl std::fmt::Debug for SrtTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtTransport")
            .field("connected", &self.socket.is_some())
            .finish()
    }
}

#[async_trait::async_trait]
impl PushTransport for SrtTransport {
    type Config = SrtTransportConfig;
    type Error = srt_runtime::Error;

    async fn connect(url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        let addr = url.strip_prefix("srt://").unwrap_or(url);
        let socket = SrtSocket::connect(addr, config.srt_config.clone()).await?;
        Ok(Self {
            socket: Some(socket),
        })
    }

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let socket = self.socket.as_mut().ok_or(srt_runtime::Error::Io {
            kind: std::io::ErrorKind::NotConnected,
            context: "push send",
        })?;
        socket.send(data).await
    }

    fn close(&mut self) {
        // Dropping the owned SrtSocket handle aborts its driver task.
        self.socket = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srt_runtime::io::SrtListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// SRT loopback (issue #744): spawn a real test-owned `SrtListener` that
    /// accepts one Caller, then connect an `SrtTransport` (Caller mode) to it
    /// and push bytes — verifying the listener actually receives them. The
    /// `SrtSocket` data plane is delivery-guaranteed over loopback UDP, so
    /// the receiver-side `recv` poll is the assertion.
    #[tokio::test]
    async fn srt_transport_pushes_bytes_to_a_listener() {
        const PAYLOAD: &[u8] = &[0x47, 0x40, 0x00, 0x10, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00];
        let received: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let received_for_task = Arc::clone(&received);

        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("bind");
        let bound = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            let mut sock = match tokio::time::timeout_at(deadline, listener.accept()).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => panic!("accept failed: {e}"),
                Err(_) => panic!("accept timed out"),
            };
            // Receive until we see our payload or time out.
            while tokio::time::Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_millis(200), sock.recv()).await {
                    Ok(Ok(Some(data))) if data.as_slice() == PAYLOAD => {
                        received_for_task.store(true, Ordering::SeqCst);
                        break;
                    }
                    Ok(Ok(Some(_))) => continue,
                    Ok(Ok(None)) => break,
                    Ok(Err(e)) => panic!("recv failed: {e}"),
                    Err(_) => continue,
                }
            }
        });

        let cfg = SrtTransportConfig::default();
        // HANG GUARD (workspace precedent, issue #826): bound the dial so a
        // never-connecting caller fails rather than hangs.
        let transport = tokio::time::timeout(
            Duration::from_secs(30),
            SrtTransport::connect(&format!("srt://{bound}"), &cfg),
        )
        .await
        .expect("connect must not hang")
        .expect("connect");

        let mut transport = transport;
        transport.send(PAYLOAD).await.expect("send");
        // Keep `transport` alive so its SRT driver task flushes the payload
        // to the listener (dropping the handle aborts the driver mid-flight).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !received.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        transport.close();
        server.abort();
        assert!(
            received.load(Ordering::SeqCst),
            "downstream SRT listener must receive the pushed bytes"
        );
    }
}
