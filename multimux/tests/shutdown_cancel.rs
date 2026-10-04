//! Defect 5 / SP1.4: a push reconnect must ignore shutdown — cancelling during
//! the connect or the backoff sleep must return promptly.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{PushTransport, drive_push};

/// A `PushTransport` whose `connect` always fails, so `drive_push` lands in
/// the reconnect backoff after its first attempt.
struct NeverConnect;

#[derive(Debug)]
struct NeverErr;
impl std::error::Error for NeverErr {}
impl std::fmt::Display for NeverErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "never connects")
    }
}

#[async_trait::async_trait]
impl PushTransport for NeverConnect {
    type Config = ();
    type Error = NeverErr;
    async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
        Err(NeverErr)
    }
    async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
        Err(NeverErr)
    }
    fn close(&mut self) {}
}

/// A `PushTransport` whose `connect` NEVER returns — so `drive_push` parks
/// inside the connect, exactly the "connect blocks for its own (much longer)
/// timeout" case defect 5 is about.
struct HangingConnect;

#[async_trait::async_trait]
impl PushTransport for HangingConnect {
    type Config = ();
    type Error = NeverErr;
    async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
        std::future::pending::<()>().await;
        unreachable!()
    }
    async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
        Err(NeverErr)
    }
    fn close(&mut self) {}
}

/// `TrunkConfig` has no `Default` (five `NonZeroUsize` capacities).
fn test_trunk() -> Arc<Trunk> {
    let cap = NonZeroUsize::new(8).unwrap();
    Trunk::new(TrunkConfig::new(cap, cap, cap, cap, cap))
}

/// Cancelling while the push is BLOCKED in a connect must return promptly —
/// the connect is raced against the token, not awaited to its own timeout.
#[tokio::test]
async fn cancelling_during_a_push_connect_returns_promptly() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(drive_push::<HangingConnect>(
        test_trunk(),
        "srt://127.0.0.1:9/streamid=x".into(),
        (),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    // Let the loop reach the (hanging) connect (one 250 ms listen tick), then
    // cancel. Real time: the connect never returns on its own, so only the
    // cancel race can make the task exit.
    tokio::time::sleep(Duration::from_millis(400)).await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_secs(2), h).await;
    assert!(
        done.is_ok(),
        "cancel must abort a blocked connect, not await its own timeout: {done:?}"
    );
}

/// Cancelling during the reconnect backoff must also return promptly.
#[tokio::test]
async fn cancelling_during_a_push_backoff_wait_returns_promptly() {
    let cancel = tokio_util::sync::CancellationToken::new();
    // A 1 s initial backoff: cancel at 400 ms (mid-backoff) must return well
    // before the next retry at ~1.25 s.
    let h = tokio::spawn(drive_push::<NeverConnect>(
        test_trunk(),
        "srt://127.0.0.1:9/streamid=x".into(),
        (),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(400)).await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_millis(200), h).await;
    assert!(
        done.is_ok(),
        "cancel must abort the backoff sleep, not wait out the next retry: {done:?}"
    );
}
