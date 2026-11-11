//! Helpers shared by more than one integration test file under `tests/it/`.
//! Each one exists here and nowhere else under `tests/it/`.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Serializes every test that installs the process-wide tracing subscriber.
///
/// More than one test binary needs a subscriber that sees *every* event
/// (`push_log_redaction` asserts a log line never carries a secret), and
/// `tracing` allows exactly one global subscriber per process: whichever
/// installs first wins and the others silently capture nothing, and a
/// scoped `set_default` misses an event logged on another thread. So the
/// capture is global, and this lock plus [`tracing_capture`]'s one-shot
/// installer keep it correct no matter what order the tests run in.
pub fn tracing_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Every tracing event's fields, as `name=value ` text, once
/// [`install_tracing_capture`] has run.
static TRACING_CAPTURE: OnceLock<std::sync::Arc<Mutex<String>>> = OnceLock::new();

/// Install the process-wide capture subscriber (if it is not installed yet)
/// and clear the buffer. Callers hold [`tracing_lock`], so the events are
/// the test's own.
pub fn install_tracing_capture() {
    let buffer = TRACING_CAPTURE.get_or_init(|| std::sync::Arc::new(Mutex::new(String::new())));
    let _ = tracing::subscriber::set_global_default(CaptureEvents(buffer.clone()));
    clear_captured_events();
}

/// The installed capture's events so far.
pub fn captured_events() -> String {
    TRACING_CAPTURE
        .get()
        .map(|buffer| {
            buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        })
        .unwrap_or_default()
}

/// Clears the capture buffer, so a test reads only what it caused.
pub fn clear_captured_events() {
    if let Some(buffer) = TRACING_CAPTURE.get() {
        buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

/// Records every tracing event's fields as `name=value ` text.
struct CaptureEvents(std::sync::Arc<Mutex<String>>);

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
        use std::fmt::Write as _;
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

pub const HANG_GUARD: Duration = Duration::from_secs(20);
pub const OBSERVE_CEILING: Duration = Duration::from_secs(30);

pub fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

pub fn fixture_path(name: &str) -> PathBuf {
    test_fixtures::path(name)
}

pub fn fixture_string(name: &str) -> String {
    fixture_path(name).to_string_lossy().into_owned()
}

pub async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no #EXTINF: line appeared in {playlist_url} within the hang guard");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn poll_until_extinf_yield(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + HANG_GUARD;
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no #EXTINF: line appeared in {playlist_url} within the hang guard");
        }
        tokio::task::yield_now().await;
    }
}

pub async fn observe_extinf(client: &reqwest::Client, playlist_url: &str) -> Option<String> {
    let deadline = tokio::time::Instant::now() + OBSERVE_CEILING;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while tokio::time::Instant::now() < deadline {
        tick.tick().await;
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return Some(body);
        }
    }
    None
}
