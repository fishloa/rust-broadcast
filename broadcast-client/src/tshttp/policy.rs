use alloc::collections::VecDeque;
use alloc::string::String;
use core::time::Duration;

use crate::source::fetch::retry::{ReconnectTimer, backoff_ms};
use crate::tshttp::config::Config;
use crate::tshttp::events::{Action, Event, StarveKind};
use broadcast_common::stage::Timestamp;
use broadcast_common::units::MICROS_PER_MILLI;
use http::{HeaderMap, StatusCode};

/// The retry jitter's seed [`TsHttpPolicy::new`] uses: a policy takes no clock, so the seed is a
/// fixed one and two policies built from one configuration replay identically. It is chosen so
/// that the draws that matter most land exactly on their step — the first four, and the capped
/// band a long outage sits in — which is why TS-over-HTTP's scripted traces keep their exact
/// `Reconnect { after_ms }`; [`TsHttpPolicy::with_seed`] takes any other.
const RETRY_SEED: u64 = 0x5EED_2471;

const STALL_NOTICE_REPEAT_MS: u64 = 1_000;
const IDENTITY_CODING: &str = "identity";
const HEADER_CONTENT_ENCODING: &str = "content-encoding";
const HEADER_CONTENT_LENGTH: &str = "content-length";
const HEADER_CONTENT_RANGE: &str = "content-range";

/// A [`Duration`] as whole milliseconds, saturating at `u64::MAX`; the policy's own clock is
/// milliseconds, so every configured span is read at that resolution.
fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Opening { open_at: u64 },
    Awaiting { open_at: u64 },
    Playing,
    Backoff { due: u64 },
    Ended,
}

/// Demux-free TS-over-HTTP pull policy: the whole transition table of
/// `docs/tshttp-runtime/DESIGN.md`, driven by response headers, chunk lengths,
/// a "sample produced" flag and timestamps. It never sees TS bytes.
pub struct TsHttpPolicy {
    url: String,
    config: Config,
    phase: Phase,
    attempt: u32,
    retry: ReconnectTimer,
    opens: u64,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
    open_at: u64,
    last_byte_at: u64,
    last_sample_at: u64,
    first_notice_at: Option<u64>,
    next_notice_at: Option<u64>,
    starved_data_fired: bool,
    starved_nodata_fired: bool,
    finite: bool,
    expected_bytes: u64,
    received_bytes: u64,
    finite_done: bool,
    retry_after_ms: u64,
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
        let now_ms = now.as_millis();
        let config = config.clamped();
        let (steps_ms, cap_ms) = backoff_ms(&config.backoff);
        let retry = ReconnectTimer::new(&steps_ms, cap_ms, seed);
        let mut c = Self {
            url: url.into(),
            config,
            phase: Phase::Backoff { due: 0 },
            attempt: 0,
            retry,
            opens: 0,
            actions: VecDeque::new(),
            events: VecDeque::new(),
            open_at: now_ms,
            last_byte_at: now_ms,
            last_sample_at: now_ms,
            first_notice_at: None,
            next_notice_at: None,
            starved_data_fired: false,
            starved_nodata_fired: false,
            finite: false,
            expected_bytes: 0,
            received_bytes: 0,
            finite_done: false,
            retry_after_ms: 0,
        };
        c.start_open(now_ms);
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
    pub fn on_response(&mut self, status: StatusCode, headers: &HeaderMap, now: Timestamp) {
        let now_ms = now.as_millis();
        if !matches!(self.phase, Phase::Opening { .. }) {
            return;
        }
        let open_at = match self.phase {
            Phase::Opening { open_at } => open_at,
            _ => self.open_at,
        };
        if status.is_success() {
            let finite = Self::detect_finite(status, headers);
            let expected = Self::expected_bytes(status, headers);
            self.finite = finite;
            self.expected_bytes = expected;
            self.received_bytes = 0;
            self.finite_done = false;
            self.retry_after_ms = 0;
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
            self.last_byte_at = now_ms;
            self.last_sample_at = now_ms;
            self.starved_data_fired = false;
            self.starved_nodata_fired = false;
            self.first_notice_at = None;
            self.next_notice_at = None;
        } else if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            self.push_event(Event::NotFound);
            self.fail_before_playing(now_ms);
        } else {
            self.fail_before_playing(now_ms);
        }
    }

    /// Server-requested retry delay, milliseconds, applied to the next failure.
    pub fn set_retry_after(&mut self, retry_after_ms: u64) {
        self.retry_after_ms = retry_after_ms;
    }

    fn header_values<'a>(headers: &'a HeaderMap, name: &str) -> alloc::vec::Vec<&'a str> {
        headers
            .get_all(name)
            .iter()
            .filter_map(|v| v.to_str().ok())
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

    fn is_identity_coded(headers: &HeaderMap) -> bool {
        Self::header_values(headers, HEADER_CONTENT_ENCODING)
            .iter()
            .flat_map(|v| v.split(','))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .all(|t| t.eq_ignore_ascii_case(IDENTITY_CODING))
    }

    fn finite_and_expected(status: StatusCode, headers: &HeaderMap) -> (bool, u64) {
        if !Self::is_identity_coded(headers) {
            return (false, 0);
        }
        if status == StatusCode::OK {
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
        } else if status == StatusCode::PARTIAL_CONTENT {
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

    fn detect_finite(status: StatusCode, headers: &HeaderMap) -> bool {
        Self::finite_and_expected(status, headers).0
    }

    fn expected_bytes(status: StatusCode, headers: &HeaderMap) -> u64 {
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
        let now_ms = now.as_millis();
        if len == 0 || !self.accepts_bytes() {
            return;
        }
        self.last_byte_at = now_ms;
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
            self.on_sample_ms(now_ms);
        } else if !(self.starved_data_fired || (self.finite && self.finite_done)) {
            let baseline = self.last_sample_at;
            if now_ms.saturating_sub(baseline) >= millis(self.config.stall_notice) {
                self.events.push_back(Event::Starved(StarveKind::Data));
                self.starved_data_fired = true;
            }
        }
    }

    /// A sample was produced (T5): Awaiting becomes Playing and the
    /// starvation episode resets.
    pub fn on_sample(&mut self, now: Timestamp) {
        let now_ms = now.as_millis();
        self.on_sample_ms(now_ms);
    }

    fn on_sample_ms(&mut self, now_ms: u64) {
        if matches!(self.phase, Phase::Awaiting { .. }) {
            self.phase = Phase::Playing;
            self.attempt = 0;
        }
        self.last_sample_at = now_ms;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.first_notice_at = None;
        self.next_notice_at = None;
    }

    /// Body ended (T7, T8).
    pub fn on_eof(&mut self, now: Timestamp) {
        let now_ms = now.as_millis();
        self.on_eof_inner(now_ms);
    }

    /// Connect/read failure (T4, T7, T8); handled exactly as a body end.
    pub fn on_error(&mut self, now: Timestamp) {
        let now_ms = now.as_millis();
        self.on_eof_inner(now_ms);
    }

    fn on_eof_inner(&mut self, now_ms: u64) {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        if self.finite && self.finite_done {
            self.phase = Phase::Ended;
            self.events.push_back(Event::Ended);
            return;
        }
        match self.phase {
            Phase::Opening { .. } => self.fail_before_playing(now_ms),
            Phase::Awaiting { .. } => self.fail_before_playing(now_ms),
            Phase::Playing => {
                if self.finite {
                    self.fail_before_playing(now_ms);
                } else {
                    self.played_ended(now_ms);
                }
            }
            Phase::Backoff { .. } => {}
            Phase::Ended => {}
        }
    }

    /// Caller-initiated reconnect (T13).
    pub fn reconnect(&mut self, now: Timestamp) {
        let now_ms = now.as_millis();
        match self.phase {
            Phase::Opening { .. } | Phase::Awaiting { .. } | Phase::Playing | Phase::Ended => {
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
                self.actions.push_back(Action::Reconnect { after_ms: 0 });
                self.phase = Phase::Backoff { due: now_ms };
            }
            Phase::Backoff { .. } => {}
        }
    }

    /// Advance timers, calling `flush` right before a terminal start/stall
    /// decision (T6, T12). `flush` finishes the host's demuxer, appends its
    /// events to the queue it is given and returns whether it produced a
    /// sample.
    pub fn poll_timeout_with<F>(&mut self, now: Timestamp, mut flush: F)
    where
        F: FnMut(&mut VecDeque<Event>) -> bool,
    {
        let now_ms = now.as_millis();
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        // Backoff due -> Open
        if let Phase::Backoff { due } = self.phase
            && now_ms >= due
        {
            self.start_open(now_ms);
            return;
        }
        if let Phase::Opening { open_at } = self.phase {
            if now_ms.saturating_sub(open_at) >= millis(self.config.start_timeout) {
                self.actions.push_back(Action::Abort);
                self.fail_before_playing(now_ms);
            }
            return;
        }
        if let Phase::Awaiting { open_at } = self.phase
            && !(self.finite && self.finite_done)
            && now_ms.saturating_sub(open_at) >= millis(self.config.start_timeout)
        {
            self.flush_via(&mut flush, now_ms);
            if matches!(self.phase, Phase::Playing) {
                self.actions.push_back(Action::Abort);
                self.played_ended(now_ms);
            } else {
                self.actions.push_back(Action::Abort);
                self.fail_before_playing(now_ms);
            }
            return;
        }
        // Playing stall timers (disabled once finite transfer complete)
        if matches!(self.phase, Phase::Playing) && !(self.finite && self.finite_done) {
            let idle = now_ms.saturating_sub(self.last_byte_at);
            if idle >= millis(self.config.stall_notice) {
                if !self.starved_nodata_fired {
                    self.events.push_back(Event::Starved(StarveKind::NoData));
                    self.starved_nodata_fired = true;
                }
                if self.first_notice_at.is_none() {
                    self.first_notice_at = Some(now_ms);
                    self.next_notice_at = Some(now_ms.saturating_add(STALL_NOTICE_REPEAT_MS));
                    self.events
                        .push_back(Event::StallNotice { secs: idle / 1000 });
                } else if let Some(n) = self.next_notice_at
                    && now_ms >= n
                {
                    self.events
                        .push_back(Event::StallNotice { secs: idle / 1000 });
                    let steps = (now_ms - n) / STALL_NOTICE_REPEAT_MS + 1;
                    self.next_notice_at =
                        Some(n.saturating_add(steps.saturating_mul(STALL_NOTICE_REPEAT_MS)));
                }
                // stall_reconnect
                if idle >= millis(self.config.stall_reconnect) {
                    self.flush_via(&mut flush, now_ms);
                    self.actions.push_back(Action::Abort);
                    self.played_ended(now_ms);
                }
            }
        }
    }

    /// Earliest deadline for [`Self::poll_timeout`].
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.next_deadline_ms().map(Timestamp::from_millis)
    }

    /// [`Self::next_deadline`] on the policy's own millisecond clock.
    fn next_deadline_ms(&self) -> Option<u64> {
        if matches!(self.phase, Phase::Ended) {
            return None;
        }
        if self.finite && self.finite_done {
            return None;
        }
        match self.phase {
            Phase::Backoff { due } => Some(due),
            Phase::Awaiting { open_at } => {
                Some(open_at.saturating_add(millis(self.config.start_timeout)))
            }
            Phase::Playing => {
                let notice = self
                    .last_byte_at
                    .saturating_add(millis(self.config.stall_notice));
                let reconnect = self
                    .last_byte_at
                    .saturating_add(millis(self.config.stall_reconnect));
                let next_notice = match self.first_notice_at {
                    Some(_) => self.next_notice_at.unwrap_or(notice),
                    None => notice,
                };
                Some(next_notice.min(reconnect))
            }
            Phase::Opening { open_at } => {
                Some(open_at.saturating_add(millis(self.config.start_timeout)))
            }
            Phase::Ended => None,
        }
    }

    fn flush_via<F>(&mut self, flush: &mut F, now_ms: u64)
    where
        F: FnMut(&mut VecDeque<Event>) -> bool,
    {
        if flush(&mut self.events) {
            self.on_sample_ms(now_ms);
        }
    }

    fn start_open(&mut self, now_ms: u64) {
        self.opens = self.opens.saturating_add(1);
        self.phase = Phase::Opening { open_at: now_ms };
        self.open_at = now_ms;
        self.last_byte_at = now_ms;
        self.last_sample_at = now_ms;
        self.first_notice_at = None;
        self.next_notice_at = None;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.finite = false;
        self.expected_bytes = 0;
        self.received_bytes = 0;
        self.finite_done = false;
        self.retry_after_ms = 0;
        self.actions.push_back(Action::Open {
            url: self.url.clone(),
        });
    }

    fn fail_before_playing(&mut self, now_ms: u64) {
        self.attempt = self.attempt.saturating_add(1);
        let wait = self
            .retry
            .on_disconnected_exact(now_ms.saturating_mul(MICROS_PER_MILLI));
        let after = u64::try_from(wait.as_micros())
            .unwrap_or(u64::MAX)
            .div_ceil(MICROS_PER_MILLI);
        let after = after.max(self.retry_after_ms);
        self.retry_after_ms = 0;
        self.actions
            .push_back(Action::Reconnect { after_ms: after });
        self.events.push_back(Event::Reconnecting {
            attempt: self.attempt,
        });
        self.phase = Phase::Backoff {
            due: now_ms.saturating_add(after),
        };
    }

    fn played_ended(&mut self, now_ms: u64) {
        // A connection that played and then ended is a healthy one: the
        // failure counter restarts at its floor, and the retry series is
        // rewound rather than charged for this reconnect — its whole wait is
        // `min_reconnect_interval`.
        self.attempt = 0;
        self.retry.reset();
        let lasted = now_ms.saturating_sub(self.open_at);
        let after = millis(self.config.min_reconnect_interval).saturating_sub(lasted);
        self.actions
            .push_back(Action::Reconnect { after_ms: after });
        self.phase = Phase::Backoff {
            due: now_ms.saturating_add(after),
        };
    }
}
