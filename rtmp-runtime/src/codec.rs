//! `tokio_util::codec` adapters over the sans-IO RTMP cores (W1 SP1.1).
//!
//! The sessions buffer partial handshake/chunk input internally, so the codecs
//! hand every inbound chunk to them. A stream that ends while a message is
//! still in progress is not a clean EOF: the adapter asks the session how many
//! bytes it is still holding and surfaces one `Damage` report
//! ([`crate::damage`]) for the truncated message, once, before the stream
//! reports its end.

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
    /// Bytes the session holds that have not yet formed a message.
    fn pending_input(&self) -> usize;
    /// One `Damage` event for a message the input ended inside.
    fn truncated_event() -> Self::Event;
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

    fn pending_input(&self) -> usize {
        self.pending_len()
    }

    fn truncated_event() -> ServerEvent {
        ServerEvent::Damage(Box::new(crate::damage::truncated()))
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

    fn pending_input(&self) -> usize {
        self.pending_len()
    }

    fn truncated_event() -> ClientEvent {
        ClientEvent::Damage(Box::new(crate::damage::truncated()))
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
    /// Whether the end-of-input truncation report has been surfaced, so it is
    /// not repeated on every later `decode_eof` poll.
    eof_reported: bool,
}

impl<T> SessionCodec<T> {
    pub(crate) fn new(session: T) -> Self {
        Self {
            session,
            reply: Vec::new(),
            eof_reported: false,
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

    fn decode_eof(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<T::Event>>> {
        let mut events = self.decode(src)?.unwrap_or_default();
        if !self.eof_reported && self.session.pending_input() > 0 {
            self.eof_reported = true;
            events.push(T::truncated_event());
        }
        if events.is_empty() {
            Ok(None)
        } else {
            Ok(Some(events))
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handshake::HANDSHAKE_PACKET_LEN;

    /// An end of input inside a message is reported, once, and only when
    /// there really is a partial message buffered.
    #[test]
    fn eof_inside_a_message_is_reported_once() {
        let mut codec = SessionCodec::new(ServerSession::with_defaults());
        let mut c0_c1 = vec![0u8; 1 + HANDSHAKE_PACKET_LEN];
        c0_c1[0] = crate::handshake::RTMP_VERSION;
        codec
            .decode(&mut BytesMut::from(c0_c1.as_slice()))
            .expect("C0+C1");

        // One byte of a basic header: a message in progress, then EOF.
        let mut tail = BytesMut::from(&[0x04u8][..]);
        let events = codec
            .decode_eof(&mut tail)
            .expect("decode_eof")
            .expect("a truncated message is an event");
        assert_eq!(events.len(), 1);
        let ServerEvent::Damage(damage) = &events[0] else {
            panic!("expected a damage report, got {events:?}");
        };
        assert_eq!(
            damage.kind(),
            broadcast_core::damage::DamageKind::InputTruncated
        );

        // A second poll reports nothing further.
        let mut empty = BytesMut::new();
        assert!(
            codec
                .decode_eof(&mut empty)
                .expect("second decode_eof")
                .is_none(),
            "the truncation is reported once"
        );
    }

    /// A clean stream end reports nothing: the handshake completed and no
    /// chunk is partly buffered.
    #[test]
    fn eof_at_a_message_boundary_reports_nothing() {
        let mut codec = SessionCodec::new(ServerSession::with_defaults());
        let mut c0_c1 = vec![0u8; 1 + HANDSHAKE_PACKET_LEN];
        c0_c1[0] = crate::handshake::RTMP_VERSION;
        codec
            .decode(&mut BytesMut::from(c0_c1.as_slice()))
            .expect("C0+C1");
        codec
            .decode(&mut BytesMut::from(&vec![0u8; HANDSHAKE_PACKET_LEN][..]))
            .expect("C2");
        let mut empty = BytesMut::new();
        assert!(codec.decode_eof(&mut empty).expect("decode_eof").is_none());
    }
}
