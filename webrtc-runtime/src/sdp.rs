//! The SDP documents a WHIP or WHEP endpoint signals, and the ICE-restart
//! answer fragment (RFC 8866 §5, RFC 8840 §4.4) it sends for one.
//!
//! An offer is written by a publisher or viewer and parsed here; the answer
//! is this side's whole session description — the `v=`/`o=`/`s=`/`c=`/`t=`
//! lines and one media section — and the fragment is the answer's `a=` lines
//! and media section only. Because signalling is what this crate owns, it is
//! also what writes them: a caller supplies the typed inputs (its local
//! address, its ICE credentials, its DTLS certificate fingerprint, the
//! offer's media section, and the candidates it gathered) and gets the
//! document back, or the crate's own error — never a lossy conversion.
//!
//! Field order is fixed by RFC 8866 §5, so the bytes are deterministic:
//! one CRLF-terminated line per field, an `a=` attribute on its own line, and
//! a `t=0 0` time description. A resource description (`a=ice-ufrag`,
//! `a=ice-pwd`) is either an offer/answer's `a=` line or, in a fragment, a
//! session-level line ahead of one `m=` section (RFC 8840 §4.4).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

use sdp_types::{Attribute, Connection, Media, Origin, Session};

use crate::error::Error;

/// The `s=` session name a WHIP/WHEP answer carries, the same value the
/// endpoint's own offer uses.
pub const SESSION_NAME: &str = "-";

/// How this crate's answer descriptions sign DTLS certificates (RFC 8122
/// §5). SHA-256 is the one hash function RFC 8827 §6.5 makes mandatory.
const FINGERPRINT_HASH: &str = "sha-256";

/// The media identification attribute (RFC 8843 §9.1) a bundled answer
/// echoes.
const MID_ATTRIBUTE: &str = "mid";

/// The `a=mid` an offer without one is answered with: the first bundled
/// media section (RFC 8843 §9.1).
pub const DEFAULT_MID: &str = "0";

/// The line terminator every SDP line ends with (RFC 8866 §5).
const CRLF: &str = "\r\n";

/// The `a=rtcp:` address an answer names: the peer's, never this side's
/// real one (RFC 8866 §5.13).
const RTCP_FALLBACK_ADDRESS: &str = "0.0.0.0";

/// The address a fragment's non-existent session-level `o=` names: a
/// fragment has no session header lines, so the value never reaches it.
const FRAGMENT_ORIGIN_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

/// Which way media flows in an answer (`a=recvonly`/`a=sendonly`,
/// RFC 8866 §6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Direction {
    /// A WHIP answer: the publisher sends, this side receives. The offer's
    /// `a=rtpmap`/`a=fmtp`/`a=rtcp-fb` attributes naming the chosen payload
    /// type are written, because the answer must describe the same encoding
    /// the publisher sends (RFC 6184 §8.1).
    Receive,
    /// A WHEP answer: this side sends, the viewer receives. The codec
    /// parameters are this side's own, from the track it sends.
    Send,
}

impl Direction {
    /// The SDP attribute token for this direction.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Receive => "recvonly",
            Self::Send => "sendonly",
        }
    }
}

broadcast_common::impl_spec_display!(Direction);

/// One per-payload-type codec attribute of an answer, e.g. `a=rtpmap`'s
/// `96 H264/90000`. The name is one of RFC 8866 §5.13's attribute names
/// (`rtpmap`, `fmtp`, `rtcp-fb`) and the value names the payload type first,
/// so a caller cannot smuggle a line break into the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecAttribute {
    /// The attribute's name.
    pub name: CodecAttributeName,
    /// The attribute's value.
    pub value: String,
}

impl CodecAttribute {
    /// An attribute of `name` carrying `value` verbatim.
    pub fn new(name: CodecAttributeName, value: String) -> Self {
        Self { name, value }
    }
}

/// The codec attributes an answer may carry (RFC 8866 §5.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecAttributeName {
    /// `a=rtpmap`: the payload type's encoding name and clock rate
    /// (RFC 8866 §5.13).
    Rtpmap,
    /// `a=fmtp`: format-specific parameters, e.g. RFC 6184 §8.1's
    /// `packetization-mode`.
    Fmtp,
    /// `a=rtcp-fb`: RTP feedback the sender supports (RFC 4585).
    RtcpFb,
}

impl CodecAttributeName {
    /// The SDP attribute name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Rtpmap => "rtpmap",
            Self::Fmtp => "fmtp",
            Self::RtcpFb => "rtcp-fb",
        }
    }
}

broadcast_common::impl_spec_display!(CodecAttributeName);

/// The `m=` line an answer carries: the offer's transport, with the ONE
/// payload type this side accepts (`am=video 9 UDP/TLS/RTP/SAVPF 96`).
#[derive(Debug, Clone)]
pub struct AnswerTransport {
    /// The offer's media type (`video`), port and transport protocol.
    pub offer: Media,
    /// The payload type the answer names (RFC 3264 §6.1).
    pub payload_type: u8,
}

impl AnswerTransport {
    /// The offer's transport narrowed to `payload_type`, as the `m=` line
    /// renders it.
    pub fn media(&self) -> Media {
        let mut media = self.offer.clone();
        media.fmt = self.payload_type.to_string();
        media.connections = Vec::new();
        media.bandwidths = Vec::new();
        media.key = None;
        media.attributes = Vec::new();
        media.media_title = None;
        media
    }
}

/// The media section an answer describes: which way media flows, the
/// transport and payload type, this side's per-payload-type attributes, and
/// (for a reception) the offer attributes to echo.
#[derive(Debug, Clone)]
pub struct AnswerCodec {
    /// Which way media flows.
    pub direction: Direction,
    /// The `m=` line.
    pub transport: AnswerTransport,
    /// Codec attributes in the order they are written.
    pub codec_attributes: Vec<CodecAttribute>,
}

/// This side's ICE credentials and DTLS certificate fingerprint (§5.13
/// `a=ice-ufrag`/`a=ice-pwd`/`a=fingerprint`).
#[derive(Clone, Copy)]
pub struct LocalMediaIdentity<'a> {
    /// The `a=ice-ufrag` value of this generation.
    pub ice_ufrag: &'a str,
    /// The `a=ice-pwd` value of this generation.
    pub ice_pwd: &'a str,
    /// The `sha-256` digest of this side's DTLS certificate, in SDP's
    /// colon-separated hex form.
    pub fingerprint: &'a str,
}

/// The offer's codec attributes that name the answer's payload type: every
/// `a=rtpmap`/`a=fmtp`/`a=rtcp-fb` whose value starts with `payload_type` and
/// a space, in offer order. Their values carry no line break, so echoing
/// them cannot break the answer's framing.
pub fn offer_codec_attributes(media: &Media, payload_type: u8) -> Vec<CodecAttribute> {
    let prefix = format!("{payload_type} ");
    media
        .attributes
        .iter()
        .filter_map(|attribute| {
            let mut names = [
                CodecAttributeName::Rtpmap,
                CodecAttributeName::Fmtp,
                CodecAttributeName::RtcpFb,
            ]
            .into_iter();
            let name = names.find(|name| name.name() == attribute.attribute)?;
            let value = attribute.value.as_deref()?;
            value
                .starts_with(&prefix)
                .then(|| CodecAttribute::new(name, String::from(value)))
        })
        .collect()
}

/// The `a=mid` the answer echoes: the offer media section's own `a=mid`
/// value, or [`DEFAULT_MID`] when it carries none.
pub fn offer_mid(media: &Media) -> String {
    media
        .get_first_attribute_value(MID_ATTRIBUTE)
        .flatten()
        .map_or_else(|| String::from(DEFAULT_MID), String::from)
}

/// Renders this side's whole SDP answer (RFC 8866 §5) over one media
/// section.
///
/// The origin's connection address is `local_addr`, so the address family
/// is derived from the typed address and no CR, LF or space can reach the
/// line. Candidates are written as they were gathered, one `a=candidate`
/// each, then `a=end-of-candidates`.
pub fn session_answer(
    local_addr: IpAddr,
    local: LocalMediaIdentity<'_>,
    mid: &str,
    setup: &str,
    codec: &AnswerCodec,
    candidates: &[String],
) -> Result<String, Error> {
    let mut session = Session::new(Origin::with_ip_addr("0", 0, local_addr), SESSION_NAME);
    session.medias.push(media_section(
        local_addr, local, mid, setup, codec, candidates,
    ));
    render_session(&session)
}

/// The session header lines an ICE fragment is parsed against: a fragment
/// carries none (RFC 8840 §4.4), so a parser prepends these to its own lines
/// and parses the result as one session.
pub fn fragment_prelude() -> Result<String, Error> {
    render_session(&Session::new(
        Origin::with_ip_addr("0", 0, FRAGMENT_ORIGIN_ADDRESS),
        SESSION_NAME,
    ))
}

/// Renders the ICE-restart answer fragment (RFC 8840 §4.4): this side's new
/// credentials at session level and its candidates.
pub fn restart_answer(
    local: LocalMediaIdentity<'_>,
    codec: &AnswerCodec,
    candidates: &[String],
) -> Result<String, Error> {
    let mut session = Session::new(
        Origin::with_ip_addr("0", 0, FRAGMENT_ORIGIN_ADDRESS),
        SESSION_NAME,
    );
    session.add_attribute_with_value("ice-ufrag", local.ice_ufrag);
    session.add_attribute_with_value("ice-pwd", local.ice_pwd);
    session.medias.push(fragment_media(codec, candidates));
    let rendered = render_session(&session)?;
    Ok(fragment_lines(&rendered))
}

fn media_section(
    local_addr: IpAddr,
    local: LocalMediaIdentity<'_>,
    mid: &str,
    setup: &str,
    codec: &AnswerCodec,
    candidates: &[String],
) -> Media {
    let mut section = answer_media(codec, local_addr);
    section.add_attribute_with_value("mid", mid);
    section.add_attribute(Attribute::new("rtcp-mux"));
    section.add_attribute_with_value("ice-ufrag", local.ice_ufrag);
    section.add_attribute_with_value("ice-pwd", local.ice_pwd);
    section.add_attribute_with_value(
        "fingerprint",
        format!("{FINGERPRINT_HASH} {}", local.fingerprint),
    );
    section.add_attribute_with_value("setup", setup);
    for candidate in candidates {
        section.add_attribute_with_value("candidate", candidate);
    }
    section.add_attribute(Attribute::new("end-of-candidates"));
    section
}

/// The `m=` section a fragment carries: the offer's transport narrowed to
/// the answer's payload type, the codec attributes, then the candidates. The
/// `a=rtcp:` line, a direction and `a=end-of-candidates` are an answer's,
/// not a fragment's.
fn fragment_media(codec: &AnswerCodec, candidates: &[String]) -> Media {
    let mut section = codec.transport.media();
    for attribute in &codec.codec_attributes {
        section.add_attribute_with_value(attribute.name.name(), &attribute.value);
    }
    for candidate in candidates {
        section.add_attribute_with_value("candidate", candidate);
    }
    section
}

/// The `m=` section an answer carries ahead of this side's ICE/DTLS
/// attributes: the offer's transport narrowed to the answer's payload type,
/// the `a=rtcp:` line, the codec attributes and the direction.
fn answer_media(codec: &AnswerCodec, local_addr: IpAddr) -> Media {
    let mut section = codec.transport.media();
    section.add_connection(Connection::from_ip_addr(local_addr));
    section.add_attribute_with_value("rtcp", format!("9 IN IP4 {RTCP_FALLBACK_ADDRESS}"));
    for attribute in &codec.codec_attributes {
        section.add_attribute_with_value(attribute.name.name(), &attribute.value);
    }
    section.add_attribute(Attribute::new(codec.direction.name()));
    section
}

fn render_session(session: &Session) -> Result<String, Error> {
    let mut out = Vec::new();
    session.write(&mut out).map_err(|source| Error::SdpWrite {
        what: "write",
        reason: format!("{source}"),
    })?;
    String::from_utf8(out).map_err(|source| Error::SdpWrite {
        what: "utf-8",
        reason: format!("{source}"),
    })
}

fn fragment_lines(rendered: &str) -> String {
    let mut out = String::new();
    for line in rendered.lines() {
        if matches!(line.as_bytes().first(), Some(b'a' | b'm')) {
            out.push_str(line);
            out.push_str(CRLF);
        }
    }
    out
}
