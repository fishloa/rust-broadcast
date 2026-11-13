//! [`SocketDemux`]: several [`MediaTransport`](super::MediaTransport)s on one
//! UDP socket.
//!
//! A socket shared by many sessions has to decide, per datagram, which session
//! it belongs to. ICE gives exactly one handle for that: a connectivity check
//! (STUN Binding request) carries `USERNAME = "{receiver ufrag}:{sender
//! ufrag}"` (RFC 8445 §7.2.2), and the receiver ufrag is the session's own
//! local `ice-ufrag`. Everything else (DTLS, SRTP/SRTCP, STUN responses) has no
//! such handle and is routed by the sender's address: the 5-tuple a check from
//! that session arrived on.
//!
//! Sans-IO: this type owns no socket and reads no clock. The caller registers
//! each session's local ufrag, asks [`SocketDemux::classify`] where each
//! inbound datagram goes, and feeds it to that session's transport.
//!
//! # What is and is not trusted
//!
//! A check naming a known ufrag maps its source address to that session. The
//! check is *not* authenticated here (that needs the session's ICE password,
//! which the transport holds); a spoofed one can only route a sender's later
//! datagrams to a transport that authenticates and rejects them. The damage
//! is bounded by [`SocketDemux::max_tuples_per_session`]: a session maps at
//! most that many addresses, further ones are dropped, never evicting a
//! working mapping.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::net::SocketAddr;

use rtc_stun::attributes::ATTR_USERNAME;
use rtc_stun::message::Message;
use rtc_stun::textattrs::TextAttribute;

use super::transport::DEMUX_STUN_MAX;

/// RFC 8445 §7.2.2: the separator between the receiver's and the sender's
/// ufrag in a connectivity check's `USERNAME`.
const USERNAME_SEPARATOR: char = ':';

/// Routes inbound datagrams of one shared UDP socket to the session that owns
/// them. `K` names a session (a connection id, say).
#[derive(Debug)]
pub struct SocketDemux<K> {
    by_ufrag: HashMap<String, K>,
    by_tuple: HashMap<SocketAddr, K>,
    tuples_of: HashMap<K, HashSet<SocketAddr>>,
    max_tuples_per_session: usize,
}

impl<K: Clone + Eq + Hash> SocketDemux<K> {
    /// A demux that lets each session map at most `max_tuples_per_session`
    /// source addresses.
    pub fn new(max_tuples_per_session: usize) -> Self {
        Self {
            by_ufrag: HashMap::new(),
            by_tuple: HashMap::new(),
            tuples_of: HashMap::new(),
            max_tuples_per_session,
        }
    }

    /// The most source addresses one session may map.
    pub fn max_tuples_per_session(&self) -> usize {
        self.max_tuples_per_session
    }

    /// Make `ufrag` (a session's local `ice-ufrag`, including one an ICE
    /// restart introduces) route to `session`. `false` when another session
    /// already owns that ufrag, which is left unchanged.
    pub fn register(&mut self, ufrag: &str, session: K) -> bool {
        if self
            .by_ufrag
            .get(ufrag)
            .is_some_and(|owner| *owner != session)
        {
            return false;
        }
        self.by_ufrag.insert(ufrag.to_string(), session.clone());
        self.tuples_of.entry(session).or_default();
        true
    }

    /// Forget `session`: its ufrags and every address it mapped.
    pub fn unregister(&mut self, session: &K) {
        self.by_ufrag.retain(|_, owner| owner != session);
        if let Some(tuples) = self.tuples_of.remove(session) {
            for tuple in tuples {
                self.by_tuple.remove(&tuple);
            }
        }
    }

    /// How many sessions are registered.
    pub fn sessions(&self) -> usize {
        self.tuples_of.len()
    }

    /// How many addresses `session` has mapped.
    pub fn mapped_tuples(&self, session: &K) -> usize {
        self.tuples_of.get(session).map_or(0, HashSet::len)
    }

    /// The session the datagram `data` from `from` belongs to, or `None` to
    /// drop it.
    ///
    /// - A STUN message whose `USERNAME` names a registered ufrag belongs to
    ///   that session, and maps `from` to it (unless the session is already at
    ///   its cap, which drops the message). An address previously mapped to
    ///   another session moves to this one.
    /// - Any other datagram (DTLS, SRTP/SRTCP, a STUN response) belongs to the
    ///   session `from` is mapped to, and is dropped when `from` is unmapped.
    pub fn classify(&mut self, data: &[u8], from: SocketAddr) -> Option<K> {
        let first = *data.first()?;
        if first <= DEMUX_STUN_MAX
            && let Some(ufrag) = local_ufrag(data)
        {
            let session = self.by_ufrag.get(&ufrag)?.clone();
            return self.map_tuple(from, session);
        }
        self.by_tuple.get(&from).cloned()
    }

    fn map_tuple(&mut self, from: SocketAddr, session: K) -> Option<K> {
        match self.by_tuple.get(&from) {
            Some(owner) if *owner == session => return Some(session),
            Some(owner) => {
                let previous = owner.clone();
                if let Some(tuples) = self.tuples_of.get_mut(&previous) {
                    tuples.remove(&from);
                }
            }
            None => {}
        }
        let tuples = self.tuples_of.entry(session.clone()).or_default();
        if tuples.len() >= self.max_tuples_per_session {
            self.by_tuple.remove(&from);
            return None;
        }
        tuples.insert(from);
        self.by_tuple.insert(from, session.clone());
        Some(session)
    }
}

/// The receiver ufrag a STUN message's `USERNAME` carries, or `None` when the
/// datagram is not STUN or has no `USERNAME`.
fn local_ufrag(data: &[u8]) -> Option<String> {
    let mut message = Message::new();
    message.unmarshal_binary(data).ok()?;
    let username = TextAttribute::get_from_as(&message, ATTR_USERNAME).ok()?;
    let (local, _) = username.text.split_once(USERNAME_SEPARATOR)?;
    Some(local.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc_stun::fingerprint::FINGERPRINT;
    use rtc_stun::message::{BINDING_REQUEST, TransactionId};

    const CAP: usize = 3;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn check(username: &str) -> Vec<u8> {
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_string())),
                Box::new(FINGERPRINT),
            ])
            .expect("build a binding request");
        message.raw
    }

    const DTLS_RECORD: [u8; 4] = [22, 254, 253, 0];
    const SRTP_PACKET: [u8; 4] = [0x80, 96, 0, 1];

    fn demux() -> SocketDemux<u32> {
        let mut demux = SocketDemux::new(CAP);
        assert!(demux.register("aaaaaaaa", 1));
        assert!(demux.register("bbbbbbbb", 2));
        demux
    }

    /// A check naming a registered ufrag reaches that session, and from then
    /// on so does everything else from the same address.
    #[test]
    fn a_check_maps_its_source_to_the_named_session() {
        let mut demux = demux();
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), None);
        assert_eq!(demux.classify(&check("bbbbbbbb:peer"), addr(1)), Some(2));
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), Some(2));
        assert_eq!(demux.classify(&SRTP_PACKET, addr(1)), Some(2));
        assert_eq!(demux.classify(&SRTP_PACKET, addr(2)), None);
    }

    /// A check naming an unknown ufrag maps nothing.
    #[test]
    fn a_spoofed_check_with_an_unknown_ufrag_maps_nothing() {
        let mut demux = demux();
        assert_eq!(demux.classify(&check("zzzzzzzz:peer"), addr(9)), None);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(9)), None);
        assert_eq!(demux.mapped_tuples(&1) + demux.mapped_tuples(&2), 0);
    }

    /// A session maps at most its cap of addresses; further ones are dropped
    /// and the mapped ones keep working.
    #[test]
    fn the_per_session_tuple_cap_holds() {
        let mut demux = demux();
        for port in 1..=CAP as u16 {
            assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(port)), Some(1));
        }
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(100)), None);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(100)), None);
        assert_eq!(demux.mapped_tuples(&1), CAP);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), Some(1));
        assert_eq!(demux.classify(&check("bbbbbbbb:p"), addr(100)), Some(2));
    }

    /// Unregistering a session frees its ufrag and its addresses.
    #[test]
    fn unregister_forgets_the_session() {
        let mut demux = demux();
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(1)), Some(1));
        demux.unregister(&1);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), None);
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(1)), None);
        assert_eq!(demux.sessions(), 1);
        assert!(demux.register("aaaaaaaa", 3));
    }

    /// A ufrag belongs to one session; an address moves to the session whose
    /// check arrives from it.
    #[test]
    fn a_ufrag_has_one_owner_and_an_address_follows_the_latest_check() {
        let mut demux = demux();
        assert!(!demux.register("aaaaaaaa", 2));
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(1)), Some(1));
        assert_eq!(demux.classify(&check("bbbbbbbb:p"), addr(1)), Some(2));
        assert_eq!(demux.mapped_tuples(&1), 0);
        assert_eq!(demux.mapped_tuples(&2), 1);
    }

    /// A STUN message with no `USERNAME` (a response) follows the address map.
    #[test]
    fn a_stun_message_without_a_username_follows_the_address() {
        let mut demux = demux();
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(1)), Some(1));
        let mut response = Message::new();
        response
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(FINGERPRINT),
            ])
            .expect("build");
        assert_eq!(demux.classify(&response.raw, addr(1)), Some(1));
        assert_eq!(demux.classify(&response.raw, addr(2)), None);
    }
}
