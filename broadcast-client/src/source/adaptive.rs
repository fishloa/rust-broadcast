//! The two segment-pull session cores (HLS and DASH) behind one interface, so the session drives
//! either through the same callbacks.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ConnId;
use crate::error::{Error, TransportError};
use crate::events::{Action, Event};
use crate::playout::Source;
use crate::source::dash::DashSource;
use crate::source::fetch::wall::WallClock;
use crate::source::hls_session::HlsSession;
use crate::source::smooth::SmoothSource;
use broadcast_auth::AuthState;
use broadcast_common::stage::Timestamp;

pub(crate) enum Adaptive {
    Hls(Box<HlsSession>),
    Dash(Box<DashSource>),
    Smooth(Box<SmoothSource>),
}

impl Adaptive {
    pub(crate) fn playout_source(&self) -> Source {
        match self {
            Self::Hls(s) => s.playout_source(),
            Self::Dash(s) => s.playout_source(),
            Self::Smooth(s) => s.playout_source(),
        }
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        match self {
            Self::Hls(s) => s.take_actions(),
            Self::Dash(s) => s.take_actions(),
            Self::Smooth(s) => s.take_actions(),
        }
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        match self {
            Self::Hls(s) => s.take_events(),
            Self::Dash(s) => s.take_events(),
            Self::Smooth(s) => s.take_events(),
        }
    }

    pub(crate) fn conns(&self) -> Vec<ConnId> {
        match self {
            Self::Hls(s) => s.conns(),
            Self::Dash(s) => s.conns(),
            Self::Smooth(s) => s.conns(),
        }
    }

    pub(crate) fn poll(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.poll(now, auth),
            Self::Dash(s) => s.poll(now, auth),
            Self::Smooth(s) => s.poll(now, auth),
        }
    }

    pub(crate) fn on_response(
        &mut self,
        conn: ConnId,
        status: u16,
        headers: &[(String, String)],
        now: u64,
        wall: Option<&WallClock>,
        auth: &mut AuthState,
    ) {
        match self {
            Self::Hls(s) => s.on_response(conn, status, headers, now, wall, auth),
            Self::Dash(s) => s.on_response(conn, status, headers, now, wall, auth),
            Self::Smooth(s) => s.on_response(conn, status, headers, now, wall, auth),
        }
    }

    pub(crate) fn on_data(&mut self, conn: ConnId, bytes: &[u8], now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.on_data(conn, bytes, now, auth),
            Self::Dash(s) => s.on_data(conn, bytes, now, auth),
            Self::Smooth(s) => s.on_data(conn, bytes, now, auth),
        }
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: ConnId,
        error: Option<&TransportError>,
        now: u64,
        auth: &mut AuthState,
    ) {
        match self {
            Self::Hls(s) => s.on_closed(conn, error, now, auth),
            Self::Dash(s) => s.on_closed(conn, error, now, auth),
            Self::Smooth(s) => s.on_closed(conn, error, now, auth),
        }
    }

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.resume(now, auth),
            Self::Dash(s) => s.resume(now, auth),
            Self::Smooth(s) => s.resume(now, auth),
        }
    }

    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.reconnect(now, auth),
            Self::Dash(s) => s.reconnect(now, auth),
            Self::Smooth(s) => s.reconnect(now, auth),
        }
    }

    pub(crate) fn is_ended(&self) -> bool {
        match self {
            Self::Hls(s) => s.is_ended(),
            Self::Dash(s) => s.is_ended(),
            Self::Smooth(s) => s.is_ended(),
        }
    }

    pub(crate) fn select_variant(
        &mut self,
        index: usize,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        match self {
            Self::Hls(s) => s.select_variant(index, now, auth),
            Self::Dash(s) => s.select_variant(index, now, auth),
            Self::Smooth(s) => s.select_variant(index, now, auth),
        }
    }

    pub(crate) fn select_audio(
        &mut self,
        choice: Option<usize>,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        match self {
            Self::Hls(s) => s.select_audio(choice, now, auth),
            Self::Dash(s) => s.select_audio(choice, now, auth),
            Self::Smooth(s) => s.select_audio(choice, now, auth),
        }
    }

    pub(crate) fn variant_switch_restarts_playout(&self) -> bool {
        matches!(self, Self::Hls(_))
    }

    pub(crate) fn set_wall_clock(&mut self, now: Timestamp, unix: jiff::Timestamp) {
        match self {
            Self::Dash(s) => s.set_wall_clock(now, unix),
            Self::Smooth(s) => s.set_wall_clock(
                now.as_micros(),
                u64::try_from(unix.as_microsecond()).unwrap_or(u64::MAX),
            ),
            _ => {}
        }
    }
}
