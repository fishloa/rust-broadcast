//! Typed `Transport` header — RFC 2326 §12.39.
//!
//! Parses and serializes the RTSP `Transport` header value, per the ABNF
//! transcribed in [`docs/transport-header.md`](../docs/transport-header.md).
//! A single header value is a comma-separated list of transport-specs in the
//! client's order of preference; each spec is a `transport/profile[/lower]`
//! triple followed by semicolon-separated parameters
//! (`unicast`/`multicast`, `interleaved=lo-hi`, `client_port=lo-hi`,
//! `server_port=lo-hi`, `port=lo-hi`, `mode`, `ssrc`, `destination`, `source`,
//! `ttl`, `layers`, `append`).
//!
//! The types are round-trippable: `parse` → [`TransportSpec::to_header_value`]
//! preserves the transport triple and every recognised parameter.

use crate::error::{Error, Result};

/// Transport protocol — currently only `RTP` is defined by RFC 2326 §12.39.
const PROTO_RTP: &str = "RTP";
/// Profile — currently only `AVP` is defined.
const PROFILE_AVP: &str = "AVP";

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

/// A single parsed transport-spec from a `Transport` header (RFC 2326 §12.39).
///
/// Only `RTP/AVP` (with optional `/TCP` or `/UDP`) is modelled with typed
/// parameters. The transport triple is fixed to `RTP/AVP`; the lower transport
/// and each recognised parameter are optional.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TransportSpec {
    /// Lower-layer transport. `None` means the token was absent → UDP default.
    pub lower_transport: Option<LowerTransport>,
    /// `unicast` / `multicast`.
    pub delivery: Option<Delivery>,
    /// `interleaved=lo-hi` — the `$`-framing channel pair (RFC 2326 §10.12).
    pub interleaved: Option<(u8, u8)>,
    /// `client_port=lo-hi` — unicast RTP/RTCP port pair chosen by the client.
    pub client_port: Option<(u16, u16)>,
    /// `server_port=lo-hi` — unicast RTP/RTCP port pair chosen by the server.
    pub server_port: Option<(u16, u16)>,
    /// `port=lo-hi` — multicast RTP/RTCP port pair.
    pub port: Option<(u16, u16)>,
    /// `ttl=N` — multicast time-to-live.
    pub ttl: Option<u8>,
    /// `layers=N` — number of multicast layers.
    pub layers: Option<u32>,
    /// `ssrc=HHHHHHHH` — 32-bit RTP SSRC (unicast only).
    pub ssrc: Option<u32>,
    /// `destination[=addr]`.
    pub destination: Option<String>,
    /// `source=addr`.
    pub source: Option<String>,
    /// `mode` — quoted or bare method(s); `PLAY` or `RECORD`.
    pub mode: Option<String>,
    /// `append` flag (RECORD mode).
    pub append: bool,
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

    /// Parses one transport-spec (no comma) from its textual form.
    fn parse_spec(spec: &str) -> Result<Self> {
        let mut parts = spec.split(';');
        let head = parts
            .next()
            .ok_or_else(|| Error::TransportParse("empty transport-spec".into()))?
            .trim();

        // transport-protocol / profile [ / lower-transport ]
        let mut triple = head.split('/');
        let proto = triple.next().unwrap_or("").trim();
        if !proto.eq_ignore_ascii_case(PROTO_RTP) {
            return Err(Error::TransportParse(format!(
                "unsupported transport protocol {proto:?} (only RTP)"
            )));
        }
        let profile = triple
            .next()
            .ok_or_else(|| Error::TransportParse("missing profile".into()))?
            .trim();
        if !profile.eq_ignore_ascii_case(PROFILE_AVP) {
            return Err(Error::TransportParse(format!(
                "unsupported profile {profile:?} (only AVP)"
            )));
        }
        let lower_transport = match triple.next() {
            None => None,
            Some(t) => match t.trim() {
                s if s.eq_ignore_ascii_case("TCP") => Some(LowerTransport::Tcp),
                s if s.eq_ignore_ascii_case("UDP") => Some(LowerTransport::Udp),
                other => {
                    return Err(Error::TransportParse(format!(
                        "unknown lower-transport {other:?}"
                    )))
                }
            },
        };

        let mut out = TransportSpec {
            lower_transport,
            ..Default::default()
        };

        for raw in parts {
            let param = raw.trim();
            if param.is_empty() {
                continue;
            }
            let (key, value) = match param.split_once('=') {
                Some((k, v)) => (k.trim(), Some(v.trim())),
                None => (param, None),
            };
            match key.to_ascii_lowercase().as_str() {
                "unicast" => out.delivery = Some(Delivery::Unicast),
                "multicast" => out.delivery = Some(Delivery::Multicast),
                "append" => out.append = true,
                "interleaved" => {
                    out.interleaved = Some(parse_u8_range(value, "interleaved")?);
                }
                "client_port" => out.client_port = Some(parse_u16_range(value, "client_port")?),
                "server_port" => out.server_port = Some(parse_u16_range(value, "server_port")?),
                "port" => out.port = Some(parse_u16_range(value, "port")?),
                "ttl" => out.ttl = Some(parse_scalar(value, "ttl")?),
                "layers" => out.layers = Some(parse_scalar(value, "layers")?),
                "ssrc" => {
                    let v = value
                        .ok_or_else(|| Error::TransportParse("ssrc requires a value".into()))?;
                    out.ssrc = Some(
                        u32::from_str_radix(v.trim_matches('"'), 16)
                            .map_err(|e| Error::TransportParse(format!("bad ssrc {v:?}: {e}")))?,
                    );
                }
                "destination" => out.destination = value.map(|s| s.trim_matches('"').to_string()),
                "source" => out.source = value.map(|s| s.trim_matches('"').to_string()),
                "mode" => {
                    out.mode = value.map(|s| s.trim_matches('"').to_string());
                }
                // Unknown parameters are ignored per the extensible header grammar.
                _ => {}
            }
        }
        Ok(out)
    }

    /// Serializes this spec to its `Transport` header textual form (no comma).
    pub fn to_header_value(&self) -> String {
        let mut s = String::new();
        s.push_str(PROTO_RTP);
        s.push('/');
        s.push_str(PROFILE_AVP);
        if let Some(lt) = self.lower_transport {
            s.push('/');
            s.push_str(lt.name());
        }
        if let Some(d) = self.delivery {
            s.push(';');
            s.push_str(d.name());
        }
        if let Some(dest) = &self.destination {
            s.push_str(";destination=");
            s.push_str(dest);
        }
        if let Some(src) = &self.source {
            s.push_str(";source=");
            s.push_str(src);
        }
        if let Some((lo, hi)) = self.interleaved {
            s.push_str(&format!(";interleaved={lo}-{hi}"));
        }
        if let Some(ttl) = self.ttl {
            s.push_str(&format!(";ttl={ttl}"));
        }
        if let Some(layers) = self.layers {
            s.push_str(&format!(";layers={layers}"));
        }
        if let Some((lo, hi)) = self.port {
            s.push_str(&format!(";port={lo}-{hi}"));
        }
        if let Some((lo, hi)) = self.client_port {
            s.push_str(&format!(";client_port={lo}-{hi}"));
        }
        if let Some((lo, hi)) = self.server_port {
            s.push_str(&format!(";server_port={lo}-{hi}"));
        }
        if let Some(ssrc) = self.ssrc {
            s.push_str(&format!(";ssrc={ssrc:08X}"));
        }
        if let Some(mode) = &self.mode {
            s.push_str(&format!(";mode=\"{mode}\""));
        }
        if self.append {
            s.push_str(";append");
        }
        s
    }
}

/// A `Transport` header value: one or more transport-specs in preference order
/// (RFC 2326 §12.39).
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
        let specs = value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(TransportSpec::parse_spec)
            .collect::<Result<Vec<_>>>()?;
        if specs.is_empty() {
            return Err(Error::TransportParse("no transport-specs".into()));
        }
        Ok(Transport { specs })
    }

    /// Serializes to a `Transport` header value.
    pub fn to_header_value(&self) -> String {
        self.specs
            .iter()
            .map(TransportSpec::to_header_value)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The first spec, if any (the negotiated/preferred transport).
    pub fn first(&self) -> Option<&TransportSpec> {
        self.specs.first()
    }
}

fn parse_scalar<T: core::str::FromStr>(value: Option<&str>, what: &str) -> Result<T>
where
    T::Err: core::fmt::Display,
{
    let v = value.ok_or_else(|| Error::TransportParse(format!("{what} requires a value")))?;
    v.trim()
        .parse::<T>()
        .map_err(|e| Error::TransportParse(format!("bad {what} {v:?}: {e}")))
}

fn parse_u8_range(value: Option<&str>, what: &str) -> Result<(u8, u8)> {
    let (lo, hi) = split_range(value, what)?;
    let lo: u8 = lo
        .parse()
        .map_err(|e| Error::TransportParse(format!("bad {what} low {lo:?}: {e}")))?;
    let hi: u8 = match hi {
        Some(h) => h
            .parse()
            .map_err(|e| Error::TransportParse(format!("bad {what} high {h:?}: {e}")))?,
        None => lo,
    };
    Ok((lo, hi))
}

fn parse_u16_range(value: Option<&str>, what: &str) -> Result<(u16, u16)> {
    let (lo, hi) = split_range(value, what)?;
    let lo: u16 = lo
        .parse()
        .map_err(|e| Error::TransportParse(format!("bad {what} low {lo:?}: {e}")))?;
    let hi: u16 = match hi {
        Some(h) => h
            .parse()
            .map_err(|e| Error::TransportParse(format!("bad {what} high {h:?}: {e}")))?,
        None => lo,
    };
    Ok((lo, hi))
}

fn split_range<'a>(value: Option<&'a str>, what: &str) -> Result<(&'a str, Option<&'a str>)> {
    let v = value
        .ok_or_else(|| Error::TransportParse(format!("{what} requires a value")))?
        .trim();
    match v.split_once('-') {
        Some((lo, hi)) => Ok((lo.trim(), Some(hi.trim()))),
        None => Ok((v, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_interleaved_round_trip() {
        let t = Transport::parse("RTP/AVP/TCP;interleaved=0-1").unwrap();
        let spec = t.first().unwrap();
        assert_eq!(spec.lower_transport, Some(LowerTransport::Tcp));
        assert_eq!(spec.interleaved, Some((0, 1)));
        // re-serialize and re-parse must be equal
        let s = t.to_header_value();
        let t2 = Transport::parse(&s).unwrap();
        assert_eq!(t, t2);
    }

    #[test]
    fn parse_udp_unicast_client_port_round_trip() {
        let t = Transport::parse("RTP/AVP;unicast;client_port=8000-8001").unwrap();
        let spec = t.first().unwrap();
        assert_eq!(spec.lower_transport, None);
        assert_eq!(spec.delivery, Some(Delivery::Unicast));
        assert_eq!(spec.client_port, Some((8000, 8001)));
        let t2 = Transport::parse(&t.to_header_value()).unwrap();
        assert_eq!(t, t2);
    }

    #[test]
    fn parse_fixture_setup_transport() {
        let t = Transport::parse("RTP/AVP/TCP;unicast;interleaved=0-1").unwrap();
        let spec = t.first().unwrap();
        assert_eq!(spec.delivery, Some(Delivery::Unicast));
        assert_eq!(spec.interleaved, Some((0, 1)));
    }

    #[test]
    fn ssrc_round_trips_as_hex() {
        let t = Transport::parse("RTP/AVP;unicast;ssrc=DEADBEEF").unwrap();
        assert_eq!(t.first().unwrap().ssrc, Some(0xDEAD_BEEF));
        let t2 = Transport::parse(&t.to_header_value()).unwrap();
        assert_eq!(t, t2);
    }

    #[test]
    fn constructed_spec_serializes_fields() {
        // Build from typed fields (not parsed), serialize, and assert the
        // mutated field appears — rules out a fixture-only round-trip.
        let spec = TransportSpec {
            interleaved: Some((2, 3)),
            delivery: Some(Delivery::Unicast),
            lower_transport: Some(LowerTransport::Tcp),
            ..Default::default()
        };
        let t = Transport::single(spec);
        let s = t.to_header_value();
        assert!(s.contains("interleaved=2-3"), "serialized: {s}");
        assert!(s.contains("unicast"), "serialized: {s}");
        // round-trips back to the same typed value
        let back = Transport::parse(&s).unwrap();
        assert_eq!(back.first().unwrap().interleaved, Some((2, 3)));
    }

    #[test]
    fn rejects_non_rtp() {
        assert!(Transport::parse("XYZ/AVP").is_err());
    }
}
