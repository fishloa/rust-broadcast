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
        // RFC 8489 §5: the Transaction ID "MUST be uniformly random ... and
        // cryptographically random" (see `docs/rfc8489-stun.md` §1).
        // `TransactionId::default()` is a plain `#[derive(Default)]` (all
        // zero bytes) — NOT random at all (issue #948 item 4, confirmed by
        // this crate's own test below before the fix: every Binding
        // request went out with transaction ID 0).
        //
        // `TransactionId::new()` is what RFC 8489 requires instead —
        // verified by reading `rtc-stun` 0.20.0's own source (not assumed
        // from its name), pinned in this workspace's `Cargo.lock`:
        //
        //   $CARGO_HOME/registry/src/.../rtc-stun-0.20.0/src/message.rs:48
        //     pub fn new() -> Self {
        //         let mut b = TransactionId([0u8; TRANSACTION_ID_SIZE]);
        //         rand::rng().fill(&mut b.0);   // doc comment: "using crypto/rand as source"
        //         b
        //     }
        //
        // `rand::rng()` resolves (per this workspace's locked `rand
        // 0.10.2`) to `rand::rngs::thread::rng()` → `ThreadRng`, whose
        // generator core is, per that crate's own source
        // ($CARGO_HOME/registry/src/.../rand-0.10.2/src/rngs/thread.rs:41):
        //
        //     type Core = chacha20::ChaChaCore<chacha20::R12, chacha20::variants::Legacy>;
        //
        // i.e. ChaCha12, a stream cipher used as a CSPRNG — seeded from
        // `SysRng` (the OS entropy source, via `Core::try_from_rng(&mut
        // SysRng)` at thread.rs:63) and periodically reseeded from it
        // (`RESEED_BLOCK_THRESHOLD`, thread.rs:37). That satisfies RFC
        // 8489 §5's "cryptographically random" on both counts: the
        // generator is a CSPRNG, and it is seeded from real entropy, not a
        // fixed or predictable value.
        msg.build(&[Box::new(TransactionId::new()), Box::new(BINDING_REQUEST)])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The 96-bit Transaction ID sits at byte offset 8..20 of a STUN
    /// message (RFC 8489 §5, `docs/rfc8489-stun.md` §1's own header
    /// diagram: 2 (type) + 2 (length) + 4 (magic cookie) = 8 header bytes
    /// before it starts).
    const TRANSACTION_ID_OFFSET: usize = 8;
    const TRANSACTION_ID_LEN: usize = 12;

    fn queued_request_bytes(gather: &mut StunGather) -> Vec<u8> {
        gather
            .poll_transmit()
            .expect("StunGather::new must queue its Binding request")
            .message
            .to_vec()
    }

    /// Bite test for issue #948 item 4: before the fix, `gather.rs` built
    /// its Binding request with `Box::<TransactionId>::default()`, which is
    /// a plain `#[derive(Default)]` on `rtc_stun::message::TransactionId` —
    /// i.e. all-zero bytes, not random at all. Swapping `TransactionId::new()`
    /// back to `TransactionId::default()` in `StunGather::new` makes this
    /// test fail (both transaction IDs would be the same all-zero value);
    /// restoring `::new()` makes it pass again.
    #[test]
    fn binding_requests_get_distinct_transaction_ids() {
        let local: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server: SocketAddr = "127.0.0.1:3478".parse().unwrap();

        let mut first = StunGather::new(local, server).unwrap();
        let mut second = StunGather::new(local, server).unwrap();

        let first_bytes = queued_request_bytes(&mut first);
        let second_bytes = queued_request_bytes(&mut second);

        let first_tid =
            &first_bytes[TRANSACTION_ID_OFFSET..TRANSACTION_ID_OFFSET + TRANSACTION_ID_LEN];
        let second_tid =
            &second_bytes[TRANSACTION_ID_OFFSET..TRANSACTION_ID_OFFSET + TRANSACTION_ID_LEN];

        assert_ne!(
            first_tid,
            &[0u8; TRANSACTION_ID_LEN][..],
            "transaction ID must not be the all-zero Default value (RFC 8489 §5 requires \
             cryptographic randomness)"
        );
        assert_ne!(
            first_tid, second_tid,
            "two independently-built Binding requests must not share a transaction ID"
        );
    }
}
