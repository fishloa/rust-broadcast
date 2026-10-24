//! Adapter over [`tshttp_runtime::TsHttpClient`].

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;

use url::Url;

use broadcast_common::clock33::PCR_HZ;
use transmux::{DemuxEvent, TrackId, TrackSpec};
use tshttp_runtime::{Config as TsConfig, TsHttpClient};

use mpeg_ts::pid::Pid;

use crate::source::ts_http_hold::{DeclaredStreams, HoldLimits, SETTLE_US};
use crate::{Event, Status};

pub(crate) type Item = Option<Event>;

const MICROS_PER_MILLI: u64 = 1_000;
const MICROS_PER_SEC: i128 = 1_000_000;

struct HoldState {
    started_us: u64,
    bytes: usize,
    quiet_bytes: usize,
    first_pts: BTreeMap<TrackId, i64>,
    span_us: u64,
    settle_from_us: u64,
    settle_base_span_us: u64,
}

fn to_27mhz(ticks: u64, clock_hz: u32) -> u64 {
    if clock_hz == 0 {
        return ticks;
    }
    u64::try_from(u128::from(ticks) * u128::from(PCR_HZ) / u128::from(clock_hz)).unwrap_or(u64::MAX)
}

/// Adapter around the TS-over-HTTP client.
pub struct TsHttpSource {
    ts: TsHttpClient,
    url: Url,
    events: VecDeque<Item>,
    tracks: BTreeMap<TrackId, TrackSpec>,
    tracks_dirty: bool,
    conn_connected: bool,
    conn_seen_first_sample: bool,
    connection_count: u32,
    stalled: bool,
    failure_announced: bool,
    caller_actions: VecDeque<tshttp_runtime::Action>,
    last_pcr_27mhz: Option<u64>,
    now_us: u64,
    declared: Option<DeclaredStreams>,
    resolved: BTreeSet<Pid>,
    hold: Option<HoldState>,
    held: VecDeque<Item>,
    hold_limits: HoldLimits,
}

impl TsHttpSource {
    /// Create a source.
    pub fn new(url: Url, config: TsConfig, now_us: u64) -> Self {
        Self {
            ts: TsHttpClient::new(url.as_str(), config, now_us / 1000),
            url,
            events: VecDeque::new(),
            tracks: BTreeMap::new(),
            tracks_dirty: false,
            conn_connected: false,
            conn_seen_first_sample: false,
            connection_count: 0,
            stalled: false,
            failure_announced: false,
            caller_actions: VecDeque::new(),
            last_pcr_27mhz: None,
            now_us,
            declared: Some(DeclaredStreams::new()),
            resolved: BTreeSet::new(),
            hold: None,
            held: VecDeque::new(),
            hold_limits: HoldLimits::from_config(&crate::events::Config::default()),
        }
    }

    pub(crate) fn with_hold_limits(mut self, limits: HoldLimits) -> Self {
        self.hold_limits = limits;
        self
    }

    /// Request URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Set request URL.
    pub fn set_url(&mut self, url: Url) {
        self.url = url;
    }

    /// Last PCR (27 MHz) observed during the current chunk's demux.
    pub fn take_last_pcr(&mut self) -> Option<u64> {
        self.last_pcr_27mhz.take()
    }

    /// Connections that reached a successful response so far; an attempt that failed before
    /// connecting does not count.
    pub fn connection_count(&self) -> u32 {
        self.connection_count
    }

    #[cfg(test)]
    pub(crate) fn inject(&mut self, item: Item) {
        self.events.push_back(item);
    }

    pub(crate) fn next_deadline_us(&self) -> Option<u64> {
        let ts = self
            .ts
            .next_deadline()
            .map(|ms| ms.saturating_mul(MICROS_PER_MILLI));
        let hold = self.hold.as_ref().map(|h| {
            let wall = h.started_us.saturating_add(self.hold_limits.max_wall_us);
            if self.active_resolved() {
                wall.min(h.settle_from_us.saturating_add(SETTLE_US))
            } else {
                wall
            }
        });
        match (ts, hold) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Advance timers.
    pub fn poll_timeout(&mut self, now_us: u64) {
        self.now_us = now_us;
        self.ts.poll_timeout(now_us / MICROS_PER_MILLI);
        self.drain_ts_events();
    }

    /// Poll next tshttp action.
    pub fn poll_action(&mut self) -> Option<tshttp_runtime::Action> {
        if let Some(action) = self.caller_actions.pop_front() {
            return Some(action);
        }
        let action = self.ts.poll()?;
        if matches!(action, tshttp_runtime::Action::Reconnect { .. }) {
            if self.failure_announced {
                self.failure_announced = false;
            } else {
                self.push(Event::Status(Status::Reconnecting { attempt: 0 }));
            }
        }
        Some(action)
    }

    /// Poll next mapped event.
    pub fn poll_event(&mut self) -> Option<Event> {
        loop {
            if let Some(e) = self.poll_item()? {
                return Some(e);
            }
        }
    }

    pub(crate) fn poll_item(&mut self) -> Option<Item> {
        self.drain_ts_events();
        self.events.pop_front()
    }

    fn push(&mut self, e: Event) {
        self.push_item(Some(e));
    }

    fn push_item(&mut self, item: Item) {
        if self.hold.is_some() {
            self.held.push_back(item);
        } else {
            self.events.push_back(item);
        }
    }

    fn active_resolved(&self) -> bool {
        self.declared
            .as_ref()
            .is_some_and(|d| d.active_resolved(&self.resolved))
    }

    fn note_resolved(&mut self, pid: Option<Pid>) {
        let Some(pid) = pid else {
            return;
        };
        if self.resolved.insert(pid)
            && let Some(h) = self.hold.as_mut()
        {
            h.quiet_bytes = 0;
            h.settle_from_us = self.now_us;
            h.settle_base_span_us = h.span_us;
        }
    }

    fn release_hold(&mut self) {
        if self.hold.take().is_none() {
            return;
        }
        self.declared = None;
        if self.tracks_dirty {
            let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
            self.events.push_back(Some(Event::Tracks(vals)));
            self.tracks_dirty = false;
        }
        self.events.append(&mut self.held);
    }

    fn start_hold_if_unresolved(&mut self) {
        if self.hold.is_some() || self.conn_seen_first_sample || self.hold_limits.disabled() {
            return;
        }
        if self
            .declared
            .as_ref()
            .is_some_and(|d| !d.all_resolved(&self.resolved))
        {
            self.hold = Some(HoldState {
                started_us: self.now_us,
                bytes: 0,
                quiet_bytes: 0,
                first_pts: BTreeMap::new(),
                span_us: 0,
                settle_from_us: self.now_us,
                settle_base_span_us: 0,
            });
        }
    }

    fn release_hold_if_due(&mut self) {
        let Some(h) = self.hold.as_ref() else {
            return;
        };
        let settled = self.now_us.saturating_sub(h.settle_from_us) >= SETTLE_US
            || h.span_us.saturating_sub(h.settle_base_span_us) >= SETTLE_US;
        let due = self.hold_limits.reached(
            h.bytes,
            h.quiet_bytes,
            h.span_us,
            self.now_us.saturating_sub(h.started_us),
        ) || self
            .declared
            .as_ref()
            .is_none_or(|d| d.all_resolved(&self.resolved))
            || (settled && self.active_resolved());
        if due {
            self.release_hold();
        }
    }

    fn note_held_sample(&mut self, track: TrackId, pts: Option<i64>) {
        let (Some(pts), Some(spec)) = (pts, self.tracks.get(&track)) else {
            return;
        };
        let timescale = i128::from(spec.timescale.max(1));
        let Some(h) = self.hold.as_mut() else {
            return;
        };
        let first = *h.first_pts.entry(track).or_insert(pts);
        let span = i128::from(pts.saturating_sub(first)) * MICROS_PER_SEC / timescale;
        h.span_us = h
            .span_us
            .max(u64::try_from(span.max(0)).unwrap_or(u64::MAX));
    }

    /// Response callback.
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now_us: u64) {
        self.now_us = now_us;
        self.ts.on_response(status, headers, now_us / 1000);
        self.drain_ts_events();
    }

    /// Data callback.
    pub fn on_data(&mut self, bytes: &[u8], now_us: u64) {
        self.now_us = now_us;
        self.last_pcr_27mhz = None;
        if let Some(d) = self.declared.as_mut() {
            d.feed(bytes);
        }
        if let Some(h) = self.hold.as_mut() {
            h.bytes = h.bytes.saturating_add(bytes.len());
            h.quiet_bytes = h.quiet_bytes.saturating_add(bytes.len());
        }
        if self.stalled && !bytes.is_empty() {
            self.push(Event::Status(Status::Live));
            self.stalled = false;
        }
        self.ts.feed(bytes, now_us / 1000);
        self.drain_ts_events();
    }

    /// Closed callback at transport level.
    pub fn on_closed(&mut self, error: Option<String>, now_us: u64) {
        self.now_us = now_us;
        if error.is_some() {
            self.ts.on_error(now_us / 1000);
        } else {
            self.ts.on_eof(now_us / 1000);
        }
        self.drain_ts_events();
    }

    /// Error callback.
    pub fn on_error(&mut self, now_us: u64) {
        self.now_us = now_us;
        self.ts.on_error(now_us / 1000);
        self.drain_ts_events();
    }

    /// Reconnect.
    pub fn reconnect(&mut self, now_us: u64) {
        self.now_us = now_us;
        self.ts.reconnect(now_us / 1000);
        while let Some(action) = self.ts.poll() {
            self.caller_actions.push_back(action);
        }
        self.drain_ts_events();
    }

    fn drain_ts_events(&mut self) {
        while let Some(ev) = self.ts.poll_event() {
            self.map_ts_event(ev);
        }
        self.release_hold_if_due();
    }

    fn map_ts_event(&mut self, e: tshttp_runtime::Event) {
        match e {
            tshttp_runtime::Event::Finite { expected_bytes } => {
                self.release_hold();
                self.push(Event::Finite { expected_bytes });
            }
            tshttp_runtime::Event::Ended => {
                self.release_hold();
                self.push(Event::Status(Status::Ended));
            }
            tshttp_runtime::Event::Connected => {
                self.release_hold();
                self.declared = Some(DeclaredStreams::new());
                self.resolved.clear();
                self.connection_count = self.connection_count.saturating_add(1);
                if self.connection_count > 1 {
                    self.push(Event::Discontinuity);
                }
                self.push(Event::Status(Status::Establishing));
                self.conn_connected = true;
                self.conn_seen_first_sample = false;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::ClockReference {
                ticks, clock_hz, ..
            }) => {
                self.last_pcr_27mhz = Some(to_27mhz(ticks, clock_hz));
            }
            tshttp_runtime::Event::Demux(DemuxEvent::Discontinuity { .. }) => {
                self.push_item(None);
            }
            tshttp_runtime::Event::Demux(
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec),
            ) => {
                self.note_resolved(spec.source_pid);
                self.tracks.insert(spec.track_id, spec);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::TrackAbandoned { provenance, .. }) => {
                self.note_resolved(provenance.pid);
            }
            tshttp_runtime::Event::Demux(DemuxEvent::TrackRemoved { track_id, .. }) => {
                self.tracks.remove(&track_id);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::Sample {
                track_id, sample, ..
            }) => {
                let pts = sample.pts;
                if !self.conn_seen_first_sample {
                    self.start_hold_if_unresolved();
                    self.conn_seen_first_sample = true;
                    if self.hold.is_none() {
                        self.declared = None;
                        if self.tracks_dirty {
                            let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                            self.push(Event::Tracks(vals));
                            self.tracks_dirty = false;
                        }
                    }
                    if self.conn_connected {
                        self.push(Event::Status(Status::Live));
                    }
                } else if self.tracks_dirty && self.hold.is_none() {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.push(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.push(Event::Sample {
                    track: track_id,
                    sample,
                    join: None,
                });
                self.note_held_sample(track_id, pts);
            }
            tshttp_runtime::Event::Demux(DemuxEvent::DisplayChanged {
                track_id,
                display,
                pts,
                ..
            }) => {
                self.start_hold_if_unresolved();
                if self.tracks_dirty && self.hold.is_none() {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.push(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.push(Event::DisplayChanged {
                    track: track_id,
                    pts,
                    display,
                });
            }
            tshttp_runtime::Event::StallNotice { secs } => {
                self.release_hold();
                self.push(Event::Status(Status::Stalled { secs }));
                self.stalled = true;
            }
            tshttp_runtime::Event::Reconnecting { attempt } => {
                self.release_hold();
                self.failure_announced = true;
                self.push(Event::Status(Status::Reconnecting { attempt }));
            }
            tshttp_runtime::Event::NotFound { status: _ } => {
                self.release_hold();
                self.push(Event::Status(Status::NotFound));
            }
            tshttp_runtime::Event::Starved(k) => {
                self.release_hold();
                self.push(Event::Starved(k));
            }
            _ => {}
        }
    }
}
