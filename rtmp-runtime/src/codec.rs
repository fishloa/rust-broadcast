//! `tokio_util::codec` adapters over the sans-IO RTMP cores (W1 SP1.1).
//!
//! The sessions buffer partial handshake/chunk input internally, so the codecs hand
//! every inbound chunk to them and cannot tell a clean end of stream from one that
//! cuts a message short: a partial message at EOF is treated as a clean EOF.

use std::io;

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::client::{ClientEvent, ClientSession};
use crate::server::{ServerEvent, ServerSession};

pub(crate) fn io_err(e: crate::RtmpError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

pub(crate) trait RtmpSession {
    type Event: std::fmt::Debug;

    fn handle_data(
        &mut self,
        input: &[u8],
    ) -> Result<(Vec<u8>, Vec<Self::Event>), crate::RtmpError>;
    fn opens_stream(event: &Self::Event) -> bool;
    fn is_eof(event: &Self::Event) -> bool;
}

impl RtmpSession for ServerSession {
    type Event = ServerEvent;

    fn handle_data(
        &mut self,
        input: &[u8],
    ) -> Result<(Vec<u8>, Vec<ServerEvent>), crate::RtmpError> {
        ServerSession::handle_data(self, input)
    }

    fn opens_stream(event: &ServerEvent) -> bool {
        matches!(event, ServerEvent::Connected { .. })
    }

    fn is_eof(event: &ServerEvent) -> bool {
        matches!(event, ServerEvent::Eof)
    }
}

impl RtmpSession for ClientSession {
    type Event = ClientEvent;

    fn handle_data(
        &mut self,
        input: &[u8],
    ) -> Result<(Vec<u8>, Vec<ClientEvent>), crate::RtmpError> {
        ClientSession::handle_data(self, input)
    }

    fn opens_stream(event: &ClientEvent) -> bool {
        matches!(event, ClientEvent::Publishing)
    }

    fn is_eof(event: &ClientEvent) -> bool {
        matches!(event, ClientEvent::Closed)
    }
}

/// Feeds every inbound chunk to the session. The session's reply bytes are NOT
/// returned as items: they are parked in `reply` and moved into the framed
/// write buffer by the adapter in the same synchronous step that receives the
/// events, so there is no await point at which a reply can be dropped.
#[derive(Debug)]
pub(crate) struct SessionCodec<T> {
    session: T,
    reply: Vec<u8>,
}

impl<T> SessionCodec<T> {
    pub(crate) fn new(session: T) -> Self {
        Self {
            session,
            reply: Vec::new(),
        }
    }

    pub(crate) fn session_mut(&mut self) -> &mut T {
        &mut self.session
    }

    pub(crate) fn take_reply(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.reply)
    }
}

impl<T: RtmpSession> Decoder for SessionCodec<T> {
    type Item = Vec<T::Event>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<T::Event>>> {
        if src.is_empty() {
            return Ok(None);
        }
        let chunk = src.split();
        let (reply, events) = self.session.handle_data(&chunk).map_err(io_err)?;
        self.reply.extend_from_slice(&reply);
        Ok(Some(events))
    }
}

impl<T> Encoder<Vec<u8>> for SessionCodec<T> {
    type Error = io::Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> io::Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}

pub(crate) type ServerCodec = SessionCodec<ServerSession>;
pub(crate) type ClientCodec = SessionCodec<ClientSession>;
