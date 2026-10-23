//! Adapter over [`tshttp_runtime::TsHttpClient`].

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use url::Url;

use transmux::{DemuxEvent, TrackId, TrackSpec};
use tshttp_runtime::{Config as TsConfig, TsHttpClient};

use crate::{Event, Status};

/// Adapter around the TS-over-HTTP client.
pub struct TsHttpSource {
    ts: TsHttpClient,
    url: Url,
    events: VecDeque<Event>,
    tracks: BTreeMap<TrackId, TrackSpec>,
    tracks_dirty: bool,
    conn_connected: bool,
    conn_seen_first_sample: bool,
    connection_count: u32,
    stalled: bool,
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

    /// Connection count.
    pub fn connection_count(&self) -> u32 {
        self.connection_count
    }

    /// Advance timers.
    pub fn poll_timeout(&mut self, now_us: u64) {
        self.ts.poll_timeout(now_us / 1000);
        self.drain_ts_events();
    }

    /// Poll next tshttp action.
    pub fn poll_action(&mut self) -> Option<tshttp_runtime::Action> {
        let action = self.ts.poll();
        if let Some(tshttp_runtime::Action::Open { .. }) = &action {
            self.connection_count = self.connection_count.saturating_add(1);
        }
        action
    }

    /// Poll next mapped event.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.drain_ts_events();
        self.events.pop_front()
    }

    /// Response callback.
    pub fn on_response(&mut self, status: u16, headers: &[(String, String)], now_us: u64) {
        self.ts.on_response(status, headers, now_us / 1000);
        self.drain_ts_events();
    }

    /// Data callback.
    pub fn on_data(&mut self, bytes: &[u8], now_us: u64) {
        if self.stalled && !bytes.is_empty() {
            self.events.push_back(Event::Status(Status::Live));
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
                self.events.push_back(Event::Finite { expected_bytes });
            }
            tshttp_runtime::Event::Ended => {
                self.events.push_back(Event::Status(Status::Ended));
            }
            tshttp_runtime::Event::Connected => {
                if self.connection_count > 1 {
                    self.events.push_back(Event::Discontinuity);
                }
                self.events.push_back(Event::Status(Status::Establishing));
                self.conn_connected = true;
                self.conn_seen_first_sample = false;
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
                        self.events.push_back(Event::Tracks(vals));
                        self.tracks_dirty = false;
                    }
                    if self.conn_connected {
                        self.events.push_back(Event::Status(Status::Live));
                    }
                    self.conn_seen_first_sample = true;
                } else if self.tracks_dirty {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.events.push_back(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.events.push_back(Event::Sample {
                    track: track_id,
                    sample,
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
                    self.events.push_back(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.events.push_back(Event::DisplayChanged {
                    track: track_id,
                    pts,
                    display,
                });
            }
            tshttp_runtime::Event::StallNotice { secs } => {
                self.events
                    .push_back(Event::Status(Status::Stalled { secs }));
                self.stalled = true;
            }
            tshttp_runtime::Event::Reconnecting { attempt } => {
                self.events
                    .push_back(Event::Status(Status::Reconnecting { attempt }));
            }
            tshttp_runtime::Event::NotFound { status: _ } => {
                self.events.push_back(Event::Status(Status::NotFound));
            }
            tshttp_runtime::Event::Starved(k) => {
                self.events.push_back(Event::Starved(k));
            }
            _ => {}
        }
    }
}
