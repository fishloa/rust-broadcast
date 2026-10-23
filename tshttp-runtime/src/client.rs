use alloc::collections::VecDeque;
use alloc::string::String;

use transmux::{DemuxEvent, StreamingTsDemux};

use crate::config::Config;
use crate::events::{Action, Event, StarveKind};

#[derive(Debug, Clone, Copy)]
enum Phase {
    Opening { open_at: u64 },
    Awaiting { open_at: u64 },
    Playing,
    Backoff { due: u64 },
}

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
}

impl TsHttpClient {
    pub fn new(url: impl Into<String>, config: Config, now_ms: u64) -> Self {
        let mut c = Self {
            url: url.into(),
            config,
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
        };
        c.start_open(now_ms);
        c
    }

    pub fn poll(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    pub fn on_response(&mut self, status: u16, now_ms: u64) {
        if !matches!(self.phase, Phase::Opening { .. }) {
            return;
        }
        let open_at = match self.phase {
            Phase::Opening { open_at } => open_at,
            _ => self.open_at,
        };
        if (200..300).contains(&status) {
            self.events.push_back(Event::Connected);
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

    pub fn feed(&mut self, chunk: &[u8], now_ms: u64) {
        if chunk.is_empty() {
            return;
        }
        match self.phase {
            Phase::Awaiting { .. } | Phase::Playing => {}
            _ => return,
        }
        self.last_byte_at = now_ms;
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
        } else if !self.starved_data_fired {
            let baseline = self.last_sample_at;
            if now_ms.saturating_sub(baseline) >= self.config.stall_notice_ms {
                self.events.push_back(Event::Starved(StarveKind::Data));
                self.starved_data_fired = true;
            }
        }
    }

    pub fn on_eof(&mut self, now_ms: u64) {
        match self.phase {
            Phase::Opening { .. } => self.fail_before_playing(now_ms),
            Phase::Awaiting { .. } => self.fail_before_playing(now_ms),
            Phase::Playing => self.played_ended(now_ms),
            Phase::Backoff { .. } => {}
        }
    }

    pub fn on_error(&mut self, now_ms: u64) {
        match self.phase {
            Phase::Opening { .. } => self.fail_before_playing(now_ms),
            Phase::Awaiting { .. } => self.fail_before_playing(now_ms),
            Phase::Playing => self.played_ended(now_ms),
            Phase::Backoff { .. } => {}
        }
    }

    pub fn reconnect(&mut self, now_ms: u64) {
        match self.phase {
            Phase::Opening { .. } | Phase::Awaiting { .. } | Phase::Playing => {
                self.attempt = 0;
                self.actions.push_back(Action::Abort);
                self.actions.push_back(Action::Reconnect { after_ms: 0 });
                self.phase = Phase::Backoff { due: now_ms };
            }
            Phase::Backoff { .. } => {}
        }
    }

    pub fn poll_timeout(&mut self, now_ms: u64) {
        // Backoff due -> Open
        if let Phase::Backoff { due } = self.phase
            && now_ms >= due
        {
            self.start_open(now_ms);
            return;
        }
        // Awaiting timeout
        if let Phase::Awaiting { open_at } = self.phase
            && now_ms.saturating_sub(open_at) >= self.config.start_timeout_ms
        {
            self.actions.push_back(Action::Abort);
            self.fail_before_playing(now_ms);
            return;
        }
        // Playing stall timers
        if matches!(self.phase, Phase::Playing) {
            let idle = now_ms.saturating_sub(self.last_byte_at);
            if idle >= self.config.stall_notice_ms {
                if !self.starved_nodata_fired {
                    self.events.push_back(Event::Starved(StarveKind::NoData));
                    self.starved_nodata_fired = true;
                }
                // StallNotice
                if self.first_notice_at.is_none() {
                    self.first_notice_at = Some(now_ms);
                    self.next_notice_at = Some(now_ms + 1000);
                    self.events
                        .push_back(Event::StallNotice { secs: idle / 1000 });
                } else {
                    let mut next = self.next_notice_at;
                    while let Some(n) = next {
                        if now_ms >= n {
                            let elapsed = n.saturating_sub(self.last_byte_at);
                            self.events.push_back(Event::StallNotice {
                                secs: elapsed / 1000,
                            });
                            let nn = n + 1000;
                            self.next_notice_at = Some(nn);
                            next = Some(nn);
                        } else {
                            break;
                        }
                    }
                }
                // stall_reconnect
                if idle >= self.config.stall_reconnect_ms {
                    self.actions.push_back(Action::Abort);
                    self.played_ended(now_ms);
                }
            }
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
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
                let next_notice = self.next_notice_at.unwrap_or(notice);
                Some(next_notice.min(reconnect).min(notice))
            }
            Phase::Opening { .. } => None,
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
        self.actions.push_back(Action::Reconnect { after_ms: 0 });
        self.phase = Phase::Backoff { due: now_ms };
    }
}
