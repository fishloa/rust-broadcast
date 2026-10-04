//! Defect 5 / SP1.4: a push reconnect must ignore shutdown — cancelling during
//! the connect or the backoff sleep must return promptly.
//!
//! Both tests drive the loop to the *exact* arm under test before cancelling,
//! rather than advancing a fixed 400 ms under `start_paused` (which only runs
//! the spawned task AFTER the advance, so it parks in the 250 ms listen-wait
//! instead). The transport shares an `AtomicUsize` connect counter; the test
//! `yield_now`s until that counter shows `T::connect` was entered, so the
//! cancel lands while the task is genuinely parked inside the connect (or,
//! for the backoff test, until the first connect has failed and the loop is
//! in the backoff sleep). A revert that removes the connect/backoff `select!`
//! then hangs the hanging-connect transport / waits out the 1 s backoff and
//! the bounded `timeout` fails the test.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::{PushFormat, ReconnectPolicy};
use multimux::push::{PushTransport, drive_push};

#[derive(Debug)]
struct NeverErr;
impl std::error::Error for NeverErr {}
impl std::fmt::Display for NeverErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "never connects")
    }
}

/// A `PushTransport` whose `connect` NEVER returns, while incrementing a
/// shared counter the instant it is entered — so `drive_push` parks inside a
/// connect the test can observe, exactly the "connect blocks for its own
/// (much longer) timeout" case defect 5 is about.
struct HangingConnect;

#[async_trait::async_trait]
impl PushTransport for HangingConnect {
    type Config = Arc<AtomicUsize>;
    type Error = NeverErr;
    async fn connect(_url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        config.fetch_add(1, Ordering::SeqCst);
        std::future::pending::<()>().await;
        unreachable!()
    }
    async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
        Err(NeverErr)
    }
    fn close(&mut self) {}
}

/// A `PushTransport` whose `connect` always fails immediately (no real I/O,
/// no `.await` suspension), while counting its attempts — so a test can tell
/// the first connect has failed and the loop is in the reconnect backoff.
struct NeverConnect;

#[async_trait::async_trait]
impl PushTransport for NeverConnect {
    type Config = Arc<AtomicUsize>;
    type Error = NeverErr;
    async fn connect(_url: &str, config: &Self::Config) -> Result<Self, Self::Error> {
        config.fetch_add(1, Ordering::SeqCst);
        Err(NeverErr)
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

/// Advance virtual time in 250 ms steps (the push loop's own listen-wait cap)
/// with a `yield_now` after each, until `cond` holds — so the test drives the
/// loop to the arm under test, letting the listen-wait tick complete and the
/// loop iterate forward. Never sleeps (the clock is paused). Panics if the
/// loop never gets there.
async fn advance_until(what: &str, mut cond: impl FnMut() -> bool) {
    const MAX: u32 = 100;
    for _ in 0..MAX {
        if cond() {
            return;
        }
        tokio::time::advance(Duration::from_millis(250)).await;
        tokio::task::yield_now().await;
    }
    panic!("the push loop never reached {what} within {MAX} 250 ms steps");
}

/// Cancelling while the push is BLOCKED in a connect must return promptly —
/// the connect is raced against the token, not awaited to its own timeout.
///
/// The hanging connect never returns on its own, so only the cancel race can
/// make the task exit: yield until the shared connect counter shows the
/// connect was entered, then cancel. Deleting the connect `select!` (awaiting
/// `T::connect` directly) makes this hang and the bounded timeout fails.
#[tokio::test(start_paused = true)]
async fn cancelling_during_a_push_connect_returns_promptly() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let connects = Arc::new(AtomicUsize::new(0));
    let h = tokio::spawn(drive_push::<HangingConnect>(
        test_trunk(),
        "srt://127.0.0.1:9/streamid=x".into(),
        Arc::clone(&connects),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    advance_until("the hanging connect", || {
        connects.load(Ordering::SeqCst) > 0
    })
    .await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_secs(2), h).await;
    assert!(
        done.is_ok(),
        "cancel must abort a blocked connect, not await its own timeout: {done:?}"
    );
}

/// Cancelling during the reconnect backoff must also return promptly.
///
/// `NeverConnect` fails instantly and repeatedly, so the loop lands in the
/// backoff sleep once the first attempt is counted. Yield until that count is
/// seen (the loop has entered — or is about to enter — the backoff), then
/// cancel. Removing the backoff `select!` (awaiting `sleep(wait)` directly)
/// makes the resolve wait out the 1 s initial backoff; the bounded timeout
/// fails.
#[tokio::test(start_paused = true)]
async fn cancelling_during_a_push_backoff_wait_returns_promptly() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let connects = Arc::new(AtomicUsize::new(0));
    let h = tokio::spawn(drive_push::<NeverConnect>(
        test_trunk(),
        "srt://127.0.0.1:9/streamid=x".into(),
        Arc::clone(&connects),
        PushFormat::Ts,
        ReconnectPolicy::default(),
        cancel.clone(),
    ));
    advance_until("the first failed connect", || {
        connects.load(Ordering::SeqCst) > 0
    })
    .await;
    // One more yield so the loop has run past the failure and reached the
    // backoff sleep.
    tokio::task::yield_now().await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_millis(200), h).await;
    assert!(
        done.is_ok(),
        "cancel must abort the backoff sleep, not wait out the next retry: {done:?}"
    );
}
