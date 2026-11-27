use alloc::collections::VecDeque;
use alloc::string::String;
use core::num::NonZeroUsize;
use core::time::Duration;

use broadcast_core::damage::DamagePolicy;
use transmux::{DemuxEvent, StreamingTsDemux};

use crate::tshttp::config::Config;
use crate::tshttp::events::{Action, Event};
use crate::tshttp::policy::TsHttpPolicy;
use broadcast_common::stage::Timestamp;
use http::{HeaderMap, StatusCode};

/// TS-over-HTTP pull session client (DESIGN.md): [`TsHttpPolicy`] plus a
/// [`StreamingTsDemux`] fed the body bytes.
pub struct TsHttpClient {
    policy: TsHttpPolicy,
    demux: StreamingTsDemux,
    opens: u64,
    /// The caller's PES unit ceiling, carried across a reconnect's fresh demux.
    max_pes_unit_bytes: Option<NonZeroUsize>,
    /// The caller's decision about each damage report, kept here so a drain
    /// applies it without borrowing the event queue as well.
    damage_policy: DamagePolicy,
}

/// What one demux drain produced.
struct Drained {
    /// At least one sample was produced.
    sampled: bool,
    /// The damage policy refused a report: the session stops with this error.
    refused: bool,
}

/// Drain `demux` into `events`, applying `policy` to every damage report.
fn drain_demux(
    demux: &mut StreamingTsDemux,
    policy: &DamagePolicy,
    events: &mut VecDeque<Event>,
) -> Drained {
    let mut drained = Drained {
        sampled: false,
        refused: false,
    };
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::Damage(damage) => match policy.admit(damage) {
                Ok(damage) => events.push_back(Event::Damage(damage)),
                Err(error) => {
                    // The session stops here: nothing the same drain held back
                    // is delivered after the refusal.
                    drained.refused = true;
                    events.push_back(Event::DemuxError(transmux::Error::Damage(error)));
                    break;
                }
            },
            other => {
                drained.sampled |= matches!(other, DemuxEvent::Sample { .. });
                events.push_back(Event::Demux(Box::new(other)));
            }
        }
    }
    drained
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
        let max_pes_unit_bytes = config.max_pes_unit_bytes;
        let damage_policy = config.damage_policy.clone();
        let policy = TsHttpPolicy::with_seed(url, config, now, seed);
        let opens = policy.open_count();
        Self {
            policy,
            demux: StreamingTsDemux::new().with_max_pes_unit_bytes(max_pes_unit_bytes),
            opens,
            max_pes_unit_bytes,
            damage_policy,
        }
    }

    fn sync_demux(&mut self) {
        let opens = self.policy.open_count();
        if opens != self.opens {
            self.opens = opens;
            self.demux = StreamingTsDemux::new().with_max_pes_unit_bytes(self.max_pes_unit_bytes);
        }
    }

    fn flush_demux(&mut self, now: Timestamp) {
        let finished = self.demux.finish();
        let drained = drain_demux(
            &mut self.demux,
            &self.damage_policy,
            self.policy.events_mut(),
        );
        if drained.sampled {
            self.policy.on_sample(now);
        }
        if drained.refused {
            self.policy.stop();
        }
        if let Err(error) = finished {
            self.policy.push_event(Event::DemuxError(error));
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
    pub fn on_response(&mut self, status: StatusCode, headers: &HeaderMap, now: Timestamp) {
        self.policy.on_response(status, headers, now);
    }

    /// Server-requested retry delay (from a `Retry-After` header), to be applied to the next failed
    /// response. Call before [`Self::on_response`].
    pub fn set_retry_after(&mut self, retry_after: Duration) {
        self.policy.set_retry_after(retry_after);
    }

    /// Hold or release the stall timers because the host stopped or resumed reading.
    pub fn set_read_paused(&mut self, paused: bool, now: Timestamp) {
        self.policy.set_read_paused(paused, now);
    }

    /// Feed body bytes (T5, T14).
    pub fn feed(&mut self, chunk: &[u8], now: Timestamp) {
        if chunk.is_empty() || !self.policy.accepts_bytes() {
            return;
        }
        let fed = self.demux.feed(chunk);
        let drained = drain_demux(
            &mut self.demux,
            &self.damage_policy,
            self.policy.events_mut(),
        );
        if drained.refused {
            self.policy.stop();
            return;
        }
        if let Err(error) = fed {
            self.policy.push_event(Event::DemuxError(error));
        }
        self.policy.on_bytes(chunk.len(), now, drained.sampled);
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

    /// Caller-initiated reconnect (T13).
    pub fn reconnect(&mut self, now: Timestamp) {
        self.flush_demux(now);
        self.policy.reconnect(now);
        self.sync_demux();
    }

    /// Advance timers (T6, T10-T12, T15).
    pub fn poll_timeout(&mut self, now: Timestamp) {
        let demux = &mut self.demux;
        let damage_policy = &self.damage_policy;
        let mut error = None;
        let mut refused = false;
        self.policy.poll_timeout_with(now, |events| {
            if let Err(e) = demux.finish() {
                error = Some(e);
            }
            let drained = drain_demux(demux, damage_policy, events);
            refused = drained.refused;
            drained.sampled
        });
        if refused {
            self.policy.stop();
            return;
        }
        if let (Some(error), false) = (error, self.policy.is_ended()) {
            self.policy.push_event(Event::DemuxError(error));
        }
        self.sync_demux();
    }

    /// Earliest deadline for [`Self::poll_timeout`].
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.policy.next_deadline()
    }
}
