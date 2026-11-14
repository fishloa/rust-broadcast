use alloc::collections::VecDeque;
use alloc::string::String;
use core::time::Duration;

use crate::source::fetch::retry::{ReconnectTimer, backoff_ms};
use crate::tshttp::config::Config;
use crate::tshttp::events::{Action, Event, StarveKind};
use broadcast_common::stage::Timestamp;

/// The retry jitter's seed [`TsHttpPolicy::new`] uses: a policy takes no clock, so the seed is a
/// fixed one and two policies built from one configuration replay identically. It is chosen so
/// that the draws that matter most land exactly on their step — the first four, and the capped
/// band a long outage sits in — which is why TS-over-HTTP's scripted traces keep their exact
/// `Reconnect { after }`; [`TsHttpPolicy::with_seed`] takes any other.
const RETRY_SEED: u64 = 0x5EED_2471;

const STALL_NOTICE_REPEAT: Duration = Duration::from_secs(1);

/// How much longer each repeat of a stall notice waits than the one before: a stalled stream
/// reports once a second, then doubles.
const NOTICE_STEP_GROWTH: u32 = 2;
const IDENTITY_CODING: &str = "identity";
const HEADER_CONTENT_ENCODING: &str = "content-encoding";
const HEADER_CONTENT_LENGTH: &str = "content-length";
const HEADER_CONTENT_RANGE: &str = "content-range";
const STATUS_OK: u16 = 200;
const STATUS_PARTIAL_CONTENT: u16 = 206;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;
const STATUS_SUCCESS_CLASS: core::ops::Range<u16> = 200..300;

/// The span from `from` to `to` on the monotonic clock, never negative: a clock that went
/// backwards reads as no time passed.
fn since(to: Timestamp, from: Timestamp) -> Duration {
    to.saturating_sub(from)
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Opening { open_at: Timestamp },
    Awaiting { open_at: Timestamp },
    Playing,
    Backoff { due: Timestamp },
    Ended,
}

/// Demux-free TS-over-HTTP pull policy: the whole transition table of
/// `docs/tshttp-runtime/DESIGN.md`, driven by response headers, chunk lengths,
/// a "sample produced" flag and timestamps. It never sees TS bytes.
pub struct TsHttpPolicy {
    url: String,
    config: Config,
    phase: Phase,
    attempt: u64,
    retry: ReconnectTimer,
    opens: u64,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
    open_at: Timestamp,
    last_byte_at: Timestamp,
    last_sample_at: Timestamp,
    first_notice_at: Option<Timestamp>,
    next_notice_at: Option<Timestamp>,
    notice_step: Duration,
    starved_data_fired: bool,
    starved_nodata_fired: bool,
    finite: bool,
    expected_bytes: u64,
    received_bytes: u64,
    finite_done: bool,
    retry_after: Duration,
}

impl TsHttpPolicy {
    /// Create a new policy (T1).
    ///
    /// An inconsistent `config` is replaced by [`Config::clamped`] rather
    /// than rejected; call [`Config::validate`] first to detect it.
    pub fn new(url: impl Into<String>, config: Config, now: Timestamp) -> Self {
        Self::with_seed(url, config, now, RETRY_SEED)
    }

    /// Create a policy whose retry jitter is drawn from `seed`.
    ///
    /// Every decision here is a pure function of the caller's `now`, so two policies made
    /// from one configuration replay identically unless their seeds differ: a host that runs
    /// several sessions against one origin passes each a seed mixed from its own instant, and a
    /// test that asserts an exact delay passes a fixed one.
    pub fn with_seed(url: impl Into<String>, config: Config, now: Timestamp, seed: u64) -> Self {
        let config = config.clamped();
        let (steps_ms, cap_ms) = backoff_ms(&config.backoff);
        let retry = ReconnectTimer::new(&steps_ms, cap_ms, seed);
        let mut c = Self {
            url: url.into(),
            config,
            phase: Phase::Backoff {
                due: Timestamp::ZERO,
            },
            attempt: 0,
            retry,
            opens: 0,
            actions: VecDeque::new(),
            events: VecDeque::new(),
            open_at: now,
            last_byte_at: now,
            last_sample_at: now,
            first_notice_at: None,
            next_notice_at: None,
            notice_step: STALL_NOTICE_REPEAT,
            starved_data_fired: false,
            starved_nodata_fired: false,
            finite: false,
            expected_bytes: 0,
            received_bytes: 0,
            finite_done: false,
            retry_after: Duration::ZERO,
        };
        c.start_open(now);
        c
    }

    /// Poll next action.
    pub fn poll(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    /// Poll next event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Response headers received (T2-T4).
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now: Timestamp) {
        if !matches!(self.phase, Phase::Opening { .. }) {
            return;
        }
        let open_at = match self.phase {
            Phase::Opening { open_at } => open_at,
            _ => self.open_at,
        };
        if STATUS_SUCCESS_CLASS.contains(&status) {
            let finite = Self::detect_finite(status, headers);
            let expected = Self::expected_bytes(status, headers);
            self.finite = finite;
            self.expected_bytes = expected;
            self.received_bytes = 0;
            self.finite_done = false;
            self.retry_after = Duration::ZERO;
            if finite && expected == 0 {
                self.finite_done = true;
            }
            self.events.push_back(Event::Connected);
            if finite {
                self.events.push_back(Event::Finite {
                    expected_bytes: expected,
                });
            }
            self.phase = Phase::Awaiting { open_at };
            self.last_byte_at = now;
            self.last_sample_at = now;
            self.starved_data_fired = false;
            self.starved_nodata_fired = false;
            self.first_notice_at = None;
            self.next_notice_at = None;
        } else if status == STATUS_NOT_FOUND || status == STATUS_GONE {
            self.push_event(Event::NotFound);
            self.fail_before_playing(now);
        } else {
            self.fail_before_playing(now);
        }
    }

    /// Allowed to act on a queued reopen now? `false` when the backoff it waits out would end
    /// past the end of the clock: that deadline is never reached, so the policy never reopens and
    /// raises no wake for it (the backoff stays outstanding, and `Ended` is never entered).
    pub(crate) fn reconnect_ready(&self, now: Timestamp) -> bool {
        match self.phase {
            Phase::Backoff { due } => due > now,
            _ => true,
        }
    }

    /// Server-requested retry delay (a `Retry-After`), applied to the next failure.
    pub fn set_retry_after(&mut self, retry_after: Duration) {
        self.retry_after = retry_after;
    }

    fn header_values<'a>(headers: &'a [(String, String)], name: &str) -> alloc::vec::Vec<&'a str> {
        headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }

    fn parse_u64(s: &str) -> Option<u64> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        // decimal only
        for b in t.bytes() {
            if !b.is_ascii_digit() {
                return None;
            }
        }
        t.parse::<u64>().ok()
    }

    fn is_identity_coded(headers: &[(String, String)]) -> bool {
        Self::header_values(headers, HEADER_CONTENT_ENCODING)
            .iter()
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .all(|t| t.eq_ignore_ascii_case(IDENTITY_CODING))
    }

    fn finite_and_expected(status: u16, headers: &[(String, String)]) -> (bool, u64) {
        if !Self::is_identity_coded(headers) {
            return (false, 0);
        }
        if status == STATUS_OK {
            let vals = Self::header_values(headers, HEADER_CONTENT_LENGTH);
            if vals.is_empty() {
                return (false, 0);
            }
            let mut parsed: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
            for v in &vals {
                match Self::parse_u64(v) {
                    Some(n) => parsed.push(n),
                    None => return (false, 0),
                }
            }
            if parsed.windows(2).any(|w| w[0] != w[1]) {
                return (false, 0);
            }
            (true, parsed[0])
        } else if status == STATUS_PARTIAL_CONTENT {
            let vals = Self::header_values(headers, HEADER_CONTENT_RANGE);
            if vals.is_empty() {
                return (false, 0);
            }
            // conflicting duplicates
            let v0 = vals[0].trim();
            if vals.iter().any(|x| x.trim() != v0) {
                return (false, 0);
            }
            let Some(range) = broadcast_common::content_range::ContentRange::parse(v0) else {
                return (false, 0);
            };
            match range.len() {
                Some(len) => (true, len),
                None => (false, 0),
            }
        } else {
            (false, 0)
        }
    }

    fn detect_finite(status: u16, headers: &[(String, String)]) -> bool {
        Self::finite_and_expected(status, headers).0
    }

    fn expected_bytes(status: u16, headers: &[(String, String)]) -> u64 {
        Self::finite_and_expected(status, headers).1
    }

    /// `true` when body bytes are being accepted (Awaiting or Playing); a
    /// host driving a demuxer must not feed it otherwise.
    #[must_use]
    pub fn accepts_bytes(&self) -> bool {
        matches!(self.phase, Phase::Awaiting { .. } | Phase::Playing)
    }

    /// `true` once the session reached the terminal `Ended` phase.
    #[must_use]
    pub fn is_ended(&self) -> bool {
        matches!(self.phase, Phase::Ended)
    }

    /// Number of `Open` actions issued so far; changes whenever a new
    /// connection starts.
    #[must_use]
    pub fn open_count(&self) -> u64 {
        self.opens
    }

    /// Queue a host-produced event (for example a demuxer event) so it stays
    /// ordered against the policy's own events.
    pub fn push_event(&mut self, event: Event) {
        self.events.push_back(event);
    }

    /// A chunk of `len` body bytes arrived (T5, T14). `sampled` is true when
    /// the bytes produced at least one sample.
    pub fn on_bytes(&mut self, len: usize, now: Timestamp, sampled: bool) {
        if len == 0 || !self.accepts_bytes() {
            return;
        }
        self.last_byte_at = now;
        self.first_notice_at = None;
        self.next_notice_at = None;
        if self.finite {
            self.received_bytes = self.received_bytes.saturating_add(len as u64);
            if !self.finite_done
                && self.expected_bytes > 0
                && self.received_bytes >= self.expected_bytes
            {
                self.finite_done = true;
            }
            if !self.finite_done && self.expected_bytes == 0 {
                self.finite_done = true;
            }
        }
        if sampled {
            self.on_sample(now);
        } else if !(self.starved_data_fired || (self.finite && self.finite_done)) {
            let baseline = self.last_sample_at;
            if since(now, baseline) >= self.config.stall_notice {
                self.events.push_back(Event::Starved(StarveKind::Data));
                self.starved_data_fired = true;
            }
        }
    }

    /// A sample was produced (T5): Awaiting becomes Playing and the
    /// starvation episode resets.
    pub fn on_sample(&mut self, now: Timestamp) {
        if matches!(self.phase, Phase::Awaiting { .. }) {
            self.phase = Phase::Playing;
            self.attempt = 0;
        }
        self.last_sample_at = now;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.first_notice_at = None;
        self.next_notice_at = None;
    }

    /// Body ended (T7, T8).
    pub fn on_eof(&mut self, now: Timestamp) {
        self.on_eof_inner(now);
    }

    /// Connect/read failure (T4, T7, T8); handled exactly as a body end.
    pub fn on_error(&mut self, now: Timestamp) {
        self.on_eof_inner(now);
    }

    fn on_eof_inner(&mut self, now: Timestamp) {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        if self.finite && self.finite_done {
            self.phase = Phase::Ended;
            self.events.push_back(Event::Ended);
            return;
        }
        match self.phase {
            Phase::Opening { .. } => self.fail_before_playing(now),
            Phase::Awaiting { .. } => self.fail_before_playing(now),
            Phase::Playing => {
                if self.finite {
                    self.fail_before_playing(now);
                } else {
                    self.played_ended(now);
                }
            }
            Phase::Backoff { .. } => {}
            Phase::Ended => {}
        }
    }

    /// Caller-initiated reconnect (T13).
    pub fn reconnect(&mut self, now: Timestamp) {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        self.attempt = 0;
        self.finite = false;
        self.expected_bytes = 0;
        self.received_bytes = 0;
        self.finite_done = false;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.first_notice_at = None;
        self.next_notice_at = None;
        self.actions.push_back(Action::Abort);
        self.actions.push_back(Action::Reconnect {
            after: Duration::ZERO,
        });
        self.phase = Phase::Backoff { due: now };
    }

    /// Advance timers, calling `flush` right before a terminal start/stall
    /// decision (T6, T12). `flush` finishes the host's demuxer, appends its
    /// events to the queue it is given and returns whether it produced a
    /// sample.
    pub fn poll_timeout_with<F>(&mut self, now: Timestamp, mut flush: F)
    where
        F: FnMut(&mut VecDeque<Event>) -> bool,
    {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        // Backoff due -> Open
        if let Phase::Backoff { due } = self.phase
            && now >= due
        {
            self.start_open(now);
            return;
        }
        if let Phase::Opening { open_at } = self.phase {
            if since(now, open_at) >= self.config.start_timeout {
                self.actions.push_back(Action::Abort);
                self.fail_before_playing(now);
            }
            return;
        }
        if let Phase::Awaiting { open_at } = self.phase
            && !(self.finite && self.finite_done)
            && since(now, open_at) >= self.config.start_timeout
        {
            self.flush_via(&mut flush, now);
            if matches!(self.phase, Phase::Playing) {
                self.actions.push_back(Action::Abort);
                self.played_ended(now);
            } else {
                self.actions.push_back(Action::Abort);
                self.fail_before_playing(now);
            }
            return;
        }
        // Playing stall timers (disabled once finite transfer complete)
        if matches!(self.phase, Phase::Playing) && !(self.finite && self.finite_done) {
            let idle = since(now, self.last_byte_at);
            if idle >= self.config.stall_notice {
                if !self.starved_nodata_fired {
                    self.events.push_back(Event::Starved(StarveKind::NoData));
                    self.starved_nodata_fired = true;
                }
                if self.first_notice_at.is_none() {
                    self.first_notice_at = Some(now);
                    self.notice_step = STALL_NOTICE_REPEAT;
                    self.next_notice_at = now.checked_add(STALL_NOTICE_REPEAT);
                    self.events.push_back(Event::StallNotice {
                        secs: idle.as_secs(),
                    });
                } else if let Some(n) = self.next_notice_at
                    && now >= n
                {
                    self.events.push_back(Event::StallNotice {
                        secs: idle.as_secs(),
                    });
                    self.notice_step = self.notice_step.saturating_mul(NOTICE_STEP_GROWTH);
                    // A next notice past the end of the clock never fires, so none is armed.
                    self.next_notice_at = n.checked_add(self.notice_step);
                }
                // stall_reconnect
                if idle >= self.config.stall_reconnect {
                    self.flush_via(&mut flush, now);
                    self.actions.push_back(Action::Abort);
                    self.played_ended(now);
                }
            }
        }
    }

    /// Earliest deadline for [`Self::poll_timeout`].
    ///
    /// `None` is a timer that never fires: no phase is waiting, or the timer is at the `Ended`
    /// phase, or its instant does not fit in the clock.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        if matches!(self.phase, Phase::Ended) {
            return None;
        }
        if self.finite && self.finite_done {
            return None;
        }
        match self.phase {
            Phase::Backoff { due } => Some(due),
            Phase::Awaiting { open_at } => open_at.checked_add(self.config.start_timeout),
            Phase::Playing => {
                let notice = self
                    .last_byte_at
                    .checked_add(self.config.stall_notice)
                    .or(self.next_notice_at);
                let reconnect = self.last_byte_at.checked_add(self.config.stall_reconnect);
                let next_notice = if self.first_notice_at.is_some() {
                    self.next_notice_at.or(notice)
                } else {
                    notice
                };
                next_notice
                    .zip(reconnect)
                    .map_or(next_notice.or(reconnect), |(n, r)| Some(n.min(r)))
            }
            Phase::Opening { open_at } => open_at.checked_add(self.config.start_timeout),
            Phase::Ended => None,
        }
    }

    fn flush_via<F>(&mut self, flush: &mut F, now: Timestamp)
    where
        F: FnMut(&mut VecDeque<Event>) -> bool,
    {
        if flush(&mut self.events) {
            self.on_sample(now);
        }
    }

    fn start_open(&mut self, now: Timestamp) {
        self.opens = self.opens.saturating_add(1);
        self.phase = Phase::Opening { open_at: now };
        self.open_at = now;
        self.last_byte_at = now;
        self.last_sample_at = now;
        self.first_notice_at = None;
        self.next_notice_at = None;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.finite = false;
        self.expected_bytes = 0;
        self.received_bytes = 0;
        self.finite_done = false;
        self.retry_after = Duration::ZERO;
        self.actions.push_back(Action::Open {
            url: self.url.clone(),
        });
    }

    /// Enter the backoff that reopens `after` from `now`.
    ///
    /// A reopen past the end of the clock is no deadline: the policy takes no `Reconnect` action
    /// and stays in `Ended`, which arms no wake and so never reopens. A saturated action would
    /// read as due on every poll forever.
    fn back_off(&mut self, now: Timestamp, after: Duration) {
        let Some(due) = now.checked_add(after) else {
            self.phase = Phase::Ended;
            return;
        };
        self.actions.push_back(Action::Reconnect { after });
        self.phase = Phase::Backoff { due };
    }

    fn fail_before_playing(&mut self, now: Timestamp) {
        self.attempt = self.attempt.saturating_add(1);
        let wait = self.retry.on_disconnected_exact(now.as_micros());
        let after = wait.max(self.retry_after);
        self.retry_after = Duration::ZERO;
        self.events.push_back(Event::Reconnecting {
            attempt: self.attempt,
        });
        self.back_off(now, after);
    }

    fn played_ended(&mut self, now: Timestamp) {
        // A connection that played and then ended is a healthy one: the
        // failure counter restarts at its floor, and the retry series is
        // rewound rather than charged for this reconnect — its whole wait is
        // `min_reconnect_interval`.
        self.attempt = 0;
        self.retry.reset();
        let lasted = since(now, self.open_at);
        let after = self.config.min_reconnect_interval.saturating_sub(lasted);
        self.back_off(now, after);
    }
}
