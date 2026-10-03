//! Typed `Transport` header — RFC 2326 §12.39 (`docs/rfc2326.md`, `docs/transport-header.md`).
//!
//! A full, spec-grounded parser and canonical serializer (owner decision (c): not
//! `rtsp-types`, whose Transport parser is exact-case and drops parameters). One
//! header value is `1#transport-spec`, comma-separated in order of preference;
//! each spec is `transport/profile[/lower-transport] *( ";" parameter )`:
//!
//! ```text
//! transport-spec = transport-protocol/profile[/lower-transport] *parameter
//! parameter      = ( "unicast" | "multicast" ) | ";" "destination" [ "=" address ]
//!                | ";" "interleaved" "=" channel [ "-" channel ] | ";" "append"
//!                | ";" "ttl" "=" ttl | ";" "layers" "=" 1*DIGIT
//!                | ";" "port" "=" port [ "-" port ] | ";" "client_port" "=" port [ "-" port ]
//!                | ";" "server_port" "=" port [ "-" port ] | ";" "ssrc" "=" ssrc
//!                | ";" "mode" = <"> 1#mode <">
//! ttl = 1*3(DIGIT)   port = 1*5(DIGIT)   ssrc = 8*8(HEX)   channel = 1*3(DIGIT)
//! ```
//!
//! # Input leniency (interop, see `docs/transport-header.md`)
//!
//! Lexing is RFC 2326 §15.1 (see `crate::rfc2326_lex`): transport, profile,
//! lower-transport tokens and parameter names are case-insensitive; implied LWS is
//! allowed around every separator; any value may be a quoted-string (a quoted
//! `ssrc` or `port` is accepted); `mode` is accepted unquoted as well as quoted
//! (the grammar requires the quotes; RFC 2326 §14.6's own example writes
//! `mode=record`); a bare `destination` / `source` is kept as an empty address;
//! a range with one end (`interleaved=6`, `client_port=10000`) means `lo = hi`.
//! Range checks are errors: port <= 65535, ttl <= 255, channel <= 255, ssrc exactly
//! 8 hex digits. Only `RTP/AVP` with `TCP` or `UDP` is modelled; other
//! protocols/profiles are an error. Unknown parameters are preserved in order in
//! [`TransportSpec::extensions`]. A repeated known parameter: the last wins.
//!
//! # Canonical output
//!
//! [`Transport::to_header_value`] is canonical and symmetric: parse -> serialize ->
//! parse is equal, serialize -> parse is equal, and the canonical form is
//! idempotent. Parameters are written in the spec's listing order
//! (`unicast|multicast`, `destination`, `source`, `interleaved`, `append`, `ttl`,
//! `layers`, `port`, `client_port`, `server_port`, `ssrc`, `mode`) followed by the
//! extensions in input order; ranges are always `lo-hi`, `ssrc` is 8 upper-case
//! hex digits, `mode` is the quoted list.

use crate::error::{Error, Result};
use crate::rfc2326_lex as lex;

/// Transport protocol token (§12.39 `transport-protocol`).
const PROTO_RTP: &str = "RTP";
/// Profile token (§12.39 `profile`).
const PROFILE_AVP: &str = "AVP";
/// Lower-transport tokens (§12.39 `lower-transport`).
const LOWER_TCP: &str = "TCP";
const LOWER_UDP: &str = "UDP";
/// `ttl = 1*3(DIGIT)`, max 255.
const TTL_DIGITS: usize = 3;
/// `channel = 1*3(DIGIT)`, max 255.
const CHANNEL_DIGITS: usize = 3;
/// `port = 1*5(DIGIT)`, max 65535.
const PORT_DIGITS: usize = 5;
/// `ssrc = 8*8(HEX)`.
const SSRC_HEX_DIGITS: usize = 8;
/// `layers = 1*DIGIT` — bounded to what a `u32` holds (10 digits).
const LAYERS_DIGITS: usize = 10;
/// Method tokens inside `mode` (§12.39).
const MODE_PLAY: &str = "PLAY";
const MODE_RECORD: &str = "RECORD";
/// Separator of a `mode` list inside its quoted-string.
const MODE_SEP: char = ',';

/// Lower-layer transport for an RTP/AVP spec (RFC 2326 §12.39).
///
/// For `RTP/AVP`, the default lower-transport is UDP when omitted.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LowerTransport {
    /// `UDP` — the default when no lower-transport token is present.
    Udp,
    /// `TCP` — used for interleaved (`$`-framed) delivery.
    Tcp,
}

impl LowerTransport {
    /// The RFC 2326 token for this lower transport.
    pub fn name(&self) -> &'static str {
        match self {
            LowerTransport::Udp => "UDP",
            LowerTransport::Tcp => "TCP",
        }
    }
}

impl core::fmt::Display for LowerTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Delivery mode: unicast or multicast (RFC 2326 §12.39). Mutually exclusive.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Delivery {
    /// `unicast` delivery.
    Unicast,
    /// `multicast` delivery (the RFC default when neither token is present).
    Multicast,
}

impl Delivery {
    /// The RFC 2326 token for this delivery mode.
    pub fn name(&self) -> &'static str {
        match self {
            Delivery::Unicast => "unicast",
            Delivery::Multicast => "multicast",
        }
    }
}

impl core::fmt::Display for Delivery {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// One `mode` method (§12.39 `mode = <"> *Method <"> | Method`): `PLAY` or `RECORD`
/// (matched case-insensitively), any other Method token kept as written.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransportMode {
    /// `PLAY` (the default when `mode` is absent).
    Play,
    /// `RECORD`.
    Record,
    /// Any other Method token, as written.
    Other(String),
}

impl TransportMode {
    /// The RFC 2326 token for this mode.
    pub fn name(&self) -> &str {
        match self {
            TransportMode::Play => MODE_PLAY,
            TransportMode::Record => MODE_RECORD,
            TransportMode::Other(m) => m,
        }
    }

    fn from_token(t: &str) -> Self {
        if t.eq_ignore_ascii_case(MODE_PLAY) {
            TransportMode::Play
        } else if t.eq_ignore_ascii_case(MODE_RECORD) {
            TransportMode::Record
        } else {
            TransportMode::Other(t.to_string())
        }
    }
}

impl core::fmt::Display for TransportMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// A single parsed transport-spec from a `Transport` header (RFC 2326 §12.39).
///
/// Only `RTP/AVP` (with optional `/TCP` or `/UDP`) is modelled. Every §12.39
/// parameter is a typed field; unknown parameters are kept in
/// [`extensions`](Self::extensions).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransportSpec {
    /// Lower-layer transport. `None` means the token was absent -> UDP default.
    pub lower_transport: Option<LowerTransport>,
    /// `unicast` / `multicast`.
    pub delivery: Option<Delivery>,
    /// `interleaved=lo[-hi]` — the `$`-framing channel pair (§10.12); one end means `hi = lo`.
    pub interleaved: Option<(u8, u8)>,
    /// `client_port=lo[-hi]` — unicast RTP/RTCP port pair chosen by the client.
    pub client_port: Option<(u16, u16)>,
    /// `server_port=lo[-hi]` — unicast RTP/RTCP port pair chosen by the server.
    pub server_port: Option<(u16, u16)>,
    /// `port=lo[-hi]` — multicast RTP/RTCP port pair.
    pub port: Option<(u16, u16)>,
    /// `ttl=N` — multicast time-to-live.
    pub ttl: Option<u8>,
    /// `layers=N` — number of multicast layers.
    pub layers: Option<u32>,
    /// `ssrc=HHHHHHHH` — 32-bit RTP SSRC (unicast only).
    pub ssrc: Option<u32>,
    /// `destination[=addr]`; a bare `destination` is an empty string.
    pub destination: Option<String>,
    /// `source[=addr]`; a bare `source` is an empty string.
    pub source: Option<String>,
    /// `mode` — the methods this session supports (`PLAY`, `RECORD`, …); empty = absent.
    pub mode: Vec<TransportMode>,
    /// `append` flag (RECORD mode).
    pub append: bool,
    /// Parameters this crate does not model, `(name, value)` in input order
    /// (a value-less parameter has `None`; quoted values are stored unquoted).
    pub extensions: Vec<(String, Option<String>)>,
}

fn bad<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::TransportParse(msg.into()))
}

impl From<lex::LexError> for Error {
    fn from(e: lex::LexError) -> Self {
        Error::TransportParse(e.0)
    }
}

/// Narrowing helpers: the lexer range-checks against the target maximum first.
fn pair_u8(p: (u32, u32)) -> (u8, u8) {
    (p.0 as u8, p.1 as u8)
}
fn pair_u16(p: (u32, u32)) -> (u16, u16) {
    (p.0 as u16, p.1 as u16)
}

impl TransportSpec {
    /// A fresh RTP/AVP/TCP interleaved spec on the given channel range — the
    /// common client SETUP for TCP tunnelling (RFC 2326 §10.12).
    pub fn rtp_avp_tcp_interleaved(lo: u8, hi: u8) -> Self {
        TransportSpec {
            lower_transport: Some(LowerTransport::Tcp),
            delivery: Some(Delivery::Unicast),
            interleaved: Some((lo, hi)),
            ..Default::default()
        }
    }

    fn parse(spec: &str) -> Result<Self> {
        let mut segs = lex::split_outside_quotes(spec, lex::PARAM_SEP).into_iter();
        let head = segs.next().unwrap_or("");
        let triple: Vec<&str> = lex::split_outside_quotes(head, lex::SLASH)
            .into_iter()
            .map(lex::trim_lws)
            .collect();
        let (proto, profile, lower) = match triple.as_slice() {
            [p, pr] => (*p, *pr, None),
            [p, pr, l] => (*p, *pr, Some(*l)),
            _ => {
                return bad(format!(
                    "transport-spec {head:?} is not transport/profile[/lower]"
                ));
            }
        };
        if !proto.eq_ignore_ascii_case(PROTO_RTP) {
            return bad(format!(
                "unsupported transport protocol {proto:?} (only RTP)"
            ));
        }
        if !profile.eq_ignore_ascii_case(PROFILE_AVP) {
            return bad(format!("unsupported profile {profile:?} (only AVP)"));
        }
        let mut out = TransportSpec {
            lower_transport: match lower {
                None => None,
                Some(l) if l.eq_ignore_ascii_case(LOWER_TCP) => Some(LowerTransport::Tcp),
                Some(l) if l.eq_ignore_ascii_case(LOWER_UDP) => Some(LowerTransport::Udp),
                Some(l) => return bad(format!("unknown lower-transport {l:?}")),
            },
            ..Default::default()
        };
        for seg in segs {
            if lex::trim_lws(seg).is_empty() {
                continue;
            }
            let (name, raw) = lex::split_param(seg)?;
            out.apply(name, raw)?;
        }
        Ok(out)
    }

    /// Applies one `name[=value]` parameter.
    fn apply(&mut self, name: &str, raw: Option<&str>) -> Result<()> {
        let is = |n: &str| name.eq_ignore_ascii_case(n);
        // The value, unquoted; `None` for a value-less parameter.
        let value = match raw {
            Some(v) => Some(lex::unquote(v)?.0),
            None => None,
        };
        let need = |what: &str| -> Result<&str> {
            match &value {
                Some(v) => Ok(v.as_str()),
                None => bad(format!("{what} requires a value")),
            }
        };
        let flag = |what: &str| -> Result<()> {
            match &value {
                None => Ok(()),
                Some(_) => bad(format!("{what} takes no value")),
            }
        };
        if is("unicast") {
            flag("unicast")?;
            self.delivery = Some(Delivery::Unicast);
        } else if is("multicast") {
            flag("multicast")?;
            self.delivery = Some(Delivery::Multicast);
        } else if is("append") {
            flag("append")?;
            self.append = true;
        } else if is("destination") {
            self.destination = Some(value.unwrap_or_default());
        } else if is("source") {
            self.source = Some(value.unwrap_or_default());
        } else if is("interleaved") {
            let r = lex::range(
                need("interleaved")?,
                CHANNEL_DIGITS,
                u8::MAX.into(),
                "interleaved",
            )?;
            self.interleaved = Some(pair_u8(r));
        } else if is("ttl") {
            let t = lex::range(need("ttl")?, TTL_DIGITS, u8::MAX.into(), "ttl")?;
            if t.0 != t.1 {
                return bad("ttl is a single value");
            }
            self.ttl = Some(t.0 as u8);
        } else if is("layers") {
            self.layers = Some(lex::digits(need("layers")?, LAYERS_DIGITS, "layers")?);
        } else if is("port") {
            self.port = Some(pair_u16(lex::range(
                need("port")?,
                PORT_DIGITS,
                u16::MAX.into(),
                "port",
            )?));
        } else if is("client_port") {
            self.client_port = Some(pair_u16(lex::range(
                need("client_port")?,
                PORT_DIGITS,
                u16::MAX.into(),
                "client_port",
            )?));
        } else if is("server_port") {
            self.server_port = Some(pair_u16(lex::range(
                need("server_port")?,
                PORT_DIGITS,
                u16::MAX.into(),
                "server_port",
            )?));
        } else if is("ssrc") {
            self.ssrc = Some(lex::hex_exact(need("ssrc")?, SSRC_HEX_DIGITS, "ssrc")?);
        } else if is("mode") {
            self.mode = lex::split_outside_quotes(need("mode")?, MODE_SEP)
                .into_iter()
                .map(lex::trim_lws)
                .filter(|m| !m.is_empty())
                .map(|m| {
                    if lex::is_token(m) {
                        Ok(TransportMode::from_token(m))
                    } else {
                        bad(format!("invalid mode method {m:?}"))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
        } else {
            self.extensions.push((name.to_string(), value));
        }
        Ok(())
    }

    /// Serializes this spec to its canonical `Transport` header text (no comma).
    ///
    /// # Errors
    ///
    /// [`Error::HeaderSerialize`] if a field cannot be emitted safely: a control
    /// character (CR, LF, …) in a value, or a parameter / mode name that is not an
    /// RFC 2326 §15.1 token. Nothing unsafe is ever written.
    pub fn to_header_value(&self) -> Result<String> {
        let emit = |v: &str| lex::emit_value(v).map_err(|e| Error::HeaderSerialize(e.0));
        let mut s = format!("{PROTO_RTP}/{PROFILE_AVP}");
        if let Some(lt) = self.lower_transport {
            s.push(lex::SLASH);
            s.push_str(match lt {
                LowerTransport::Tcp => LOWER_TCP,
                LowerTransport::Udp => LOWER_UDP,
            });
        }
        let mut param = |text: String| {
            s.push(lex::PARAM_SEP);
            s.push_str(&text);
        };
        let kv = |k: &str, v: &str| format!("{k}{}{v}", lex::VALUE_SEP);
        let rng =
            |k: &str, lo: u32, hi: u32| format!("{k}{}{lo}{}{hi}", lex::VALUE_SEP, lex::RANGE_SEP);
        if let Some(d) = self.delivery {
            param(d.name().to_string());
        }
        if let Some(d) = &self.destination {
            param(if d.is_empty() {
                "destination".to_string()
            } else {
                kv("destination", &emit(d)?)
            });
        }
        if let Some(d) = &self.source {
            param(if d.is_empty() {
                "source".to_string()
            } else {
                kv("source", &emit(d)?)
            });
        }
        if let Some((lo, hi)) = self.interleaved {
            param(rng("interleaved", lo.into(), hi.into()));
        }
        if self.append {
            param("append".to_string());
        }
        if let Some(ttl) = self.ttl {
            param(kv("ttl", &ttl.to_string()));
        }
        if let Some(l) = self.layers {
            param(kv("layers", &l.to_string()));
        }
        if let Some((lo, hi)) = self.port {
            param(rng("port", lo.into(), hi.into()));
        }
        if let Some((lo, hi)) = self.client_port {
            param(rng("client_port", lo.into(), hi.into()));
        }
        if let Some((lo, hi)) = self.server_port {
            param(rng("server_port", lo.into(), hi.into()));
        }
        if let Some(ssrc) = self.ssrc {
            param(kv(
                "ssrc",
                &format!("{ssrc:0width$X}", width = SSRC_HEX_DIGITS),
            ));
        }
        if !self.mode.is_empty() {
            if let Some(m) = self.mode.iter().find(|m| !lex::is_token(m.name())) {
                return Err(Error::HeaderSerialize(format!(
                    "mode method {:?} is not a token",
                    m.name()
                )));
            }
            let list = self
                .mode
                .iter()
                .map(TransportMode::name)
                .collect::<Vec<_>>()
                .join(&MODE_SEP.to_string());
            param(kv(
                "mode",
                &lex::emit_value_quoted(&list).map_err(|e| Error::HeaderSerialize(e.0))?,
            ));
        }
        for (name, value) in &self.extensions {
            if !lex::is_token(name) {
                return Err(Error::HeaderSerialize(format!(
                    "extension name {name:?} is not a token"
                )));
            }
            param(match value {
                Some(v) => kv(name, &emit(v)?),
                None => name.clone(),
            });
        }
        Ok(s)
    }
}

/// A `Transport` header value: one or more transport-specs in preference order
/// (RFC 2326 §12.39).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Transport {
    /// The transport-specs, in the order they appear (preference order).
    pub specs: Vec<TransportSpec>,
}

impl Transport {
    /// Constructs a `Transport` from a single spec.
    pub fn single(spec: TransportSpec) -> Self {
        Transport { specs: vec![spec] }
    }

    /// Parses a full `Transport` header value (comma-separated specs).
    pub fn parse(value: &str) -> Result<Self> {
        let specs = lex::split_outside_quotes(value, lex::LIST_SEP)
            .into_iter()
            .filter(|s| !lex::trim_lws(s).is_empty())
            .map(TransportSpec::parse)
            .collect::<Result<Vec<_>>>()?;
        if specs.is_empty() {
            return bad("no transport-specs");
        }
        Ok(Transport { specs })
    }

    /// Serializes to the canonical `Transport` header value (specs joined by `,`).
    ///
    /// # Errors
    ///
    /// [`Error::HeaderSerialize`], see [`TransportSpec::to_header_value`].
    pub fn to_header_value(&self) -> Result<String> {
        Ok(self
            .specs
            .iter()
            .map(TransportSpec::to_header_value)
            .collect::<Result<Vec<_>>>()?
            .join(&lex::LIST_SEP.to_string()))
    }

    /// The first spec, if any (the negotiated/preferred transport).
    pub fn first(&self) -> Option<&TransportSpec> {
        self.specs.first()
    }
}
