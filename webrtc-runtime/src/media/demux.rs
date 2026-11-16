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
//! each session's local ufrag and ice-pwd, asks [`SocketDemux::classify`]
//! where each inbound datagram goes, and feeds it to that session's transport.
//!
//! # What is and is not trusted
//!
//! A check naming a known ufrag maps its source address to that session only
//! when its `MESSAGE-INTEGRITY` verifies under the ice-pwd signalled with that
//! ufrag (RFC 8445 §6.1.3), so a sender who knows the session's ufrag (not
//! secret) but not its password cannot map an address: unmatched checks are
//! dropped before they can occupy — or steal — a mapping. The budget is
//! further bounded by [`SocketDemux::max_tuples_per_session`]: a session maps
//! at most that many addresses, further ones are dropped, never evicting a
//! working mapping.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::Arc;

use rtc_stun::attributes::ATTR_USERNAME;
use rtc_stun::crypto::{RTCCryptoProvider, default_provider};
use rtc_stun::integrity::MessageIntegrity;
use rtc_stun::message::Message;
use rtc_stun::textattrs::TextAttribute;

use super::transport::DEMUX_STUN_MAX;

/// RFC 8445 §7.2.2: the separator between the receiver's and the sender's
/// ufrag in a connectivity check's `USERNAME`.
const USERNAME_SEPARATOR: char = ':';

/// Routes inbound datagrams of one shared UDP socket to the session that owns
/// them. `K` names a session (a connection id, say).
///
/// `Debug` names the registered ufrags but never a password.
pub struct SocketDemux<K> {
    /// Each registered local ufrag with its owning session and the ice-pwd
    /// that authenticates a check naming it.
    by_ufrag: HashMap<String, (K, String)>,
    by_tuple: HashMap<SocketAddr, K>,
    tuples_of: HashMap<K, HashSet<SocketAddr>>,
    max_tuples_per_session: usize,
    /// The provider `MESSAGE-INTEGRITY` verification runs on. `None` when the
    /// build has no built-in provider, in which case no check is trusted.
    crypto: Option<Arc<dyn RTCCryptoProvider>>,
}

impl<K: fmt::Debug> fmt::Debug for SocketDemux<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SocketDemux")
            .field("ufrags", &self.by_ufrag.keys().collect::<Vec<_>>())
            .field("by_tuple", &self.by_tuple)
            .field("tuples_of", &self.tuples_of)
            .field("max_tuples_per_session", &self.max_tuples_per_session)
            .finish_non_exhaustive()
    }
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
            crypto: default_provider().ok(),
        }
    }

    /// The most source addresses one session may map.
    pub fn max_tuples_per_session(&self) -> usize {
        self.max_tuples_per_session
    }

    /// Make `ufrag` (a session's local `ice-ufrag`, including one an ICE
    /// restart introduces) route to `session`, admitting a check from a new
    /// source address only when `pwd` — the local `ice-pwd` signalled with
    /// that ufrag — keys its `MESSAGE-INTEGRITY`. `false` when another session
    /// already owns that ufrag, which is left unchanged.
    pub fn register(&mut self, ufrag: &str, pwd: &str, session: K) -> bool {
        if self
            .by_ufrag
            .get(ufrag)
            .is_some_and(|(owner, _)| *owner != session)
        {
            return false;
        }
        self.by_ufrag
            .insert(ufrag.to_string(), (session.clone(), pwd.to_string()));
        self.tuples_of.entry(session).or_default();
        true
    }

    /// Forget `session`: its ufrags and every address it mapped.
    pub fn unregister(&mut self, session: &K) {
        self.by_ufrag.retain(|_, (owner, _)| owner != session);
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
    /// - A STUN message whose `USERNAME` names a registered ufrag and whose
    ///   `MESSAGE-INTEGRITY` verifies under that ufrag's password belongs to
    ///   that session, and maps `from` to it (unless the session is already at
    ///   its cap, which drops the message). An address previously mapped to
    ///   another session moves to this one. An unauthenticated check is
    ///   dropped.
    /// - Any other datagram (DTLS, SRTP/SRTCP, a STUN response) belongs to the
    ///   session `from` is mapped to, and is dropped when `from` is unmapped.
    pub fn classify(&mut self, data: &[u8], from: SocketAddr) -> Option<K> {
        let first = *data.first()?;
        if first <= DEMUX_STUN_MAX
            && let Some(ufrag) = local_ufrag(data)
        {
            let (session, pwd) = self.by_ufrag.get(&ufrag)?;
            if !check_is_authenticated(data, pwd, self.crypto.as_deref()) {
                return None;
            }
            let session = session.clone();
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

/// Whether `data` carries a `MESSAGE-INTEGRITY` keyed with `pwd` (RFC 8445
/// §6.1.3), the check's only proof it comes from the session whose ufrag it
/// names. A message without one, or a build without a crypto provider, is not
/// authentic.
fn check_is_authenticated(data: &[u8], pwd: &str, crypto: Option<&dyn RTCCryptoProvider>) -> bool {
    let Some(crypto) = crypto else {
        return false;
    };
    let mut message = Message::new();
    if message.unmarshal_binary(data).is_err() {
        return false;
    }
    MessageIntegrity::check(&mut message, pwd.as_bytes(), crypto.crypto()).is_ok()
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

    const PWD_A: &str = "aaaaaaaa-ice-password-0000";
    const PWD_B: &str = "bbbbbbbb-ice-password-0000";

    fn authenticated_check(username: &str, pwd: &str) -> Vec<u8> {
        use rtc_stun::integrity::MessageIntegrity;

        let crypto = rtc_stun::crypto::default_provider().expect("default crypto provider");
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_string())),
                Box::new(MessageIntegrity::new_short_term_integrity_with_provider(
                    pwd.to_string(),
                    crypto.crypto(),
                )),
                Box::new(FINGERPRINT),
            ])
            .expect("build an authenticated binding request");
        message.raw
    }

    const DTLS_RECORD: [u8; 4] = [22, 254, 253, 0];
    const SRTP_PACKET: [u8; 4] = [0x80, 96, 0, 1];

    fn demux() -> SocketDemux<u32> {
        let mut demux = SocketDemux::new(CAP);
        assert!(demux.register("aaaaaaaa", PWD_A, 1));
        assert!(demux.register("bbbbbbbb", PWD_B, 2));
        demux
    }

    /// A check naming a registered ufrag reaches that session, and from then
    /// on so does everything else from the same address.
    #[test]
    fn a_check_maps_its_source_to_the_named_session() {
        let mut demux = demux();
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), None);
        assert_eq!(
            demux.classify(&authenticated_check("bbbbbbbb:peer", PWD_B), addr(1)),
            Some(2)
        );
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), Some(2));
        assert_eq!(demux.classify(&SRTP_PACKET, addr(1)), Some(2));
        assert_eq!(demux.classify(&SRTP_PACKET, addr(2)), None);
    }

    /// A check naming an unknown ufrag maps nothing.
    #[test]
    fn a_spoofed_check_with_an_unknown_ufrag_maps_nothing() {
        let mut demux = demux();
        assert_eq!(demux.classify(&check("zzzzzzzz:peer"), addr(9)), None);
        assert_eq!(
            demux.classify(&authenticated_check("zzzzzzzz:peer", PWD_A), addr(9)),
            None
        );
        assert_eq!(demux.classify(&DTLS_RECORD, addr(9)), None);
        assert_eq!(demux.mapped_tuples(&1) + demux.mapped_tuples(&2), 0);
    }

    /// A session maps at most its cap of addresses; further ones are dropped
    /// and the mapped ones keep working.
    #[test]
    fn the_per_session_tuple_cap_holds() {
        let mut demux = demux();
        for port in 1..=CAP as u16 {
            assert_eq!(
                demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(port)),
                Some(1)
            );
        }
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(100)),
            None
        );
        assert_eq!(demux.classify(&DTLS_RECORD, addr(100)), None);
        assert_eq!(demux.mapped_tuples(&1), CAP);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), Some(1));
        assert_eq!(
            demux.classify(&authenticated_check("bbbbbbbb:p", PWD_B), addr(100)),
            Some(2)
        );
    }

    /// Unregistering a session frees its ufrag and its addresses.
    #[test]
    fn unregister_forgets_the_session() {
        let mut demux = demux();
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(1)),
            Some(1)
        );
        demux.unregister(&1);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(1)), None);
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(1)),
            None
        );
        assert_eq!(demux.sessions(), 1);
        assert!(demux.register("aaaaaaaa", PWD_A, 3));
    }

    /// A ufrag belongs to one session; an address moves to the session whose
    /// check arrives from it.
    #[test]
    fn a_ufrag_has_one_owner_and_an_address_follows_the_latest_check() {
        let mut demux = demux();
        assert!(!demux.register("aaaaaaaa", PWD_B, 2));
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(1)),
            Some(1)
        );
        assert_eq!(
            demux.classify(&authenticated_check("bbbbbbbb:p", PWD_B), addr(1)),
            Some(2)
        );
        assert_eq!(demux.mapped_tuples(&1), 0);
        assert_eq!(demux.mapped_tuples(&2), 1);
    }

    /// A STUN message with no `USERNAME` (a response) follows the address map.
    #[test]
    fn a_stun_message_without_a_username_follows_the_address() {
        let mut demux = demux();
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(1)),
            Some(1)
        );
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

    /// Audit 20 finding 40: a check only maps (or steals) a source address
    /// once its `MESSAGE-INTEGRITY` proves it came from the session it names,
    /// so unauthenticated checks cannot fill the tuple budget before the real
    /// peer's address arrives.
    #[test]
    fn an_unauthenticated_check_does_not_map_or_consume_the_cap() {
        let mut demux = demux();
        for port in 1..=CAP as u16 {
            assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(port)), None);
        }
        assert_eq!(demux.mapped_tuples(&1), 0);
        assert_eq!(
            demux.classify(&authenticated_check("aaaaaaaa:p", PWD_A), addr(100)),
            Some(1)
        );

        assert_eq!(
            demux.classify(&authenticated_check("bbbbbbbb:p", PWD_B), addr(201)),
            Some(2)
        );
        assert_eq!(demux.classify(&check("aaaaaaaa:p"), addr(201)), None);
        assert_eq!(demux.classify(&DTLS_RECORD, addr(201)), Some(2));
        assert_eq!(demux.mapped_tuples(&2), 1);
    }
}
