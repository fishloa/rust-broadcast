//! `tokio_util::codec` adapters over the sans-IO RTMP cores (W1 SP1.1).
//!
//! The sessions buffer partial handshake/chunk input internally, so the codecs hand
//! every inbound chunk to them and cannot tell a clean end of stream from one that
//! cuts a message short: a partial message at EOF is treated as a clean EOF.

use std::io;

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::client::{ClientConfig, ClientEvent, ClientSession};
use crate::server::{ServerEvent, ServerSession};

pub(crate) fn io_err(e: crate::RtmpError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Feeds every inbound chunk to the session. The session's reply bytes are NOT
/// returned as items: they are parked in `reply` and moved into the framed
/// write buffer by the adapter in the same synchronous step that receives the
/// events, so there is no await point at which a reply can be dropped.
#[derive(Debug)]
pub(crate) struct ServerCodec {
    session: ServerSession,
    reply: Vec<u8>,
}

impl ServerCodec {
    pub(crate) fn new(session: ServerSession) -> Self {
        Self {
            session,
            reply: Vec::new(),
        }
    }

    pub(crate) fn take_reply(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.reply)
    }
}

impl Decoder for ServerCodec {
    type Item = Vec<ServerEvent>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<ServerEvent>>> {
        if src.is_empty() {
            return Ok(None);
        }
        let chunk = src.split();
        let (reply, events) = self.session.handle_data(&chunk).map_err(io_err)?;
        self.reply.extend_from_slice(&reply);
        Ok(Some(events))
    }
}

impl Encoder<Vec<u8>> for ServerCodec {
    type Error = io::Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> io::Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}

/// Client-side counterpart of [`ServerCodec`]: inbound chunks go to
/// [`ClientSession::handle_data`]; its reply bytes are parked for the adapter to
/// move into the framed write buffer in the same synchronous step.
#[derive(Debug)]
pub(crate) struct ClientCodec {
    session: ClientSession,
    reply: Vec<u8>,
}

impl ClientCodec {
    pub(crate) fn new(config: ClientConfig) -> Self {
        Self {
            session: ClientSession::new(config),
            reply: Vec::new(),
        }
    }

    pub(crate) fn session_mut(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub(crate) fn take_reply(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.reply)
    }
}

impl Decoder for ClientCodec {
    type Item = Vec<ClientEvent>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<ClientEvent>>> {
        if src.is_empty() {
            return Ok(None);
        }
        let chunk = src.split();
        let (reply, events) = self.session.handle_data(&chunk).map_err(io_err)?;
        self.reply.extend_from_slice(&reply);
        Ok(Some(events))
    }
}

impl Encoder<Vec<u8>> for ClientCodec {
    type Error = io::Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> io::Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}
