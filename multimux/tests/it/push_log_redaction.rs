//! Audit T14 (#1142): a push destination's credentials (URL userinfo) and
//! stream key (path/query) must never reach a log line; the destination is
//! logged as `scheme://host[:port]/<redacted>`.
//!
//! The capture is the one process-wide subscriber
//! (`support::shared_it::install_tracing_capture`, which every test needing
//! one shares); a scoped `set_default` cannot serve here, because
//! `tracing` caches each callsite's "is anyone listening" verdict per thread
//! and this test's events are logged on tokio worker threads of its own
//! runtime. The tests that install a subscriber serialize on
//! `support::shared_it::tracing_lock`, so no two of them interleave, and the
//! assertions read only this test's own destination lines out of the buffer
//! (other tests may still log into it, which is why the filter exists).

use std::time::Duration;

use crate::support::shared_it::nz;
use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::srt::srt_url_outcome_for_test;
use multimux::push::{
    PushTransport, RtmpTransport, RtmpTransportConfig, RtspTransport, RtspTransportConfig,
    SrtTransport, SrtTransportConfig, drive_push,
};
use tokio_util::sync::CancellationToken;

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
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(1))
            .with_sparse_capacity(nz(1))
            .with_segment_capacity(nz(1))
            .with_event_capacity(nz(1))
            .with_part_capacity(nz(1)),
    );
    let reconnect = ReconnectPolicy::default()
        .with_initial_backoff_ms(1)
        .with_max_backoff_ms(1)
        .with_max_attempts(2);
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

#[test]
fn push_logs_and_errors_never_contain_the_destination_secrets() {
    // A discarded `srt://` URL's destination is masked as a bare
    // `<redacted>` (the parser rejects an empty authority), so it carries no
    // host to filter on; identify its lines by the reason instead.
    let srt_reason = srt_url_outcome_for_test("srt://?streamid=SECRETSTREAM&passphrase=SECRETPASS");
    let srt_reason = srt_reason.trim_start_matches("err ").to_string();

    let _serial = crate::support::shared_it::tracing_lock();
    crate::support::shared_it::install_tracing_capture();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime");
    runtime.block_on(async {
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
    });

    let all = crate::support::shared_it::captured_events();
    // The capture is process-wide, so other tests' events are in the buffer
    // too; this test's evidence is exactly its own destinations' log lines.
    let logs: String = all
        .split("message=")
        .filter(|line| {
            line.contains("push connect failed")
                && (line.contains("rtmp://127.0.0.1:1/")
                    || line.contains("rtsp://127.0.0.1:1/")
                    || line.contains("echo.example:1935")
                    || line.contains(srt_reason.as_str()))
        })
        .collect();
    assert_eq!(
        logs.matches("push connect failed").count(),
        8,
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
