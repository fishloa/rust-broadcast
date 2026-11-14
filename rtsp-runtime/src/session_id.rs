//! The typed `Session` id a [`ServerSession`](crate::ServerSession) allocates
//! and a client hands back (RFC 2326 §3.4, §12.37).

use core::fmt;

use crate::error::{Error, Result};

/// One RTSP `Session` id: 64 bits of the server's random source, rendered on
/// the wire as 16 lower-case hex digits (RFC 2326 §3.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(u64);

impl SessionId {
    /// How many hex digits [`SessionId`] renders.
    pub const HEX_DIGITS: usize = 16;

    /// The id with value `raw`.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        SessionId(raw)
    }

    /// The id's 64-bit value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Parses a `Session` header's id token: hex digits, at most
    /// [`SessionId::HEX_DIGITS`] of them. Case is not significant — a peer
    /// spelling its id in upper case is still the same session — and a
    /// shorter token is a counter-style id, not a malformed one.
    ///
    /// # Errors
    ///
    /// [`Error::SessionIdParse`] for a token that is empty or carries more
    /// than 16 hex digits.
    pub fn parse(text: &str) -> Result<Self> {
        if text.is_empty() || text.len() > Self::HEX_DIGITS {
            return Err(Error::SessionIdParse {
                text: text.to_string(),
            });
        }
        u64::from_str_radix(text, 16)
            .map(SessionId)
            .map_err(|_| Error::SessionIdParse {
                text: text.to_string(),
            })
    }
}

impl fmt::Display for SessionId {
    /// The wire spelling: 16 zero-padded lower-case hex digits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({:016x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_builds_a_session_whose_ids_are_the_typed_values_it_draws() {
        let mut s = crate::ServerSession::new(
            crate::RtspServerConfig::default()
                .with_session_ids(crate::SessionIdSource::counter(0xABCD)),
        )
        .expect("a fresh session");
        assert_eq!(s.session_id(), None, "no id exists before SETUP");
        let (resp, events) = s
            .handle_request(
                b"SETUP rtsp://h/s/track1 RTSP/1.0\r\nCSeq: 1\r\n\
                  Transport: RTP/AVP/TCP;interleaved=0-1\r\n\r\n",
            )
            .expect("SETUP is answered");
        let id = s.session_id().expect("SETUP allocated an id");
        assert_eq!(id, SessionId::new(0xABCD));
        assert!(
            String::from_utf8_lossy(&resp).contains("Session: 000000000000abcd"),
            "{}",
            String::from_utf8_lossy(&resp)
        );
        assert!(events.iter().any(|e| matches!(
            e,
            crate::ServerEvent::SessionSetup { session_id, .. }
                if session_id == "000000000000abcd"
        )));
    }

    #[test]
    fn the_wire_spelling_is_sixteen_lower_case_hex_digits() {
        assert_eq!(
            SessionId::new(0x0123_4567_89ab_cdef).to_string(),
            "0123456789abcdef"
        );
        assert_eq!(SessionId::new(1).to_string(), "0000000000000001");
        assert_eq!(SessionId::new(u64::MAX).to_string(), "ffffffffffffffff");
    }

    #[test]
    fn parsing_round_trips_the_wire_spelling() {
        for raw in [0, 1, 0x0123_4567_89ab_cdef, u64::MAX] {
            let id = SessionId::new(raw);
            assert_eq!(SessionId::parse(&id.to_string()).expect("own spelling"), id);
        }
        assert_eq!(
            SessionId::parse("ABCDEF0123456789").expect("hex"),
            SessionId::new(0xabcdef0123456789)
        );
    }

    #[test]
    fn a_malformed_id_is_a_typed_error_not_a_zero() {
        for text in ["", "0123456789abcdef0", "0123456789abcdeg"] {
            assert!(
                matches!(SessionId::parse(text), Err(Error::SessionIdParse { .. })),
                "{text:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_peers_upper_case_or_short_id_is_the_same_session_not_a_dropped_one() {
        assert_eq!(
            SessionId::parse("ABCDEF0123456789").expect("upper case"),
            SessionId::new(0xabcdef0123456789)
        );
        assert_eq!(
            SessionId::parse("12345678").expect("short"),
            SessionId::new(0x12345678)
        );
    }
}
