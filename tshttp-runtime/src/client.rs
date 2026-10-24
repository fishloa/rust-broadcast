use alloc::collections::VecDeque;
use alloc::string::String;

use transmux::{DemuxEvent, StreamingTsDemux};

use crate::config::Config;
use crate::events::{Action, Event, StarveKind};

const STALL_NOTICE_REPEAT_MS: u64 = 1_000;
const IDENTITY_CODING: &str = "identity";

#[derive(Debug, Clone, Copy)]
enum Phase {
    Opening { open_at: u64 },
    Awaiting { open_at: u64 },
    Playing,
    Backoff { due: u64 },
    Ended,
}

/// TS-over-HTTP pull session client (DESIGN.md).
pub struct TsHttpClient {
    url: String,
    config: Config,
    phase: Phase,
    attempt: u64,
    demux: StreamingTsDemux,
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
}

impl TsHttpClient {
    /// Create a new client (T1).
    ///
    /// An inconsistent `config` is replaced by [`Config::clamped`] rather
    /// than rejected; call [`Config::validate`] first to detect it.
    pub fn new(url: impl Into<String>, config: Config, now_ms: u64) -> Self {
        let mut c = Self {
            url: url.into(),
            config: config.clamped(),
            phase: Phase::Backoff { due: 0 },
            attempt: 0,
            demux: StreamingTsDemux::new(),
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
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now_ms: u64) {
        if !matches!(self.phase, Phase::Opening { .. }) {
            return;
        }
        let open_at = match self.phase {
            Phase::Opening { open_at } => open_at,
            _ => self.open_at,
        };
        if (200..300).contains(&status) {
            let finite = Self::detect_finite(status, headers);
            let expected = Self::expected_bytes(status, headers);
            self.finite = finite;
            self.expected_bytes = expected;
            self.received_bytes = 0;
            self.finite_done = false;
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
        } else if status == 404 || status == 410 {
            self.events.push_back(Event::NotFound { status });
            self.fail_before_playing(now_ms);
        } else {
            self.fail_before_playing(now_ms);
        }
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
        Self::header_values(headers, "content-encoding")
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
        if status == 200 {
            let vals = Self::header_values(headers, "content-length");
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
        } else if status == 206 {
            let vals = Self::header_values(headers, "content-range");
            if vals.is_empty() {
                return (false, 0);
            }
            // conflicting duplicates
            let v0 = vals[0].trim();
            if vals.iter().any(|x| x.trim() != v0) {
                return (false, 0);
            }
            let Some(rest) = v0.strip_prefix("bytes") else {
                return (false, 0);
            };
            let rest = rest.trim_start();
            let Some(slash) = rest.find('/') else {
                return (false, 0);
            };
            let range_part = rest[..slash].trim();
            let total = rest[slash + 1..].trim();
            let Some(total) = Self::parse_u64(total) else {
                return (false, 0);
            };
            let Some(dash) = range_part.find('-') else {
                return (false, 0);
            };
            let Some(a) = Self::parse_u64(&range_part[..dash]) else {
                return (false, 0);
            };
            let Some(b) = Self::parse_u64(&range_part[dash + 1..]) else {
                return (false, 0);
            };
            if b < a || b >= total {
                return (false, 0);
            }
            (true, b - a + 1)
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

    /// Feed body bytes (T5, T14).
    pub fn feed(&mut self, chunk: &[u8], now_ms: u64) {
        if chunk.is_empty() {
            return;
        }
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        match self.phase {
            Phase::Awaiting { .. } | Phase::Playing => {}
            _ => return,
        }
        self.last_byte_at = now_ms;
        self.first_notice_at = None;
        self.next_notice_at = None;
        if self.finite {
            self.received_bytes = self.received_bytes.saturating_add(chunk.len() as u64);
            if !self.finite_done
                && self.expected_bytes > 0
                && self.received_bytes >= self.expected_bytes
            {
                self.finite_done = true;
            }
            if !self.finite_done && self.expected_bytes == 0 && self.finite {
                // zero-length finite: complete immediately
                self.finite_done = true;
            }
        }
        self.demux.feed(chunk);
        let mut first_sample = false;
        while let Some(ev) = self.demux.poll_event() {
            if matches!(ev, DemuxEvent::Sample { .. }) {
                first_sample = true;
            }
            self.events.push_back(Event::Demux(ev));
        }
        if first_sample {
            if matches!(self.phase, Phase::Awaiting { .. }) {
                self.phase = Phase::Playing;
                self.attempt = 0;
            }
            self.last_sample_at = now_ms;
            self.starved_data_fired = false;
            self.starved_nodata_fired = false;
            self.first_notice_at = None;
            self.next_notice_at = None;
        } else if !(self.starved_data_fired || (self.finite && self.finite_done)) {
            let baseline = self.last_sample_at;
            if now_ms.saturating_sub(baseline) >= self.config.stall_notice_ms {
                self.events.push_back(Event::Starved(StarveKind::Data));
                self.starved_data_fired = true;
            }
        }
    }

    /// Body ended (T7, T8).
    pub fn on_eof(&mut self, now_ms: u64) {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        self.flush_demux(now_ms);
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
                    // truncated transfer
                    self.fail_before_playing(now_ms);
                } else {
                    self.played_ended(now_ms);
                }
            }
            Phase::Backoff { .. } => {}
            Phase::Ended => {}
        }
    }

    /// Connect/read failure (T4, T7, T8).
    pub fn on_error(&mut self, now_ms: u64) {
        if matches!(self.phase, Phase::Ended) {
            return;
        }
        self.flush_demux(now_ms);
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
    pub fn reconnect(&mut self, now_ms: u64) {
        match self.phase {
            Phase::Opening { .. } | Phase::Awaiting { .. } | Phase::Playing | Phase::Ended => {
                self.flush_demux(now_ms);
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

    /// Advance timers (T6, T10-T12, T15).
    pub fn poll_timeout(&mut self, now_ms: u64) {
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
            if now_ms.saturating_sub(open_at) >= self.config.start_timeout_ms {
                self.actions.push_back(Action::Abort);
                self.fail_before_playing(now_ms);
            }
            return;
        }
        if let Phase::Awaiting { open_at } = self.phase
            && !(self.finite && self.finite_done)
            && now_ms.saturating_sub(open_at) >= self.config.start_timeout_ms
        {
            self.flush_demux(now_ms);
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
            if idle >= self.config.stall_notice_ms {
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
                if idle >= self.config.stall_reconnect_ms {
                    self.flush_demux(now_ms);
                    self.actions.push_back(Action::Abort);
                    self.played_ended(now_ms);
                }
            }
        }
    }

    /// Earliest deadline for poll_timeout.
    pub fn next_deadline(&self) -> Option<u64> {
        if matches!(self.phase, Phase::Ended) {
            return None;
        }
        if self.finite && self.finite_done {
            return None;
        }
        match self.phase {
            Phase::Backoff { due } => Some(due),
            Phase::Awaiting { open_at } => {
                Some(open_at.saturating_add(self.config.start_timeout_ms))
            }
            Phase::Playing => {
                let notice = self
                    .last_byte_at
                    .saturating_add(self.config.stall_notice_ms);
                let reconnect = self
                    .last_byte_at
                    .saturating_add(self.config.stall_reconnect_ms);
                let next_notice = match self.first_notice_at {
                    Some(_) => self.next_notice_at.unwrap_or(notice),
                    None => notice,
                };
                Some(next_notice.min(reconnect))
            }
            Phase::Opening { open_at } => {
                Some(open_at.saturating_add(self.config.start_timeout_ms))
            }
            Phase::Ended => None,
        }
    }

    fn flush_demux(&mut self, now_ms: u64) {
        self.demux.finish();
        let mut first_sample = false;
        while let Some(ev) = self.demux.poll_event() {
            if matches!(ev, DemuxEvent::Sample { .. }) {
                first_sample = true;
            }
            self.events.push_back(Event::Demux(ev));
        }
        if first_sample {
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
    }

    fn start_open(&mut self, now_ms: u64) {
        self.demux = StreamingTsDemux::new();
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
        self.actions.push_back(Action::Open {
            url: self.url.clone(),
        });
    }

    fn fail_before_playing(&mut self, now_ms: u64) {
        self.attempt = self.attempt.saturating_add(1);
        let after = if (self.attempt as usize) <= self.config.backoff_ms.len() {
            self.config.backoff_ms[self.attempt as usize - 1]
        } else {
            self.config.backoff_cap_ms
        };
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
        self.attempt = 0;
        let lasted = now_ms.saturating_sub(self.open_at);
        let after = self.config.min_reconnect_interval_ms.saturating_sub(lasted);
        self.actions
            .push_back(Action::Reconnect { after_ms: after });
        self.phase = Phase::Backoff {
            due: now_ms.saturating_add(after),
        };
    }
}
