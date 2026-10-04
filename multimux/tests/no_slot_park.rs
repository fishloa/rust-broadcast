//! B10b / SP1.4: a push that finds no free `Trunk` waiter slot parks on the
//! slot-release event (raced with cancel) instead of a fixed 50 ms sleep-poll.
//!
//! Both tests run under `start_paused`, with NO real-time sleep used for
//! synchronisation: virtual time only advances when the runtime is otherwise
//! idle, so an implementation that parked on a fixed timer would be forced to
//! wait for the timer to fire rather than wake the instant the slot is freed —
//! making "event-driven" observably different from "50 ms poll". The parking
//! point is the `trunk.listen() == None` branch; `NeverConnect` keeps every
//! other step free of a real async wait (zero backoff skips the retry wait),
//! so the only place the loop can park is that branch.

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

/// Zero backoff so the retry-wait is skipped and the only parking point in
/// `drive_push` is the `None` (no free waiter slot) branch.
fn zero_backoff() -> ReconnectPolicy {
    ReconnectPolicy {
        initial_backoff_ms: 0,
        max_backoff_ms: 0,
        max_attempts: None,
    }
}

/// With every waiter slot held, `trunk.listen()` returns `None`; cancelling the
/// push must still return promptly. The parking await is raced against `cancel`
/// inside `drive_push`'s own `select!`, so this resolves without advancing
/// virtual time at all.
#[tokio::test(start_paused = true)]
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
        zero_backoff(),
        cancel.clone(),
    ));

    // Let the loop reach the (occupied) listen and park there. Under paused
    // time a `yield_now` spin does not advance the clock, so the push cannot
    // get past a fixed-timer park this way — it has genuinely parked.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    cancel.cancel();
    // The cancel arm of `drive_push`'s select resolves without any timer, so
    // this completes with virtual time frozen.
    let done = tokio::time::timeout(Duration::from_secs(2), h).await;
    assert!(done.is_ok(), "cancel must return the parked push: {done:?}");
    drop(held);
}

/// Freeing a held slot wakes a parked push, which then re-registers a listener
/// — i.e. the no-slot branch waits on the slot-release event, NOT a fixed
/// timer.
///
/// DISCRIMINATES EVENT-DRIVEN FROM 50 ms POLL: this test never advances virtual
/// time after the push parks. `drop(held)` fires the slot-release notify
/// synchronously; an event-driven push re-registers on its next poll, while a
/// 50 ms-sleep-poll push would need virtual time to advance to 50 ms — which
/// this test never does — and so would never re-register within the bounded
/// spin below.
#[tokio::test(start_paused = true)]
async fn freeing_a_slot_wakes_a_parked_push() {
    let trunk = Trunk::new(TrunkConfig::new(nz(1), nz(1), nz(1), nz(1), nz(1)));
    let held = trunk.listen().expect("the sole waiter slot is free here");

    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(drive_push::<NeverConnect>(
        Arc::clone(&trunk),
        "srt://127.0.0.1:9/streamid=x".into(),
        (),
        PushFormat::Ts,
        zero_backoff(),
        cancel.clone(),
    ));

    // Let the push reach the occupied listen and park (see the note in the
    // test above: a yield spin does not advance virtual time).
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        trunk.waiter_count(),
        1,
        "the push has parked, not registered"
    );

    // Free the slot: the parked push must take it, with virtual time frozen.
    drop(held);
    let mut re_registered = false;
    for _ in 0..1000 {
        if trunk.waiter_count() >= 1 {
            re_registered = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        re_registered,
        "freeing the slot must wake the parked push to re-register a listener \
         without any timer advancing (event-driven, not a 50 ms poll)"
    );
    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
}
