//! Retry pacing: the backoff schedule, the server-requested `Retry-After`, and the gate that holds
//! the request queue until a backoff has elapsed.

use alloc::vec::Vec;

const MIN_BACKOFF_MS: u64 = 1;
const MICROS_PER_MILLI: u64 = 1_000;
const MICROS_PER_SECOND: u64 = MICROS_PER_MILLI * 1_000;
const MAX_RETRY_AFTER_SECS: u64 = 60;
const STATUS_TOO_MANY_REQUESTS: u16 = 429;
const STATUS_SERVICE_UNAVAILABLE: u16 = 503;
const HEADER_RETRY_AFTER: &str = "retry-after";

/// The retry schedule: `steps_ms[attempt - 1]`, then `cap_ms` forever. Every entry and the cap are
/// raised to at least one millisecond, so a zero never produces an unbounded burst of retries at
/// one instant.
#[derive(Debug, Clone)]
pub(crate) struct Backoff {
    steps_ms: Vec<u64>,
    cap_ms: u64,
}

impl Backoff {
    pub(crate) fn new(steps_ms: &[u64], cap_ms: u64) -> Self {
        Self {
            steps_ms: steps_ms
                .iter()
                .map(|ms| (*ms).max(MIN_BACKOFF_MS))
                .collect(),
            cap_ms: cap_ms.max(MIN_BACKOFF_MS),
        }
    }

    pub(crate) fn delay_us(&self, attempt: u64) -> u64 {
        let idx = usize::try_from(attempt.saturating_sub(1)).unwrap_or(usize::MAX);
        self.steps_ms
            .get(idx)
            .copied()
            .unwrap_or(self.cap_ms)
            .saturating_mul(MICROS_PER_MILLI)
    }
}

/// The wait a `429`/`503` response asks for with a delta-seconds `Retry-After`, clamped to 60 s;
/// `0` for any other status, a missing header or one that is not delta-seconds.
pub(crate) fn retry_after_us(status: u16, headers: &[(String, String)]) -> u64 {
    if status != STATUS_TOO_MANY_REQUESTS && status != STATUS_SERVICE_UNAVAILABLE {
        return 0;
    }
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(HEADER_RETRY_AFTER))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
        .map_or(0, |secs| {
            secs.min(MAX_RETRY_AFTER_SECS)
                .saturating_mul(MICROS_PER_SECOND)
        })
}

/// Holds the request queue until an instant: the latest hold wins, and the gate lifts itself the
/// first time it is asked at or after that instant.
#[derive(Debug, Default)]
pub(crate) struct RetryGate {
    until: Option<u64>,
}

impl RetryGate {
    pub(crate) fn hold_until(&mut self, at: u64) {
        self.until = Some(self.until.map_or(at, |held| held.max(at)));
    }

    pub(crate) fn lifted(&mut self, now: u64) -> bool {
        match self.until {
            Some(until) if now < until => false,
            _ => {
                self.until = None;
                true
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.until = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> Vec<(String, String)> {
        vec![(name.to_string(), value.to_string())]
    }

    #[test]
    fn each_attempt_takes_its_own_step_then_the_cap() {
        let backoff = Backoff::new(&[2_000, 4_000, 8_000], 10_000);
        assert_eq!(backoff.delay_us(1), 2_000_000);
        assert_eq!(backoff.delay_us(2), 4_000_000);
        assert_eq!(backoff.delay_us(3), 8_000_000);
        assert_eq!(backoff.delay_us(4), 10_000_000);
        assert_eq!(backoff.delay_us(u64::MAX), 10_000_000);
    }

    #[test]
    fn attempt_zero_takes_the_first_step() {
        let backoff = Backoff::new(&[2_000, 4_000], 10_000);
        assert_eq!(backoff.delay_us(0), 2_000_000);
    }

    #[test]
    fn a_zero_step_or_cap_never_bursts() {
        let backoff = Backoff::new(&[0, 5, 0], 0);
        assert_eq!(backoff.delay_us(1), MICROS_PER_MILLI);
        assert_eq!(backoff.delay_us(2), 5 * MICROS_PER_MILLI);
        assert_eq!(backoff.delay_us(3), MICROS_PER_MILLI);
        assert_eq!(backoff.delay_us(4), MICROS_PER_MILLI);
    }

    #[test]
    fn an_empty_schedule_is_the_cap_from_the_first_attempt() {
        assert_eq!(Backoff::new(&[], 7).delay_us(1), 7 * MICROS_PER_MILLI);
    }

    #[test]
    fn a_huge_step_saturates_instead_of_wrapping() {
        assert_eq!(Backoff::new(&[u64::MAX], 1).delay_us(1), u64::MAX);
    }

    #[test]
    fn retry_after_is_honoured_only_for_429_and_503() {
        let h = header("Retry-After", "5");
        assert_eq!(retry_after_us(429, &h), 5_000_000);
        assert_eq!(retry_after_us(503, &h), 5_000_000);
        for status in [200, 404, 500, 502, 504] {
            assert_eq!(retry_after_us(status, &h), 0, "{status}");
        }
    }

    #[test]
    fn retry_after_is_clamped_to_sixty_seconds() {
        assert_eq!(
            retry_after_us(503, &header("retry-after", "60")),
            60_000_000
        );
        assert_eq!(
            retry_after_us(503, &header("retry-after", "61")),
            60_000_000
        );
        assert_eq!(
            retry_after_us(503, &header("retry-after", "86400")),
            60_000_000
        );
        assert_eq!(
            retry_after_us(503, &header("retry-after", &u64::MAX.to_string())),
            60_000_000
        );
    }

    #[test]
    fn retry_after_that_is_not_delta_seconds_is_ignored() {
        for value in ["", "soon", "-3", "1.5", "Wed, 21 Oct 2026 07:28:00 GMT"] {
            assert_eq!(
                retry_after_us(503, &header("Retry-After", value)),
                0,
                "{value}"
            );
        }
        assert_eq!(retry_after_us(503, &[]), 0);
        assert_eq!(retry_after_us(503, &header("X-Other", "5")), 0);
    }

    #[test]
    fn retry_after_tolerates_whitespace_and_header_case() {
        assert_eq!(
            retry_after_us(429, &header("RETRY-AFTER", "  3 ")),
            3_000_000
        );
    }

    #[test]
    fn a_gate_holds_until_its_instant_and_lifts_there() {
        let mut gate = RetryGate::default();
        assert!(gate.lifted(0), "an unheld gate is open");
        gate.hold_until(100);
        assert!(!gate.lifted(99));
        assert!(gate.lifted(100), "open at exactly the instant");
        assert!(gate.lifted(0), "lifting cleared the hold");
    }

    #[test]
    fn the_latest_hold_wins_in_either_order() {
        let mut gate = RetryGate::default();
        gate.hold_until(50);
        gate.hold_until(100);
        gate.hold_until(70);
        assert!(!gate.lifted(99));
        assert!(gate.lifted(100));
    }

    #[test]
    fn clear_opens_the_gate() {
        let mut gate = RetryGate::default();
        gate.hold_until(u64::MAX);
        assert!(!gate.lifted(1));
        gate.clear();
        assert!(gate.lifted(1));
    }

    #[test]
    fn a_still_held_gate_stays_held_when_asked_again() {
        let mut gate = RetryGate::default();
        gate.hold_until(100);
        assert!(!gate.lifted(10));
        assert!(!gate.lifted(20));
        assert!(gate.lifted(100));
    }
}
