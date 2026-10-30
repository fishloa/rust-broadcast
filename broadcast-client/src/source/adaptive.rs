//! The two segment-pull session cores (HLS and DASH) behind one interface, so the session drives
//! either through the same callbacks.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::auth::AuthState;
use crate::error::Error;
use crate::events::{Action, Event};
use crate::playout::Source;
use crate::source::dash::DashSource;
use crate::source::fetch::wall::WallClock;
use crate::source::hls_session::HlsSession;

pub(crate) enum Adaptive {
    Hls(Box<HlsSession>),
    Dash(Box<DashSource>),
}

impl Adaptive {
    pub(crate) fn playout_source(&self) -> Source {
        match self {
            Self::Hls(s) => s.playout_source(),
            Self::Dash(s) => s.playout_source(),
        }
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        match self {
            Self::Hls(s) => s.take_actions(),
            Self::Dash(s) => s.take_actions(),
        }
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        match self {
            Self::Hls(s) => s.take_events(),
            Self::Dash(s) => s.take_events(),
        }
    }

    pub(crate) fn conns(&self) -> Vec<u32> {
        match self {
            Self::Hls(s) => s.conns(),
            Self::Dash(s) => s.conns(),
        }
    }

    pub(crate) fn poll(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.poll(now, auth),
            Self::Dash(s) => s.poll(now, auth),
        }
    }

    pub(crate) fn on_response(
        &mut self,
        conn: u32,
        status: u16,
        headers: &[(String, String)],
        now: u64,
        wall: Option<&WallClock>,
        auth: &mut AuthState,
    ) {
        match self {
            Self::Hls(s) => s.on_response(conn, status, headers, now, wall, auth),
            Self::Dash(s) => s.on_response(conn, status, headers, now, wall, auth),
        }
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.on_data(conn, bytes, now, auth),
            Self::Dash(s) => s.on_data(conn, bytes, now, auth),
        }
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: u32,
        error: Option<&str>,
        now: u64,
        auth: &mut AuthState,
    ) {
        match self {
            Self::Hls(s) => s.on_closed(conn, error, now, auth),
            Self::Dash(s) => s.on_closed(conn, error, now, auth),
        }
    }

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.resume(now, auth),
            Self::Dash(s) => s.resume(now, auth),
        }
    }

    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {
        match self {
            Self::Hls(s) => s.reconnect(now, auth),
            Self::Dash(s) => s.reconnect(now, auth),
        }
    }

    pub(crate) fn is_ended(&self) -> bool {
        match self {
            Self::Hls(s) => s.is_ended(),
            Self::Dash(s) => s.is_ended(),
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
        }
    }

    pub(crate) fn variant_switch_restarts_playout(&self) -> bool {
        matches!(self, Self::Hls(_))
    }

    pub(crate) fn set_wall_clock(&mut self, now_us: u64, unix_us: u64) {
        if let Self::Dash(s) = self {
            s.set_wall_clock(now_us, unix_us);
        }
    }
}
