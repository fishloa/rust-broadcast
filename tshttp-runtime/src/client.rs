use alloc::string::String;

use transmux::{DemuxEvent, StreamingTsDemux};

use crate::config::Config;
use crate::events::{Action, Event};
use crate::policy::TsHttpPolicy;

/// TS-over-HTTP pull session client (DESIGN.md): [`TsHttpPolicy`] plus a
/// [`StreamingTsDemux`] fed the body bytes.
pub struct TsHttpClient {
    policy: TsHttpPolicy,
    demux: StreamingTsDemux,
    opens: u64,
}

fn drain_demux(demux: &mut StreamingTsDemux, mut sink: impl FnMut(Event)) -> bool {
    let mut sampled = false;
    while let Some(ev) = demux.poll_event() {
        if matches!(ev, DemuxEvent::Sample { .. }) {
            sampled = true;
        }
        sink(Event::Demux(ev));
    }
    sampled
}

impl TsHttpClient {
    /// Create a new client (T1).
    ///
    /// An inconsistent `config` is replaced by [`Config::clamped`] rather
    /// than rejected; call [`Config::validate`] first to detect it.
    pub fn new(url: impl Into<String>, config: Config, now_ms: u64) -> Self {
        let policy = TsHttpPolicy::new(url, config, now_ms);
        let opens = policy.open_count();
        Self {
            policy,
            demux: StreamingTsDemux::new(),
            opens,
        }
    }

    fn sync_demux(&mut self) {
        let opens = self.policy.open_count();
        if opens != self.opens {
            self.opens = opens;
            self.demux = StreamingTsDemux::new();
        }
    }

    fn flush_demux(&mut self, now_ms: u64) {
        self.demux.finish();
        let policy = &mut self.policy;
        if drain_demux(&mut self.demux, |ev| policy.push_event(ev)) {
            policy.on_sample(now_ms);
        }
    }

    /// Poll next action.
    pub fn poll(&mut self) -> Option<Action> {
        self.policy.poll()
    }

    /// Poll next event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.policy.poll_event()
    }

    /// Response headers received (T2-T4).
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now_ms: u64) {
        self.policy.on_response(status, headers, now_ms);
    }

    /// Feed body bytes (T5, T14).
    pub fn feed(&mut self, chunk: &[u8], now_ms: u64) {
        if chunk.is_empty() || !self.policy.accepts_bytes() {
            return;
        }
        self.demux.feed(chunk);
        let policy = &mut self.policy;
        let sampled = drain_demux(&mut self.demux, |ev| policy.push_event(ev));
        policy.on_bytes(chunk.len(), now_ms, sampled);
    }

    /// Body ended (T7, T8).
    pub fn on_eof(&mut self, now_ms: u64) {
        if self.policy.is_ended() {
            return;
        }
        self.flush_demux(now_ms);
        self.policy.on_eof(now_ms);
        self.sync_demux();
    }

    /// Connect/read failure (T4, T7, T8).
    pub fn on_error(&mut self, now_ms: u64) {
        if self.policy.is_ended() {
            return;
        }
        self.flush_demux(now_ms);
        self.policy.on_error(now_ms);
        self.sync_demux();
    }

    /// Caller-initiated reconnect (T13).
    pub fn reconnect(&mut self, now_ms: u64) {
        self.flush_demux(now_ms);
        self.policy.reconnect(now_ms);
        self.sync_demux();
    }

    /// Advance timers (T6, T10-T12, T15).
    pub fn poll_timeout(&mut self, now_ms: u64) {
        let demux = &mut self.demux;
        self.policy.poll_timeout_with(now_ms, |events| {
            demux.finish();
            drain_demux(demux, |ev| events.push_back(ev))
        });
        self.sync_demux();
    }

    /// Earliest deadline for poll_timeout.
    pub fn next_deadline(&self) -> Option<u64> {
        self.policy.next_deadline()
    }
}
