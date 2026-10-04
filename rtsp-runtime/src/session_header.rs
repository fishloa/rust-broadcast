//! Typed `Session` header — RFC 2326 §12.37 (`docs/rfc2326.md`, `docs/session-header.md`).
//!
//! ```text
//! Session = "Session" ":" session-id [ ";" "timeout" "=" delta-seconds ]
//! ```
//!
//! `session-id` (§3.4) is `1*( ALPHA | DIGIT | safe )` with
//! `safe = "$" | "-" | "_" | "." | "+"`; identifiers are opaque and a parser must not
//! reject short ones. `delta-seconds` is `1*DIGIT`; the `timeout` defaults to 60 s
//! when absent.
//!
//! Lenient on input, canonical on output (the interop table is in
//! `docs/session-header.md`): LWS around `;` and `=`, case-insensitive parameter
//! names, a bad or zero `timeout` value keeps the id and falls back to the default (with a
//! warning, see [`SessionHeader::parse_with_warnings`]), unknown parameters are
//! preserved. The session id is everything before the first `;` outside a
//! quoted-string, LWS-trimmed; when the quotes do not balance (an unterminated
//! quote) the id is instead everything before the first `;`, so `ab"c;timeout=30`
//! keeps id `ab"c` and timeout 30 rather than swallowing the `;`. On RECEIVE any
//! non-empty id without a control
//! character is accepted verbatim (`"weird"`, `a b`, `abc,def`, `ab"c`: real servers
//! send them and the client echoes them back unchanged). Only what we EMIT is strict
//! (non-empty, trimmed, no control characters, representable). Output is always
//! `id[;timeout=N][;ext[=value]]…` with no whitespace.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::rfc2326_lex as lex;

/// §12.37: the session timeout when none is declared.
pub const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(60);

/// The `timeout` parameter name (§12.37).
const PARAM_TIMEOUT: &str = "timeout";
/// `delta-seconds = 1*DIGIT` — bounded to what a `u64` holds (19 digits).
const DELTA_SECONDS_DIGITS: usize = 19;

/// A parsed `Session` header (RFC 2326 §12.37).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SessionHeader {
    /// The session identifier (opaque).
    pub id: String,
    /// The declared `timeout`, if any (a malformed value is [`DEFAULT_SESSION_TIMEOUT`]).
    pub timeout: Option<Duration>,
    /// Unknown parameters `(name, value)` in input order.
    pub extensions: Vec<(String, Option<String>)>,
}

impl SessionHeader {
    /// A header carrying only `id`.
    pub fn new(id: impl Into<String>) -> Self {
        SessionHeader {
            id: id.into(),
            timeout: None,
            extensions: Vec::new(),
        }
    }

    /// Parses a `Session` field value; see the module docs for the leniency rules.
    pub fn parse(value: &str) -> Result<Self> {
        Self::parse_with_warnings(value).map(|(h, _)| h)
    }

    /// Like [`parse`](Self::parse), also returning the recoverable problems found
    /// (e.g. a malformed `timeout` that fell back to the default).
    pub fn parse_with_warnings(value: &str) -> Result<(Self, Vec<String>)> {
        // The lexer owns the split: the id is everything before the first `;`
        // outside a quoted-string, falling back to the first `;` when the quotes
        // do not balance, so `ab"c;timeout=30` keeps id `ab"c` and timeout 30.
        let (id, segs) = lex::split_session_value(value);
        if id.is_empty() {
            return Err(Error::SessionParse("empty session id".into()));
        }
        // Receive side is lenient (interop): any non-empty id without a control
        // character is kept verbatim, whatever else it contains.
        if lex::has_ctl(id) {
            return Err(Error::SessionParse(format!(
                "control character in session id {id:?}"
            )));
        }
        let mut out = SessionHeader::new(id);
        let mut warnings = Vec::new();
        for seg in segs {
            if lex::trim_lws(seg).is_empty() {
                continue;
            }
            let (name, raw) = lex::split_param(seg).map_err(|e| Error::SessionParse(e.0))?;
            let value = match raw {
                Some(v) => Some(lex::unquote(v).map_err(|e| Error::SessionParse(e.0))?.0),
                None => None,
            };
            if name.eq_ignore_ascii_case(PARAM_TIMEOUT) {
                let parsed = value
                    .as_deref()
                    .and_then(|v| lex::digits(v, DELTA_SECONDS_DIGITS, PARAM_TIMEOUT).ok());
                match parsed {
                    // 0 is invalid too: it would make the keepalive deadline "now".
                    Some(n) if n > 0 => out.timeout = Some(Duration::from_secs(n.into())),
                    _ => {
                        warnings.push(format!(
                            "malformed or zero timeout {value:?}; using the 60 s default"
                        ));
                        out.timeout = Some(DEFAULT_SESSION_TIMEOUT);
                    }
                }
            } else {
                out.extensions.push((name.to_string(), value));
            }
        }
        Ok((out, warnings))
    }

    /// Canonical serialization: `id[;timeout=N][;ext[=value]]…`.
    ///
    /// # Errors
    ///
    /// [`Error::HeaderSerialize`] if the id is not a valid session-id, an extension
    /// name is not an RFC 2326 §15.1 token, or a value holds a control character;
    /// nothing unsafe is ever written.
    pub fn to_header_value(&self) -> Result<String> {
        // Strict only for what WE emit: non-empty, trimmed, no control characters.
        if self.id.is_empty() || lex::has_ctl(&self.id) || lex::trim_lws(&self.id) != self.id {
            return Err(Error::HeaderSerialize(format!(
                "invalid session id {:?}",
                self.id
            )));
        }
        let mut s = self.id.clone();
        if let Some(t) = self.timeout {
            s.push(lex::PARAM_SEP);
            s.push_str(&format!("{PARAM_TIMEOUT}{}{}", lex::VALUE_SEP, t.as_secs()));
        }
        for (name, value) in &self.extensions {
            if !lex::is_token(name) {
                return Err(Error::HeaderSerialize(format!(
                    "extension name {name:?} is not a token"
                )));
            }
            s.push(lex::PARAM_SEP);
            s.push_str(name);
            if let Some(v) = value {
                s.push(lex::VALUE_SEP);
                s.push_str(&lex::emit_value(v).map_err(|e| Error::HeaderSerialize(e.0))?);
            }
        }
        // The id must come back as the first `;`-segment (an unbalanced quote in it
        // would swallow the separator): otherwise it cannot be emitted faithfully.
        let first = lex::split_outside_quotes(&s, lex::PARAM_SEP)[0];
        if lex::trim_lws(first) != self.id {
            return Err(Error::HeaderSerialize(format!(
                "session id {:?} cannot be emitted unambiguously",
                self.id
            )));
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &str) -> SessionHeader {
        SessionHeader::parse(v).unwrap()
    }

    #[test]
    fn the_interop_table() {
        for v in [
            "abc;timeout=30",
            "abc; timeout=30",
            "abc ; timeout=30",
            "abc;TIMEOUT=30",
            "abc;timeout = 30",
        ] {
            let h = p(v);
            assert_eq!(
                (h.id.as_str(), h.timeout),
                ("abc", Some(Duration::from_secs(30))),
                "{v}"
            );
        }
        let (h, w) = SessionHeader::parse_with_warnings("abc;timeout=0").unwrap();
        assert_eq!(
            h.timeout,
            Some(DEFAULT_SESSION_TIMEOUT),
            "timeout=0 is invalid"
        );
        assert_eq!(w.len(), 1);
        let (h, w) = SessionHeader::parse_with_warnings("abc;timeout=x").unwrap();
        assert_eq!(h.timeout, Some(DEFAULT_SESSION_TIMEOUT));
        assert_eq!(w.len(), 1);
        let h = p("abc;foo=bar");
        assert_eq!(h.extensions, vec![("foo".into(), Some("bar".into()))]);
        assert!(SessionHeader::parse("").is_err());
        assert!(SessionHeader::parse("  ;timeout=5").is_err());
        assert_eq!(p("12345678").timeout, None);
    }

    #[test]
    fn an_unbalanced_quote_falls_back_to_the_first_semicolon() {
        // An unterminated quote must not swallow the `;` that introduces the params.
        let (h, w) = SessionHeader::parse_with_warnings("ab\"c;timeout=30").unwrap();
        assert_eq!(h.id, "ab\"c");
        assert_eq!(h.timeout, Some(Duration::from_secs(30)));
        assert!(w.is_empty(), "{w:?}");
        // ... but a balanced quoted id still splits quote-aware.
        let (h, w) = SessionHeader::parse_with_warnings("\"a;b\";timeout=30").unwrap();
        assert_eq!(h.id, "\"a;b\"");
        assert_eq!(h.timeout, Some(Duration::from_secs(30)));
        assert!(w.is_empty(), "{w:?}");
        // No parameters: the unbalanced-quote id is kept whole.
        assert_eq!(p("ab\"c").id, "ab\"c");
    }

    #[test]
    fn canonical_output_round_trips_and_is_idempotent() {
        for v in [
            "abc",
            "abc ; Timeout = 30 ; x=\"a b\"",
            "abc;timeout=x;flag",
        ] {
            let a = p(v);
            let c = a.to_header_value().unwrap();
            let b = p(&c);
            assert_eq!(a.id, b.id);
            assert_eq!(b.to_header_value().unwrap(), c, "idempotent {v}");
            assert_eq!(p(&c), b);
        }
        assert_eq!(
            p("abc ; Timeout = 30").to_header_value().unwrap(),
            "abc;timeout=30"
        );
    }

    #[test]
    fn the_serializer_never_emits_control_characters() {
        let mut h = SessionHeader::new("abc");
        h.id = "ab\r\nSet-Cookie: x".into();
        assert!(matches!(
            h.to_header_value(),
            Err(Error::HeaderSerialize(_))
        ));
        let mut h = SessionHeader::new("abc");
        h.extensions.push(("x\r\ny".into(), None));
        assert!(h.to_header_value().is_err());
        let mut h = SessionHeader::new("abc");
        h.extensions.push(("x".into(), Some("a\r\nb".into())));
        assert!(h.to_header_value().is_err());
        let mut h = SessionHeader::new("abc");
        h.id = String::new();
        assert!(h.to_header_value().is_err());
        // an id that would swallow the `;` separator cannot be emitted faithfully
        let mut h = SessionHeader::new("ab\"c");
        h.timeout = Some(Duration::from_secs(5));
        assert!(h.to_header_value().is_err());
    }
    /// Fuzz regression (crash input `";;!` from the rtsp_headers target): a received id of a
    /// lone `"` parses (receive is lenient) but cannot be emitted unambiguously (emit is strict).
    #[test]
    fn a_received_lone_quote_id_parses_but_is_not_emittable() {
        let (h, _) = SessionHeader::parse_with_warnings("\";;!").unwrap();
        assert_eq!(h.id, "\"");
        assert!(matches!(
            h.to_header_value(),
            Err(crate::Error::HeaderSerialize(_))
        ));
    }
}
