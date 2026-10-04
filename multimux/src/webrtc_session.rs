//! Shared WHIP/WHEP session limits and the RAII capacity slot.
//!
//! The bounded HTTP request *reader* that used to live beside these moved out
//! with the WHIP/WHEP listeners onto axum `Router`s (W2a SP2.1); what remains
//! here is the part that is not HTTP at all: the per-listener session cap and
//! the RAII slot that hands a reservation off to a long-lived session.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Bound on concurrent in-flight signalling requests for one WHIP/WHEP
/// listener (a shared `GlobalConcurrencyLimitLayer` request-concurrency pool,
/// not a TCP connection cap — see `serve_hyper_util`'s `MAX_CONNECTIONS` for
/// that). A flood of requests that never complete cannot spawn an unbounded
/// number of concurrent handler tasks.
pub(crate) const MAX_PENDING_HTTP_CONNECTIONS: usize = 256;

/// Body cap for a WHIP/WHEP signalling request (SDP offer/answer). A request
/// claiming a larger body is rejected `413` before any body byte is read.
pub(crate) const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;

/// Bounds an entire signalling request (headers + body) — a peer that stops
/// sending mid-request is closed rather than held open indefinitely.
pub(crate) const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Minimum floor between two consecutive [`MediaTransport::handle_timeout`]
/// calls (finding N3): a transport whose `poll_timeout` keeps returning an
/// instant at or before now (an ICE/DTLS error state) would otherwise fire
/// `handle_timeout` back-to-back and spin a core. An overdue deadline still
/// fires immediately *on the first* call; only the *next* is floored.
pub(crate) const MIN_TIMER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// Absolute instant at which a transport timer with `deadline` should fire:
/// `max(deadline, last_fire + MIN_TIMER_INTERVAL)`. `None` means never.
/// Absolute (not a duration measured from `last_fire`), so rebuilding the
/// sleep on every loop iteration (each inbound datagram) cannot push the fire
/// time back.
pub(crate) fn timer_fire_at(
    deadline: Option<std::time::Instant>,
    last_fire: std::time::Instant,
) -> Option<tokio::time::Instant> {
    deadline.map(|d| tokio::time::Instant::from_std(d.max(last_fire + MIN_TIMER_INTERVAL)))
}

/// Sleep until [`timer_fire_at`], or forever when there is no deadline.
pub(crate) fn sleep_until_timer(
    deadline: Option<std::time::Instant>,
    last_fire: std::time::Instant,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    match timer_fire_at(deadline, last_fire) {
        None => Box::pin(std::future::pending()),
        Some(at) => Box::pin(tokio::time::sleep_until(at)),
    }
}

/// RAII capacity slot for a WHIP/WHEP session (issue r07-C11 follow-up):
/// [`Self::acquire`] does the `fetch_add`/cap-check/`fetch_sub`-on-refusal
/// dance once, in one place, and — critically — releases the slot on
/// `Drop` if it is ever dropped still armed. Without this, any `?` between
/// a successful `acquire` and the point a session is finally handed off to
/// its long-lived owner (an admitted-session channel send, in both WHIP and
/// WHEP) leaked the slot forever on that error path; `max_sessions` such
/// leaks with no real live session left disabled the route.
pub(crate) struct SessionSlot<'a> {
    counter: &'a AtomicUsize,
    armed: bool,
}

impl<'a> SessionSlot<'a> {
    /// Reserves one slot against `counter`, refusing (and leaving `counter`
    /// unchanged) once `max` are already held.
    pub(crate) fn acquire(counter: &'a AtomicUsize, max: usize) -> Option<Self> {
        let prev = counter.fetch_add(1, Ordering::SeqCst);
        if prev >= max {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(SessionSlot {
            counter,
            armed: true,
        })
    }

    /// Hands this slot's lifetime off to whatever the caller is about to
    /// admit the session into — from this point the *new* owner is
    /// responsible for decrementing `counter` (e.g. on session reap), and
    /// this guard's own `Drop` becomes a no-op so the slot is never
    /// double-released.
    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for SessionSlot<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.counter.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn session_slot_releases_on_drop() {
        let counter = AtomicUsize::new(0);
        {
            let _slot = SessionSlot::acquire(&counter, 1).expect("first acquire succeeds");
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "a slot dropped without being disarmed must release itself"
        );
    }

    #[test]
    fn session_slot_refuses_over_capacity_without_changing_the_counter() {
        let counter = AtomicUsize::new(1);
        assert!(SessionSlot::acquire(&counter, 1).is_none());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "a refused acquire must leave the counter exactly as it found it"
        );
    }

    #[test]
    fn session_slot_disarm_prevents_double_release() {
        let counter = AtomicUsize::new(0);
        let slot = SessionSlot::acquire(&counter, 1).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        slot.disarm();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "disarm must not itself release the slot — the new owner does, later"
        );
        // Simulate the new owner's own teardown decrementing the same
        // counter exactly once.
        counter.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    use std::time::Duration;

    fn std_now() -> std::time::Instant {
        tokio::time::Instant::now().into_std()
    }

    /// K1: the sleep is absolute. A deadline 500 ms out with the previous fire
    /// 10 s ago fires at +500 ms, not +10.5 s.
    #[tokio::test(start_paused = true)]
    async fn a_real_deadline_fires_on_time_regardless_of_the_last_fire() {
        let start = tokio::time::Instant::now();
        let last_fire = std_now() - Duration::from_secs(10);
        sleep_until_timer(Some(std_now() + Duration::from_millis(500)), last_fire).await;
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }

    /// K1: the sleep is rebuilt every loop iteration (once per inbound
    /// datagram); a datagram every 10 ms must not delay a timer due at +500 ms.
    #[tokio::test(start_paused = true)]
    async fn steady_inbound_datagrams_do_not_starve_the_transport_timer() {
        let start = tokio::time::Instant::now();
        let last_fire = std_now() - Duration::from_secs(10);
        let deadline = std_now() + Duration::from_millis(500);
        let mut datagrams = tokio::time::interval(Duration::from_millis(10));
        datagrams.tick().await;
        loop {
            tokio::select! {
                _ = datagrams.tick() => {}
                () = sleep_until_timer(Some(deadline), last_fire) => break,
            }
            assert!(start.elapsed() < Duration::from_secs(2), "timer starved");
        }
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }

    /// An overdue deadline is floored to `last_fire + MIN_TIMER_INTERVAL`.
    #[tokio::test(start_paused = true)]
    async fn an_overdue_deadline_is_floored_from_the_last_fire() {
        let start = tokio::time::Instant::now();
        let now = std_now();
        sleep_until_timer(Some(now - Duration::from_secs(1)), now).await;
        assert_eq!(start.elapsed(), MIN_TIMER_INTERVAL);
    }
}
