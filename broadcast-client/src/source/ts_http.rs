//! Adapter over [`tshttp_runtime::TsHttpClient`].

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use url::Url;

use broadcast_common::clock33::PCR_HZ;
use transmux::{DemuxEvent, TrackId, TrackSpec};
use tshttp_runtime::{Config as TsConfig, TsHttpClient};

use crate::{Event, Status};

pub(crate) type Item = Option<Event>;

const MICROS_PER_MILLI: u64 = 1_000;

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
    last_pcr_27mhz: Option<u64>,
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
            last_pcr_27mhz: None,
        }
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
        self.ts
            .next_deadline()
            .map(|ms| ms.saturating_mul(MICROS_PER_MILLI))
    }

    /// Advance timers.
    pub fn poll_timeout(&mut self, now_us: u64) {
        self.ts.poll_timeout(now_us / MICROS_PER_MILLI);
        self.drain_ts_events();
    }

    /// Poll next tshttp action.
    pub fn poll_action(&mut self) -> Option<tshttp_runtime::Action> {
        self.ts.poll()
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
        self.events.push_back(Some(e));
    }

    /// Response callback.
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now_us: u64) {
        self.ts.on_response(status, headers, now_us / 1000);
        self.drain_ts_events();
    }

    /// Data callback.
    pub fn on_data(&mut self, bytes: &[u8], now_us: u64) {
        self.last_pcr_27mhz = None;
        if self.stalled && !bytes.is_empty() {
            self.push(Event::Status(Status::Live));
            self.stalled = false;
        }
        self.ts.feed(bytes, now_us / 1000);
        self.drain_ts_events();
    }

    /// Closed callback at transport level.
    pub fn on_closed(&mut self, error: Option<String>, now_us: u64) {
        if error.is_some() {
            self.ts.on_error(now_us / 1000);
        } else {
            self.ts.on_eof(now_us / 1000);
        }
        self.drain_ts_events();
    }

    /// Error callback.
    pub fn on_error(&mut self, now_us: u64) {
        self.ts.on_error(now_us / 1000);
        self.drain_ts_events();
    }

    /// Reconnect.
    pub fn reconnect(&mut self, now_us: u64) {
        self.ts.reconnect(now_us / 1000);
        self.drain_ts_events();
    }

    fn drain_ts_events(&mut self) {
        while let Some(ev) = self.ts.poll_event() {
            self.map_ts_event(ev);
        }
    }

    fn map_ts_event(&mut self, e: tshttp_runtime::Event) {
        match e {
            tshttp_runtime::Event::Finite { expected_bytes } => {
                self.push(Event::Finite { expected_bytes });
            }
            tshttp_runtime::Event::Ended => {
                self.push(Event::Status(Status::Ended));
            }
            tshttp_runtime::Event::Connected => {
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
                self.events.push_back(None);
            }
            tshttp_runtime::Event::Demux(DemuxEvent::TrackAdded(spec)) => {
                self.tracks.insert(spec.track_id, spec);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::TrackUpdated(spec)) => {
                self.tracks.insert(spec.track_id, spec);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::TrackRemoved { track_id, .. }) => {
                self.tracks.remove(&track_id);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(DemuxEvent::Sample {
                track_id, sample, ..
            }) => {
                if !self.conn_seen_first_sample {
                    if self.tracks_dirty {
                        let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                        self.push(Event::Tracks(vals));
                        self.tracks_dirty = false;
                    }
                    if self.conn_connected {
                        self.push(Event::Status(Status::Live));
                    }
                    self.conn_seen_first_sample = true;
                } else if self.tracks_dirty {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.push(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.push(Event::Sample {
                    track: track_id,
                    sample,
                    join: None,
                });
            }
            tshttp_runtime::Event::Demux(DemuxEvent::DisplayChanged {
                track_id,
                display,
                pts,
                ..
            }) => {
                if self.tracks_dirty {
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
                self.push(Event::Status(Status::Stalled { secs }));
                self.stalled = true;
            }
            tshttp_runtime::Event::Reconnecting { attempt } => {
                self.push(Event::Status(Status::Reconnecting { attempt }));
            }
            tshttp_runtime::Event::NotFound { status: _ } => {
                self.push(Event::Status(Status::NotFound));
            }
            tshttp_runtime::Event::Starved(k) => {
                self.push(Event::Starved(k));
            }
            _ => {}
        }
    }
}
