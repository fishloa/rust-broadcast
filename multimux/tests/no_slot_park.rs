//! B10b / SP1.4: a push that finds no free `Trunk` waiter slot parks on the
//! slot-release event (raced with cancel) instead of a fixed 50 ms sleep-poll.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{PushTransport, drive_push};

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
    async fn connect(_u: &str, _c: &Self::Config) -> Result<Self, Self::Error> {
        Err(NeverErr)
    }
    async fn send(&mut self, _d: &[u8]) -> Result<(), Self::Error> {
        Err(NeverErr)
    }
    fn close(&mut self) {}
}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

/// With every waiter slot held, `trunk.listen()` returns `None`; cancelling the
/// push must still return promptly (it parks on the slot-release event, raced
/// against cancel — never a fixed sleep).
#[tokio::test]
async fn a_push_with_no_free_waiter_slot_returns_promptly_on_cancel() {
    let trunk = Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)));
    let held = trunk.listen().expect("the sole waiter slot is free here");
    assert_eq!(trunk.waiter_count(), 1);

    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(drive_push::<NeverConnect>(
        Arc::clone(&trunk),
        "srt://127.0.0.1:9/streamid=x".into(),
        (),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    // Let the loop reach the (occupied) listen, then cancel.
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_secs(2), h).await;
    assert!(done.is_ok(), "cancel must return the parked push: {done:?}");
    drop(held);
}

/// Freeing a held slot wakes a parked push, which then re-registers a listener
/// (i.e. the slot-release event is what the no-slot branch waits on, not a
/// fixed timer).
#[tokio::test]
async fn freeing_a_slot_wakes_a_parked_push() {
    let trunk = Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)));
    let held = trunk.listen().expect("the sole waiter slot is free here");

    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(drive_push::<NeverConnect>(
        Arc::clone(&trunk),
        "srt://127.0.0.1:9/streamid=x".into(),
        (),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Free the slot: the parked push must take it.
    drop(held);
    // The push re-registers a listener; `waiter_count` goes back to 1.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if trunk.waiter_count() >= 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "freeing the slot must wake the parked push to re-register a listener"
        );
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}
