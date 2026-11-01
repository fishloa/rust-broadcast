//! Audit T14 (#1142): a push destination's credentials (URL userinfo) and
//! stream key (path/query) must never reach a log line; the destination is
//! logged as `scheme://host[:port]/<redacted>`.
//!
//! This is its own test binary, with exactly one test, on purpose: `tracing`
//! caches each callsite's "is anyone listening" verdict process-wide, and
//! another test thread touching the same callsite concurrently made a
//! scoped capture subscriber miss events when this lived among the lib tests.

use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{
    PushTransport, RtmpTransport, RtmpTransportConfig, RtspTransport, RtspTransportConfig,
    SrtTransport, SrtTransportConfig, drive_push,
};
use tokio_util::sync::CancellationToken;

/// Records every tracing event's fields as `name=value ` text.
struct CaptureEvents(Arc<Mutex<String>>);

impl tracing::Subscriber for CaptureEvents {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields<'a>(&'a mut String);
        impl tracing::field::Visit for Fields<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let _ = write!(self.0, "{}={value:?} ", field.name());
            }
        }
        let mut line = String::new();
        event.record(&mut Fields(&mut line));
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_str(&line);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

/// A transport whose connect error echoes the URL it was given — the worst
/// case a real transport's `Display` could be.
struct EchoesTheUrl;

#[derive(Debug, thiserror::Error)]
#[error("cannot dial {0}")]
struct EchoError(String);

#[async_trait::async_trait]
impl PushTransport for EchoesTheUrl {
    type Config = ();
    type Error = EchoError;

    async fn connect(url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
        Err(EchoError(url.to_string()))
    }

    async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    fn close(&mut self) {}
}

const SECRETS: [&str; 6] = [
    "hunter2",
    "admin",
    "STREAMKEY123",
    "token=abc",
    "SECRETSTREAM",
    "SECRETPASS",
];

fn drive<T: PushTransport>(url: &str, config: T::Config) -> impl std::future::Future<Output = ()>
where
    T::Config: Send + 'static,
{
    let trunk = Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)));
    let reconnect = ReconnectPolicy {
        initial_backoff_ms: 1,
        max_backoff_ms: 1,
        max_attempts: Some(2),
    };
    let url = url.to_string();
    async move {
        tokio::time::timeout(
            Duration::from_secs(30),
            drive_push::<T>(
                trunk,
                url,
                config,
                PushFormat::Ts,
                reconnect,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("drive_push gives up after max_attempts");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn push_logs_and_errors_never_contain_the_destination_secrets() {
    let captured = Arc::new(Mutex::new(String::new()));
    let _guard = tracing::subscriber::set_default(CaptureEvents(Arc::clone(&captured)));

    // RTMP and RTSP against a port nothing listens on; SRT with an invalid
    // URL (the parse failure is the connect error); and a transport whose
    // error text echoes the whole URL.
    drive::<RtmpTransport>(
        "rtmp://admin:hunter2@127.0.0.1:1/app/STREAMKEY123?token=abc",
        RtmpTransportConfig::default(),
    )
    .await;
    drive::<RtspTransport>(
        "rtsp://admin:hunter2@127.0.0.1:1/live/STREAMKEY123?token=abc",
        RtspTransportConfig::default(),
    )
    .await;
    drive::<SrtTransport>(
        "srt://?streamid=SECRETSTREAM&passphrase=SECRETPASS",
        SrtTransportConfig::default(),
    )
    .await;
    drive::<EchoesTheUrl>(
        "rtmp://admin:hunter2@echo.example:1935/app/STREAMKEY123?token=abc",
        (),
    )
    .await;

    let logs = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        logs.matches("push connect failed").count() >= 8,
        "every transport's failed connect must have been logged twice: {logs:?}"
    );
    assert!(logs.contains("rtmp://127.0.0.1:1/<redacted>"), "{logs}");
    assert!(logs.contains("rtsp://127.0.0.1:1/<redacted>"), "{logs}");
    assert!(
        logs.contains("rtmp://echo.example:1935/<redacted>"),
        "{logs}"
    );
    for secret in SECRETS {
        assert!(!logs.contains(secret), "{secret} leaked into: {logs}");
    }
}
