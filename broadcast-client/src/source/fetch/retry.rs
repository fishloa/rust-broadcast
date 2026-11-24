//! Retry pacing: the backoff schedule, the server-requested `Retry-After`, and the gate that holds
//! the request queue until a backoff has elapsed.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use crate::Event;
use crate::events::MAX_SERVER_REFUSED_REASON_BYTES;

use broadcast_common::backoff::{BackoffSteps, InvalidBackoff, RetrySchedule};
use broadcast_common::http_date::parse_http_date;
use broadcast_common::stage::Timestamp;

use super::wall::WallClock;
use broadcast_common::units::MICROS_PER_SEC;
use http::{HeaderMap, StatusCode};

const MAX_RETRY_AFTER_SECS: u64 = 60;
const HEADER_RETRY_AFTER: &str = "retry-after";
const HEADER_DATE: &str = "date";
const HEADER_CONTENT_TYPE: &str = "content-type";
const HEADER_CONTENT_LENGTH: &str = "content-length";

/// The retry schedule every broadcast-client adapter waits out: the steps of a [`BackoffSteps`]
/// table (Durations end to end), then its cap forever, configured as one [`RetrySchedule`]
/// (issue #2471).
///
/// The first delay is exactly its step; every later one is the step times a jitter term of
/// `[5/6, 1.0)`, so a fleet of clients retrying one origin does not lockstep and no delay ever
/// exceeds its step. The table is taken as configured: a schedule [`RetrySchedule`] refuses is
/// reported as [`InvalidBackoff`], never repaired into one the caller did not ask for.
#[derive(Debug, Clone)]
pub(crate) struct Backoff {
    schedule: RetrySchedule,
}

impl Backoff {
    /// Build a schedule from `steps` (per-attempt delays), a `cap` used for every later attempt
    /// and the caller's own `seed` for the jitter.
    ///
    /// # Errors
    ///
    /// Whatever [`RetrySchedule::from_steps`] rejects (a cap above [`MAX_FLOOR`]).
    pub(crate) fn new(steps: &BackoffSteps, seed: u64) -> Result<Self, InvalidBackoff> {
        Ok(Self {
            schedule: RetrySchedule::from_steps(&steps.steps, steps.cap, seed)?,
        })
    }

    /// The unjittered step for `attempt` (1-based): `steps[attempt - 1]`, or `cap` once the
    /// steps are exhausted. Attempts beyond the table keep the cap.
    #[cfg(test)]
    pub(crate) fn raw_delay(&self, attempt: u32) -> Duration {
        // Attempt 0 is a caller that has taken no attempt yet: it takes the first step.
        let index = attempt.saturating_sub(1);
        self.schedule.raw_delay_for_attempt(index)
    }

    /// The jittered delay for `attempt` (1-based): in `[raw_delay * 5/6, raw_delay]`, and
    /// exactly `raw_delay` for the first.
    #[cfg(test)]
    pub(crate) fn jittered_delay(&self, attempt: u32) -> Duration {
        let index = attempt.saturating_sub(1);
        self.schedule.delay_for_attempt(index)
    }

    /// The jittered delay to wait before attempt number `attempt` (1-based).
    pub(crate) fn delay(&self, attempt: u32) -> Duration {
        let index = attempt.saturating_sub(1);
        self.schedule.delay_for_attempt(index)
    }
}

/// Parse a `Retry-After` header for a `429`/`503` response (RFC 9110 §10.2.3), returning the
/// wait. Returns `None` for any other status or a missing or unparseable header; a past
/// HTTP-date waits nothing (`Duration::ZERO`), and the result is clamped to `0..=60 s`. An
/// HTTP-date is measured against the host wall clock when `wall` is given, otherwise against the
/// response `Date`.
pub(crate) fn retry_after(
    status: StatusCode,
    headers: &HeaderMap,
    now: Timestamp,
    wall: Option<&WallClock>,
) -> Option<Duration> {
    if status != StatusCode::TOO_MANY_REQUESTS && status != StatusCode::SERVICE_UNAVAILABLE {
        return None;
    }
    let value = crate::headers::text(headers, HEADER_RETRY_AFTER).map(str::trim)?;
    let wait_us = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        let secs = value.parse::<u64>().unwrap_or(u64::MAX);
        i128::from(secs).saturating_mul(i128::from(MICROS_PER_SEC))
    } else {
        let until = parse_http_date(value).ok()?;
        let now_unix = match wall {
            Some(wall) => wall.utc_at(now),
            None => crate::headers::text(headers, HEADER_DATE)
                .and_then(|date| parse_http_date(date).ok())?,
        };
        i128::from(until.as_microsecond()) - i128::from(now_unix.as_microsecond())
    };
    let capped = wait_us.min(i128::from(MAX_RETRY_AFTER_SECS * MICROS_PER_SEC));
    Some(Duration::from_micros(u64::try_from(capped).unwrap_or(0)))
}

/// How long a connection must stay up before the retry series restarts from its floor: a peer
/// that accepts a connection and drops it seconds later, repeatedly, is a failing peer, so the
/// backoff (and the attempt counter) only restarts after a connection at least this long.
pub(crate) const HEALTHY_CONNECTION: Duration = Duration::from_millis(30_000);

/// The reconnect timer (issue #2471: the workspace carried this rule once in multimux's origin
/// supervisor and again in its push engine): a [`RetrySchedule`] plus the attempt counter, the
/// instant the current connection came up, and the rule that only a connection that stayed up for
/// [`HEALTHY_CONNECTION`] restarts the series.
///
/// The caller drives it with its own clock: [`on_connected`](Self::on_connected) and
/// [`on_disconnected`](Self::on_disconnected) take the instant, and the latter returns the wait to
/// arm. Nothing here reads a clock or sleeps.
#[derive(Debug, Clone)]
pub(crate) struct ReconnectTimer {
    schedule: RetrySchedule,
    attempts: u32,
    connected_at: Option<Timestamp>,
}

impl ReconnectTimer {
    /// A timer over a step table (then the cap), jittered from `seed`.
    ///
    /// # Errors
    ///
    /// [`InvalidBackoff`] exactly as [`RetrySchedule::from_steps`] rejects the table (a cap
    /// above [`MAX_FLOOR`]); never a schedule the caller did not configure.
    pub fn new(steps: &BackoffSteps, seed: u64) -> Result<Self, InvalidBackoff> {
        Ok(Self {
            schedule: RetrySchedule::from_steps(&steps.steps, steps.cap, seed)?,
            attempts: 0,
            connected_at: None,
        })
    }

    /// A timer whose every attempt waits nothing, for a field a caller cannot
    /// build fallibly: the TS-over-HTTP policy's; it never waits longer than
    /// configured, and its source has no way to refuse one.
    pub fn no_delay() -> Self {
        Self {
            schedule: RetrySchedule::no_delay(0),
            attempts: 0,
            connected_at: None,
        }
    }

    /// A connection came up at `now`. The attempt counter is not reset here: it restarts only
    /// if this connection stays up for [`HEALTHY_CONNECTION`] and then ends.
    pub fn on_connected(&mut self, now: Timestamp) {
        self.connected_at = Some(now);
    }

    /// Restart the series from its floor — call after a success, so the next
    /// outage waits the first step again. The jitter sequence starts over too,
    /// so the restarted series is the one a fresh timer would draw.
    pub fn reset(&mut self) {
        self.attempts = 0;
        self.connected_at = None;
        self.schedule = self.schedule.clone().with_offset(0);
    }

    /// How many attempts the current series has taken since it last restarted.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// A connection ended (or a dial-out failed) at `now`: start the next wait and return its
    /// length, so the caller can arm a timer without re-deriving it.
    ///
    /// The delay is the schedule's `[raw*5/6, raw]` band, the first retry being the raw delay
    /// itself.
    pub fn on_disconnected(&mut self, now: Timestamp) -> Duration {
        let healthy = self
            .connected_at
            .take()
            .is_some_and(|since| now.saturating_sub(since) >= HEALTHY_CONNECTION);
        if healthy {
            self.attempts = 0;
        }
        let attempt = self.attempts;
        self.attempts = self.attempts.saturating_add(1);
        self.schedule.delay_for_attempt(attempt)
    }
}

/// Holds the request queue until an instant: the latest hold wins, and the gate lifts itself the
/// first time it is asked at or after that instant. A session's wake-up *pacing* floor (an HLS
/// `Wait`, a retry's own wait) is a different rule from the retry series and stays separate.
///
/// A hold at or before the instant the gate was last asked (see
/// [`RetryGate::lift_at`]) is no hold: it would release on the spot, and the
/// request that set it would re-issue itself at the same instant until it
/// succeeded.
#[derive(Debug)]
pub(crate) struct RetryGate {
    until: Option<Timestamp>,
    asked_at: Timestamp,
}

impl Default for RetryGate {
    fn default() -> Self {
        Self {
            until: None,
            asked_at: Timestamp::ZERO,
        }
    }
}

impl RetryGate {
    pub(crate) fn hold_until(&mut self, at: Timestamp) {
        self.until = Some(self.until.map_or(at, |held| held.max(at)));
    }

    /// Note the instant the queue is being driven at, so a hold that does not
    /// move it forward is not taken.
    pub(crate) fn lift_at(&mut self, now: Timestamp) {
        self.asked_at = now;
    }

    /// Hold the queue until `at`, unless that is at or before the instant the
    /// gate was last asked ([`lift_at`](Self::lift_at)): a hold that does not
    /// move the clock forward releases at once, so the queue never spins on
    /// its own gate.
    pub(crate) fn hold_from(&mut self, at: Timestamp) {
        if at > self.asked_at {
            self.hold_until(at);
        }
    }

    /// Whether the gate is open at `now`, clearing a hold that has elapsed.
    pub(crate) fn lifted(&mut self, now: Timestamp) -> bool {
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

/// Whether `content_type` names `text/plain` with a UTF-8 (or absent) charset.
fn is_text_plain_utf8(content_type: &str) -> bool {
    let mut parts = content_type.split(';');
    let media = parts.next().unwrap_or("").trim();
    if !media.eq_ignore_ascii_case("text/plain") {
        return false;
    }
    for param in parts {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        let Some((key, value)) = param.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("charset") {
            let charset = value.trim().trim_matches('"').trim();
            if !charset.eq_ignore_ascii_case("utf-8") {
                return false;
            }
        }
    }
    true
}

/// A refused (429/503) response being captured so its body can become the reason of the
/// [`Event::ServerRefused`] emitted when the connection closes.
#[derive(Debug)]
pub(crate) struct RefusedCapture {
    status: StatusCode,
    retry_after: Option<Duration>,
    content_type: Option<String>,
    declared_len: Option<u64>,
    received: u64,
    body: Vec<u8>,
}

impl RefusedCapture {
    pub(crate) fn new(
        status: StatusCode,
        retry_after: Option<Duration>,
        headers: &HeaderMap,
    ) -> Self {
        let content_type = crate::headers::text(headers, HEADER_CONTENT_TYPE).map(String::from);
        Self {
            status,
            retry_after,
            content_type,
            declared_len: crate::headers::text(headers, HEADER_CONTENT_LENGTH)
                .and_then(|v| v.trim().parse().ok()),
            received: 0,
            body: Vec::new(),
        }
    }

    /// Whether any body byte can still become the reason: a `text/plain` UTF-8 body that is not
    /// declared empty.
    pub(crate) fn wants_body(&self) -> bool {
        self.declared_len != Some(0) && self.content_type.as_deref().is_some_and(is_text_plain_utf8)
    }

    /// Append body bytes; `true` once the whole body or the reason cap has been captured and the
    /// connection can be dropped.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> bool {
        let room = MAX_SERVER_REFUSED_REASON_BYTES.saturating_sub(self.body.len());
        let take = bytes.len().min(room);
        self.body.extend_from_slice(&bytes[..take]);
        self.received = self.received.saturating_add(bytes.len() as u64);
        self.body.len() >= MAX_SERVER_REFUSED_REASON_BYTES
            || self.declared_len.is_some_and(|d| self.received >= d)
    }

    pub(crate) fn into_event(self) -> Event {
        Event::ServerRefused {
            status: self.status,
            retry_after: self.retry_after,
            reason: sanitize_refused_reason(self.content_type.as_deref(), &self.body),
        }
    }
}

fn sanitize_refused_reason(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    let content_type = content_type?;
    if !is_text_plain_utf8(content_type) {
        return None;
    }
    let body = match core::str::from_utf8(body) {
        Err(e) if e.error_len().is_none() => &body[..e.valid_up_to()],
        _ => body,
    };
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len().min(MAX_SERVER_REFUSED_REASON_BYTES));
    for ch in text.chars() {
        if !ch.is_control() {
            out.push(ch);
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return None;
    }
    let capped = if trimmed.len() <= MAX_SERVER_REFUSED_REASON_BYTES {
        trimmed.to_string()
    } else {
        let mut end = MAX_SERVER_REFUSED_REASON_BYTES;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed[..end].to_string()
    };
    Some(capped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::backoff::MAX_FLOOR;

    fn header(name: &str, value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        let name = http::HeaderName::from_bytes(name.as_bytes()).unwrap();
        map.append(name, http::HeaderValue::from_str(value).unwrap());
        map
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn steps(entries: &[Duration], cap: Duration) -> BackoffSteps {
        BackoffSteps {
            steps: entries.to_vec(),
            cap,
        }
    }

    #[test]
    fn each_attempt_takes_its_own_step_then_the_cap() {
        let backoff = Backoff::new(
            &steps(&[ms(2_000), ms(4_000), ms(8_000)], ms(10_000)),
            0x5EED,
        )
        .unwrap();
        assert_eq!(backoff.raw_delay(1), ms(2_000));
        assert_eq!(backoff.raw_delay(2), ms(4_000));
        assert_eq!(backoff.raw_delay(3), ms(8_000));
        assert_eq!(backoff.raw_delay(4), ms(10_000));
        assert_eq!(backoff.raw_delay(u32::MAX), ms(10_000));
    }

    #[test]
    fn attempt_zero_takes_the_first_step() {
        let backoff = Backoff::new(&steps(&[ms(2_000), ms(4_000)], ms(10_000)), 0x5EED).unwrap();
        assert_eq!(backoff.raw_delay(0), ms(2_000));
    }

    #[test]
    fn a_sub_millisecond_step_keeps_its_microseconds() {
        let backoff = Backoff::new(
            &steps(
                &[Duration::from_micros(1_500)],
                Duration::from_micros(1_500),
            ),
            0x5EED,
        )
        .unwrap();
        assert_eq!(backoff.raw_delay(1), Duration::from_micros(1_500));
        assert_eq!(backoff.raw_delay(2), Duration::from_micros(1_500));
    }

    #[test]
    fn a_zero_step_or_cap_is_taken_as_configured() {
        let backoff = Backoff::new(
            &steps(&[Duration::ZERO, ms(5), Duration::ZERO], Duration::ZERO),
            0x5EED,
        )
        .unwrap();
        assert_eq!(backoff.raw_delay(1), Duration::ZERO);
        assert_eq!(backoff.raw_delay(2), ms(5));
        assert_eq!(backoff.raw_delay(3), Duration::ZERO);
        assert_eq!(backoff.raw_delay(4), Duration::ZERO);
    }

    #[test]
    fn an_empty_schedule_is_the_cap_from_the_first_attempt() {
        let backoff = Backoff::new(&steps(&[], ms(7)), 0x5EED).unwrap();
        assert_eq!(backoff.raw_delay(1), ms(7));
        assert_eq!(backoff.raw_delay(0), ms(7));
    }

    #[test]
    fn an_unrunnable_schedule_is_reported_not_repaired() {
        let above_ceiling = steps(&[ms(1)], Duration::MAX);
        let error =
            Backoff::new(&above_ceiling, 0x5EED).expect_err("a cap above MAX_FLOOR is refused");
        assert!(matches!(
            error,
            InvalidBackoff::FloorAboveCeiling { ceiling, .. } if ceiling == MAX_FLOOR
        ));
        assert!(ReconnectTimer::new(&above_ceiling, 0x5EED).is_err());
    }

    /// The schedule is jittered (issue #2471): every attempt draws in
    /// `[step * 5/6, step)`, so a fleet of clients retrying one origin does not
    /// lockstep, and no delay ever exceeds the configured step.
    #[test]
    fn every_delay_is_jittered_below_its_step_but_not_halved() {
        let backoff = Backoff::new(
            &steps(&[ms(2_000), ms(4_000), ms(8_000)], ms(10_000)),
            0x5EED,
        )
        .unwrap();
        let mut seen = alloc::collections::BTreeSet::new();
        for attempt in 0..32 {
            let raw = backoff.raw_delay(attempt);
            for _ in 0..8 {
                let delay = backoff.jittered_delay(attempt);
                let low = raw * 5 / 6;
                assert!(
                    delay >= low && delay <= raw,
                    "attempt {attempt}: {delay:?} outside [{low:?}, {raw:?}]"
                );
                seen.insert(delay);
            }
        }
        assert!(seen.len() > 1, "one constant delay is not a jittered one");
    }

    const NOW: u64 = 7_000_000;

    fn at(us: u64) -> Timestamp {
        Timestamp::from_micros(us)
    }
    const SIXTY_S: Duration = Duration::from_secs(60);
    const RESPONSE_DATE: &str = "Mon, 26 Oct 2026 08:39:04 GMT";
    const RESPONSE_UNIX_US: u64 = 1_793_003_944 * 1_000_000;

    fn us(span: Duration) -> u64 {
        u64::try_from(span.as_micros()).unwrap_or(u64::MAX)
    }

    fn wall(mono: u64, utc_us: u64) -> WallClock {
        let utc = jiff::Timestamp::from_microsecond(i64::try_from(utc_us).unwrap_or(i64::MAX))
            .unwrap_or(jiff::Timestamp::MAX);
        WallClock::new(Timestamp::from_micros(mono), utc)
    }

    fn retry(status: StatusCode, headers: &HeaderMap) -> u64 {
        retry_after(status, headers, at(NOW), None).map_or(0, us)
    }

    fn date_pair(retry_after: &str, date: &str) -> HeaderMap {
        let mut map = header("Retry-After", retry_after);
        map.append(
            http::header::DATE,
            http::HeaderValue::from_str(date).unwrap(),
        );
        map
    }

    fn anchored() -> WallClock {
        wall(NOW, RESPONSE_UNIX_US)
    }

    #[test]
    fn retry_after_is_honoured_only_for_429_and_503() {
        let h = header("Retry-After", "5");
        assert_eq!(retry(StatusCode::from_u16(429).unwrap(), &h), 5_000_000);
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 5_000_000);
        for status in [200, 404, 500, 502, 504] {
            let status = StatusCode::from_u16(status).unwrap();
            assert_eq!(retry(status, &h), 0, "{status}");
        }
    }

    #[test]
    fn retry_after_is_clamped_to_sixty_seconds() {
        assert_eq!(
            retry(
                StatusCode::from_u16(503).unwrap(),
                &header("retry-after", "60")
            ),
            us(SIXTY_S)
        );
        assert_eq!(
            retry(
                StatusCode::from_u16(503).unwrap(),
                &header("retry-after", "61")
            ),
            us(SIXTY_S)
        );
        assert_eq!(
            retry(
                StatusCode::from_u16(503).unwrap(),
                &header("retry-after", "86400")
            ),
            us(SIXTY_S)
        );
        assert_eq!(
            retry(
                StatusCode::from_u16(503).unwrap(),
                &header("retry-after", &u64::MAX.to_string())
            ),
            us(SIXTY_S)
        );
        assert_eq!(
            retry(
                StatusCode::from_u16(503).unwrap(),
                &header("retry-after", "99999999999999999999999999")
            ),
            us(SIXTY_S),
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
            assert_eq!(
                retry(
                    StatusCode::from_u16(503).unwrap(),
                    &header("Retry-After", value)
                ),
                0,
                "{value:?}"
            );
        }
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &header("X-Other", "1")),
            0
        );
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &header("X-Other", "5")),
            0
        );
    }

    #[test]
    fn retry_after_tolerates_whitespace_and_header_case() {
        assert_eq!(
            retry(
                StatusCode::from_u16(429).unwrap(),
                &header("RETRY-AFTER", "  3 ")
            ),
            3_000_000
        );
    }

    #[test]
    fn an_http_date_without_an_anchor_counts_from_the_response_date() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", RESPONSE_DATE);
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 30_000_000);
        assert_eq!(retry(StatusCode::from_u16(429).unwrap(), &h), 30_000_000);
    }

    #[test]
    fn an_http_date_without_an_anchor_or_a_date_header_is_ignored() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 0);
    }

    #[test]
    fn an_http_date_with_a_garbage_date_header_is_ignored() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", "yesterday");
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 0);
    }

    #[test]
    fn an_http_date_in_the_past_waits_nothing() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:03 GMT", RESPONSE_DATE);
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 0);
        let h = date_pair(RESPONSE_DATE, RESPONSE_DATE);
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &h),
            0,
            "now is not in the future"
        );
        let h = date_pair("Thu, 01 Jan 1970 00:00:00 GMT", RESPONSE_DATE);
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 0);
    }

    #[test]
    fn an_http_date_in_the_future_is_clamped_to_sixty_seconds() {
        let at_cap = date_pair("Mon, 26 Oct 2026 08:40:04 GMT", RESPONSE_DATE);
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &at_cap),
            us(SIXTY_S)
        );
        let past_cap = date_pair("Mon, 26 Oct 2026 08:40:05 GMT", RESPONSE_DATE);
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &past_cap),
            us(SIXTY_S)
        );
        let far = date_pair("Thu, 30 Dec 9999 00:00:00 GMT", RESPONSE_DATE);
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &far), us(SIXTY_S));
        let one_second_short = date_pair("Mon, 26 Oct 2026 08:40:03 GMT", RESPONSE_DATE);
        assert_eq!(
            retry(StatusCode::from_u16(503).unwrap(), &one_second_short),
            us(SIXTY_S) - 1_000_000
        );
    }

    #[test]
    fn the_obsolete_http_date_forms_count_too() {
        for value in ["Monday, 26-Oct-26 08:39:34 GMT", "Mon Oct 26 08:39:34 2026"] {
            assert_eq!(
                retry(
                    StatusCode::from_u16(503).unwrap(),
                    &date_pair(value, RESPONSE_DATE)
                ),
                us(Duration::from_millis(30_000)),
                "{value}"
            );
        }
    }

    #[test]
    fn an_anchor_replaces_the_response_date() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW),
                Some(&anchored())
            ),
            Some(Duration::from_millis(30_000)),
            "no Date header needed"
        );
        let lying_date = date_pair(
            "Mon, 26 Oct 2026 08:39:34 GMT",
            "Sun, 01 Jan 2023 00:00:00 GMT",
        );
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &lying_date,
                at(NOW),
                Some(&anchored())
            ),
            Some(Duration::from_millis(30_000)),
            "the anchor wins over a skewed server Date"
        );
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &lying_date,
                at(NOW),
                None
            ),
            Some(SIXTY_S),
            "the same headers without an anchor count from the skewed Date"
        );
    }

    #[test]
    fn a_far_future_anchor_is_not_a_past_date() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", RESPONSE_DATE);
        let anchor = WallClock::new(Timestamp::from_micros(0), jiff::Timestamp::MAX);
        assert_eq!(
            anchor.utc_at(Timestamp::from_micros(0)),
            jiff::Timestamp::MAX,
            "the crate's own mapping"
        );
        assert_eq!(
            retry_after(StatusCode::from_u16(503).unwrap(), &h, at(0), Some(&anchor)),
            Some(Duration::ZERO),
            "a URL past 584 557 AD is a past date, never a 584 557-year wait"
        );
    }

    #[test]
    fn a_seconds_long_wait_keeps_its_milliseconds() {
        let h = header("Retry-After", "4");
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW),
                Some(&anchored())
            ),
            Some(Duration::from_secs(4)),
            "a server wait is a whole span, not whole seconds"
        );
    }

    #[test]
    fn the_anchor_moves_with_the_response_instant() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        let wall = anchored();
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW + 10_000_000),
                Some(&wall)
            ),
            Some(Duration::from_secs(20))
        );
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW + 30_000_000),
                Some(&wall)
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW + 31_000_000),
                Some(&wall)
            ),
            Some(Duration::ZERO)
        );
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW - 5_000_000),
                Some(&wall)
            ),
            Some(Duration::from_secs(35)),
            "a response that predates the anchor still maps by the same offset"
        );
    }

    #[test]
    fn extreme_anchors_neither_overflow_nor_wrap() {
        let h = header("Retry-After", "Mon, 26 Oct 2026 08:39:34 GMT");
        let max_unix = wall(0, u64::MAX);
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(u64::MAX),
                Some(&max_unix)
            ),
            Some(Duration::ZERO)
        );
        let zero_unix = wall(u64::MAX, 0);
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                Timestamp::ZERO,
                Some(&zero_unix)
            ),
            Some(SIXTY_S),
            "a response far before the anchor clamps"
        );
        let late = wall(0, 0);
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(u64::MAX),
                Some(&late)
            ),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_gate_holds_until_its_instant_and_lifts_there() {
        let mut gate = RetryGate::default();
        assert!(gate.lifted(at(0)), "an unheld gate is open");
        gate.hold_until(at(100));
        assert!(!gate.lifted(at(99)));
        assert!(gate.lifted(at(100)), "open at exactly the instant");
        assert!(gate.lifted(at(0)), "lifting cleared the hold");
    }

    #[test]
    fn the_latest_hold_wins_in_either_order() {
        let mut gate = RetryGate::default();
        gate.hold_until(at(50));
        gate.hold_until(at(100));
        gate.hold_until(at(70));
        assert!(!gate.lifted(at(99)));
        assert!(gate.lifted(at(100)));
    }

    #[test]
    fn clear_opens_the_gate() {
        let mut gate = RetryGate::default();
        gate.hold_until(at(u64::MAX));
        assert!(!gate.lifted(at(1)));
        gate.clear();
        assert!(gate.lifted(at(1)));
    }

    #[test]
    fn a_still_held_gate_stays_held_when_asked_again() {
        let mut gate = RetryGate::default();
        gate.hold_until(at(100));
        assert!(!gate.lifted(at(10)));
        assert!(!gate.lifted(at(20)));
        assert!(gate.lifted(at(100)));
    }

    #[test]
    fn a_hold_that_does_not_move_the_clock_forward_is_no_hold() {
        let mut gate = RetryGate::default();
        gate.lift_at(at(50));
        gate.hold_from(at(50));
        assert!(gate.lifted(at(50)), "a zero delay releases at once");
        gate.hold_from(at(40));
        assert!(gate.lifted(at(50)), "a past hold releases at once");
        gate.hold_from(at(51));
        assert!(!gate.lifted(at(50)));
        assert!(gate.lifted(at(51)));
    }

    #[test]
    fn the_instant_asked_survives_a_lift() {
        let mut gate = RetryGate::default();
        gate.lift_at(at(70));
        assert!(gate.lifted(at(70)));
        gate.hold_from(at(70));
        assert!(
            gate.lifted(at(70)),
            "the request that lifted the gate cannot re-arm a hold at its own instant"
        );
    }

    /// The instant `us` before `Instant::now()`, derived without reading any clock (this crate
    /// An instant near the end of the clock, `us` microseconds before `u64::MAX`: the tests
    /// need a connection that stayed up "for a while" without reading a real clock, and a
    /// `Timestamp` subtraction is all the timer does.
    fn ts_ago(us: u64) -> Timestamp {
        Timestamp::from_micros(u64::MAX - us)
    }

    #[test]
    fn a_reconnect_timer_takes_its_steps_in_order() {
        let mut timer = ReconnectTimer::new(
            &steps(
                &[Duration::from_secs(2), Duration::from_secs(4)],
                Duration::from_secs(10),
            ),
            0x5EED,
        )
        .expect("a two-step table under a larger cap");
        let first = timer.on_disconnected(Timestamp::ZERO);
        assert_eq!(first, Duration::from_secs(2), "the first step, exactly");
        let second = timer.on_disconnected(Timestamp::from_micros(1_000_000));
        assert!(
            second >= Duration::from_secs(2) && second <= Duration::from_secs(4),
            "{second:?}"
        );
        let third = timer.on_disconnected(Timestamp::from_micros(2_000_000));
        assert!(
            third >= Duration::from_secs(4) && third <= Duration::from_secs(10),
            "{third:?}"
        );
    }

    #[test]
    fn only_a_connection_that_stayed_up_for_the_healthy_span_restarts_the_series() {
        let mut timer = ReconnectTimer::new(
            &steps(&[Duration::from_secs(1)], Duration::from_millis(30_000)),
            0x5EED,
        )
        .expect("a one-step table under a larger cap");
        let mut last = Duration::ZERO;
        for _ in 0..8 {
            timer.on_connected(Timestamp::ZERO);
            last = timer.on_disconnected(ts_ago(1_000));
        }
        assert!(
            last >= Duration::from_millis(500),
            "a connection that drops at once keeps counting up: {last:?}"
        );
        timer.on_connected(Timestamp::ZERO);
        let restarted = timer.on_disconnected(Timestamp::ZERO.saturating_add(HEALTHY_CONNECTION));
        assert!(
            restarted >= Duration::from_millis(500) && restarted < Duration::from_secs(2),
            "a healthy connection restarts the series at its first step: {restarted:?}"
        );
        timer.on_connected(Timestamp::ZERO.saturating_add(HEALTHY_CONNECTION));
        let wait = timer.on_disconnected(ts_ago(HEALTHY_CONNECTION.as_micros() as u64));
        assert!(
            wait < Duration::from_secs(2),
            "a connection that stayed up past the healthy span restarts the series: {wait:?}"
        );
    }

    #[test]
    fn an_exhausted_attempt_series_saturates_instead_of_wrapping_to_a_fresh_series() {
        let mut timer = ReconnectTimer::new(
            &steps(&[Duration::from_secs(1)], Duration::from_millis(30_000)),
            0x5EED,
        )
        .expect("a one-step table under a larger cap");
        timer.attempts = u32::MAX;
        let _ = timer.on_disconnected(Timestamp::ZERO);
        assert_eq!(
            timer.attempts(),
            u32::MAX,
            "a counter that wrapped to 0 would report a reconnect after a played connection"
        );
    }

    #[test]
    fn an_http_date_is_ignored_for_statuses_that_ignore_retry_after() {
        let h = date_pair("Mon, 26 Oct 2026 08:39:34 GMT", RESPONSE_DATE);
        for status in [200, 302, 404, 500, 502, 504] {
            let status = StatusCode::from_u16(status).unwrap();
            assert_eq!(retry(status, &h), 0, "{status}");
        }
    }

    #[test]
    fn delta_seconds_do_not_need_an_anchor_and_ignore_it() {
        let h = header("Retry-After", "5");
        assert_eq!(
            retry_after(
                StatusCode::from_u16(503).unwrap(),
                &h,
                at(NOW),
                Some(&anchored())
            ),
            Some(Duration::from_secs(5))
        );
        assert_eq!(retry(StatusCode::from_u16(503).unwrap(), &h), 5_000_000);
    }
}
