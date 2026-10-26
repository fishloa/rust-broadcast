//! Retry pacing: the backoff schedule, the server-requested `Retry-After`, and the gate that holds
//! the request queue until a backoff has elapsed.

use alloc::vec::Vec;

use dash_runtime::timing::datetime::parse_http_date;

use super::wall::WallClock;

const MIN_BACKOFF_MS: u64 = 1;
const MICROS_PER_MILLI: u64 = 1_000;
const MICROS_PER_SECOND: u64 = MICROS_PER_MILLI * 1_000;
const MAX_RETRY_AFTER_SECS: u64 = 60;
const STATUS_TOO_MANY_REQUESTS: u16 = 429;
const STATUS_SERVICE_UNAVAILABLE: u16 = 503;
const HEADER_RETRY_AFTER: &str = "retry-after";
const HEADER_DATE: &str = "date";

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

/// The wait a `429`/`503` response asks for with `Retry-After` (RFC 9110 10.2.3), clamped to 60 s;
/// `0` for any other status, a missing header, a value that is neither delta-seconds nor an
/// HTTP-date, or an HTTP-date in the past.
///
/// An HTTP-date counts from "now" as the host's wall clock reads it (`wall` mapped at `now_us`),
/// else from the response's own `Date` header (the server's idea of now, so a skewed server clock
/// cancels out), else it is ignored.
pub(crate) fn retry_after_us(
    status: u16,
    headers: &[(String, String)],
    now_us: u64,
    wall: Option<&WallClock>,
) -> u64 {
    if status != STATUS_TOO_MANY_REQUESTS && status != STATUS_SERVICE_UNAVAILABLE {
        return 0;
    }
    let Some(value) = header_value(headers, HEADER_RETRY_AFTER).map(str::trim) else {
        return 0;
    };
    let wait_us = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        let secs = value.parse::<u64>().unwrap_or(u64::MAX);
        i128::from(secs).saturating_mul(i128::from(MICROS_PER_SECOND))
    } else {
        let Ok(until) = parse_http_date(value) else {
            return 0;
        };
        let now_unix = match wall {
            Some(wall) => Some(wall.unix_at(now_us)),
            None => header_value(headers, HEADER_DATE)
                .and_then(|date| parse_http_date(date).ok())
                .map(i128::from),
        };
        let Some(now_unix) = now_unix else {
            return 0;
        };
        i128::from(until) - now_unix
    };
    let capped = wait_us.min(i128::from(MAX_RETRY_AFTER_SECS * MICROS_PER_SECOND));
    u64::try_from(capped).unwrap_or(0)
}

fn header_value<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
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

    const NOW: u64 = 7_000_000;
    const SIXTY_S: u64 = 60_000_000;
    const RESPONSE_DATE: &str = "Mon, 26 Oct 2026 08:39:04 GMT";
    const RESPONSE_UNIX_US: u64 = 1_793_003_944 * 1_000_000;

    fn retry(status: u16, headers: &[(String, String)]) -> u64 {
        retry_after_us(status, headers, NOW, None)
    }

    fn date_pair(retry_after: &str, date: &str) -> Vec<(String, String)> {
        vec![
            ("Date".to_string(), date.to_string()),
            ("Retry-After".to_string(), retry_after.to_string()),
        ]
    }

    fn anchored() -> WallClock {
        WallClock::new(NOW, RESPONSE_UNIX_US)
    }

    #[test]
    fn retry_after_is_honoured_only_for_429_and_503() {
        let h = header("Retry-After", "5");
        assert_eq!(retry(429, &h), 5_000_000);
        assert_eq!(retry(503, &h), 5_000_000);
        for status in [200, 404, 500, 502, 504] {
            assert_eq!(retry(status, &h), 0, "{status}");
        }
    }

    #[test]
    fn retry_after_is_clamped_to_sixty_seconds() {
        assert_eq!(retry(503, &header("retry-after", "60")), SIXTY_S);
        assert_eq!(retry(503, &header("retry-after", "61")), SIXTY_S);
        assert_eq!(retry(503, &header("retry-after", "86400")), SIXTY_S);
        assert_eq!(
            retry(503, &header("retry-after", &u64::MAX.to_string())),
            SIXTY_S
        );
        assert_eq!(
            retry(503, &header("retry-after", "99999999999999999999999999")),
            SIXTY_S,
            "a delta beyond u64 is still delta-seconds, clamped"
        );
    }

    #[test]
    fn retry_after_that_is_neither_delta_seconds_nor_a_date_is_ignored() {
        for value in [
            "",
            "soon",
            "-3",
            "+5",
            "1.5",
            "5s",
            "0x10",
            "Wed, 99 Oct 2026",
        ] {
            assert_eq!(retry(503, &header("Retry-After", value)), 0, "{value:?}");
        }
        assert_eq!(retry(503, &[]), 0);
        assert_eq!(retry(503, &header("X-Other", "5")), 0);
    }

    #[test]
    fn retry_after_tolerates_whitespace_and_header_case() {
        assert_eq!(retry(429, &header("RETRY-AFTER", "  3 ")), 3_000_000);
    }

    #[test]
    fn an_http_date_without_an_anchor_counts_from_the_response_date() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &h), 30_000_000);
        assert_eq!(retry(429, &h), 30_000_000);
    }

    #[test]
    fn an_http_date_without_an_anchor_or_a_date_header_is_ignored() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        assert_eq!(retry(503, &h), 0);
    }

    #[test]
    fn an_http_date_with_a_garbage_date_header_is_ignored() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", "yesterday");
        assert_eq!(retry(503, &h), 0);
    }

    #[test]
    fn an_http_date_in_the_past_waits_nothing() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:03 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &h), 0);
        let h = date_pair(RESPONSE_DATE, RESPONSE_DATE);
        assert_eq!(retry(503, &h), 0, "now is not in the future");
        let h = date_pair("Thu, 01 Jan 1970 00:00:00 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &h), 0);
    }

    #[test]
    fn an_http_date_in_the_future_is_clamped_to_sixty_seconds() {
        let at_cap = date_pair("Mon, 26 Oct 2026 08:40:04 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &at_cap), SIXTY_S);
        let past_cap = date_pair("Mon, 26 Oct 2026 08:40:05 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &past_cap), SIXTY_S);
        let far = date_pair("Thu, 30 Dec 9999 00:00:00 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &far), SIXTY_S);
        let one_second_short = date_pair("Mon, 26 Oct 2026 08:40:03 GMT", RESPONSE_DATE);
        assert_eq!(retry(503, &one_second_short), SIXTY_S - 1_000_000);
    }

    #[test]
    fn the_obsolete_http_date_forms_count_too() {
        for value in ["Monday, 26-Oct-26 08:39:34 GMT", "Mon Oct 26 08:39:34 2026"] {
            assert_eq!(
                retry(503, &date_pair(value, RESPONSE_DATE)),
                30_000_000,
                "{value}"
            );
        }
    }

    #[test]
    fn an_anchor_replaces_the_response_date() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        assert_eq!(
            retry_after_us(503, &h, NOW, Some(&anchored())),
            30_000_000,
            "no Date header needed"
        );
        let lying_date = date_pair(
            "Mon, 26 Oct 2026 08:39:34 GMT",
            "Sun, 01 Jan 2023 00:00:00 GMT",
        );
        assert_eq!(
            retry_after_us(503, &lying_date, NOW, Some(&anchored())),
            30_000_000,
            "the anchor wins over a skewed server Date"
        );
        assert_eq!(
            retry_after_us(503, &lying_date, NOW, None),
            SIXTY_S,
            "the same headers without an anchor count from the skewed Date"
        );
    }

    #[test]
    fn the_anchor_moves_with_the_response_instant() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        let wall = anchored();
        assert_eq!(
            retry_after_us(503, &h, NOW + 10_000_000, Some(&wall)),
            20_000_000
        );
        assert_eq!(retry_after_us(503, &h, NOW + 30_000_000, Some(&wall)), 0);
        assert_eq!(retry_after_us(503, &h, NOW + 31_000_000, Some(&wall)), 0);
        assert_eq!(
            retry_after_us(503, &h, NOW - 5_000_000, Some(&wall)),
            35_000_000,
            "a response that predates the anchor still maps by the same offset"
        );
    }

    #[test]
    fn extreme_anchors_neither_overflow_nor_wrap() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        let max_unix = WallClock::new(0, u64::MAX);
        assert_eq!(retry_after_us(503, &h, u64::MAX, Some(&max_unix)), 0);
        let zero_unix = WallClock::new(u64::MAX, 0);
        assert_eq!(
            retry_after_us(503, &h, 0, Some(&zero_unix)),
            SIXTY_S,
            "a response far before the anchor clamps"
        );
        let late = WallClock::new(0, 0);
        assert_eq!(retry_after_us(503, &h, u64::MAX, Some(&late)), 0);
    }

    #[test]
    fn an_http_date_is_ignored_for_statuses_that_ignore_retry_after() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", RESPONSE_DATE);
        for status in [200, 302, 404, 500, 502, 504] {
            assert_eq!(retry(status, &h), 0, "{status}");
        }
    }

    #[test]
    fn delta_seconds_do_not_need_an_anchor_and_ignore_it() {
        let h = header("Retry-After", "5");
        assert_eq!(retry_after_us(503, &h, NOW, Some(&anchored())), 5_000_000);
        assert_eq!(retry(503, &h), 5_000_000);
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
