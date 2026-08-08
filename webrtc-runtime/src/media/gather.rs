//! Server-reflexive candidate gathering — a one-shot STUN Binding
//! transaction (RFC 8489 §7.2.1: `rtc-stun`'s [`Client`] applies the
//! retransmission schedule) against a configured STUN server, run on the
//! same push/pull shape as the rest of [`super::MediaTransport`].
//!
//! This is *not* the ICE agent's own STUN traffic (connectivity checks):
//! it is how a [`super::MediaTransport`] learns its own server-reflexive
//! (`srflx`) address (RFC 8445 §5.1.1.2) to hand to the ICE agent as a new
//! local candidate.

use std::net::SocketAddr;
use std::time::Instant;

use bytes::BytesMut;
use rtc_shared::{TaggedBytesMut, TransportContext, TransportProtocol};
use rtc_stun::agent::StunEvent;
use rtc_stun::client::{Client, ClientBuilder};
use rtc_stun::message::{BINDING_REQUEST, Getter, Message, TransactionId};
use rtc_stun::xoraddr::XorMappedAddress;
use sansio::Protocol;

use crate::Error;

/// Drives one Binding request/response exchange against `server`.
pub(super) struct StunGather {
    client: Client,
    server: SocketAddr,
    done: bool,
}

impl StunGather {
    /// Build the gatherer and queue its Binding request; the request itself
    /// is drained via [`Self::poll_transmit`].
    pub(super) fn new(local: SocketAddr, server: SocketAddr) -> Result<Self, Error> {
        let mut client = ClientBuilder::new()
            .build(local, server, TransportProtocol::UDP)
            .map_err(|e| Error::Media(format!("build stun client: {e}")))?;

        let mut msg = Message::new();
        msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])
            .map_err(|e| Error::Media(format!("build stun binding request: {e}")))?;
        Protocol::handle_write(&mut client, msg)
            .map_err(|e| Error::Media(format!("queue stun binding request: {e}")))?;

        Ok(Self {
            client,
            server,
            done: false,
        })
    }

    /// The STUN server this gatherer is talking to.
    pub(super) fn server(&self) -> SocketAddr {
        self.server
    }

    /// Whether the transaction has concluded (success, timeout, or close).
    /// [`super::MediaTransport`] drops the gatherer once this is true.
    pub(super) fn done(&self) -> bool {
        self.done
    }

    /// The next outbound datagram to send to [`Self::server`], if any.
    pub(super) fn poll_transmit(&mut self) -> Option<TaggedBytesMut> {
        Protocol::poll_write(&mut self.client)
    }

    /// Feed an inbound datagram already known to be from [`Self::server`].
    ///
    /// Returns the resolved server-reflexive address once the Binding
    /// response arrives.
    pub(super) fn handle_datagram(
        &mut self,
        now: Instant,
        local: SocketAddr,
        data: &[u8],
    ) -> Result<Option<SocketAddr>, Error> {
        let tagged = TaggedBytesMut {
            now,
            transport: TransportContext {
                local_addr: local,
                peer_addr: self.server,
                ecn: None,
                transport_protocol: TransportProtocol::UDP,
            },
            message: BytesMut::from(data),
        };
        Protocol::handle_read(&mut self.client, tagged)
            .map_err(|e| Error::Media(format!("stun client handle_read: {e}")))?;

        // At most one event is meaningful per datagram here: this gatherer
        // runs a single transaction, so the first event settles it either
        // way (a `while` would never loop a second time).
        if let Some(event) = Protocol::poll_event(&mut self.client) {
            match event {
                StunEvent::Message(msg) => {
                    self.done = true;
                    let mut xor_addr = XorMappedAddress::default();
                    xor_addr
                        .get_from(&msg)
                        .map_err(|e| Error::Media(format!("read XOR-MAPPED-ADDRESS: {e}")))?;
                    return Ok(Some(SocketAddr::new(xor_addr.ip, xor_addr.port)));
                }
                StunEvent::TransactionTimeOut
                | StunEvent::TransactionStopped
                | StunEvent::AgentClosed => {
                    self.done = true;
                }
            }
        }
        Ok(None)
    }

    /// Drive retransmission timing (RFC 8489 §7.2.1).
    pub(super) fn handle_timeout(&mut self, now: Instant) {
        let _ = Protocol::handle_timeout(&mut self.client, now);
    }
}
