use alloc::string::String;
use core::time::Duration;

use transmux::{DemuxEvent, StreamingTsDemux};

use crate::tshttp::config::Config;
use crate::tshttp::events::{Action, Event};
use crate::tshttp::policy::TsHttpPolicy;
use broadcast_common::stage::Timestamp;

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
        sink(Event::Demux(Box::new(ev)));
    }
    sampled
}

impl TsHttpClient {
    /// Create a client (T1), its retry jitter drawn from `seed` (see
    /// [`TsHttpPolicy::with_seed`]).
    ///
    /// An inconsistent `config` is replaced by [`Config::clamped`] rather
    /// than rejected; call [`Config::validate`] first to detect it.
    pub(crate) fn with_seed(
        url: impl Into<String>,
        config: Config,
        now: Timestamp,
        seed: u64,
    ) -> Self {
        let policy = TsHttpPolicy::with_seed(url, config, now, seed);
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

    fn flush_demux(&mut self, now: Timestamp) {
        self.demux.finish();
        let policy = &mut self.policy;
        if drain_demux(&mut self.demux, |ev| policy.push_event(ev)) {
            policy.on_sample(now);
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
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now: Timestamp) {
        self.policy.on_response(status, headers, now);
    }

    /// Server-requested retry delay (from a `Retry-After` header), to be applied to the next failed
    /// response. Call before [`Self::on_response`].
    pub fn set_retry_after(&mut self, retry_after: Duration) {
        self.policy.set_retry_after(retry_after);
    }

    /// Feed body bytes (T5, T14).
    pub fn feed(&mut self, chunk: &[u8], now: Timestamp) {
        if chunk.is_empty() || !self.policy.accepts_bytes() {
            return;
        }
        self.demux.feed(chunk);
        let policy = &mut self.policy;
        let sampled = drain_demux(&mut self.demux, |ev| policy.push_event(ev));
        policy.on_bytes(chunk.len(), now, sampled);
    }

    /// Body ended (T7, T8).
    pub fn on_eof(&mut self, now: Timestamp) {
        if self.policy.is_ended() {
            return;
        }
        self.flush_demux(now);
        self.policy.on_eof(now);
        self.sync_demux();
    }

    /// Connect/read failure (T4, T7, T8).
    pub fn on_error(&mut self, now: Timestamp) {
        if self.policy.is_ended() {
            return;
        }
        self.flush_demux(now);
        self.policy.on_error(now);
        self.sync_demux();
    }

    /// Advance timers (T6, T10-T12, T15).
    pub fn poll_timeout(&mut self, now: Timestamp) {
        let demux = &mut self.demux;
        self.policy.poll_timeout_with(now, |events| {
            demux.finish();
            drain_demux(demux, |ev| events.push_back(ev))
        });
        self.sync_demux();
    }

    /// Whether an action from [`Self::poll`] is to be acted on now: all of them are, except a
    /// reopen whose backoff would end past the end of the host clock, which the host must not act
    /// on at `now` (it never comes due).
    pub fn action_ready(&self, now: Timestamp) -> bool {
        self.policy.reconnect_ready(now)
    }

    /// Earliest deadline for [`Self::poll_timeout`].
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.policy.next_deadline()
    }
}
