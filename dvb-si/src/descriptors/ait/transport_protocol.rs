//! Transport Protocol Descriptor — ETSI TS 102 809 §5.3.6.1, Table 28
//! (AIT tag 0x02).
//!
//! Carried in the AIT descriptor loops. Identifies the transport mechanism
//! for an application via `protocol_id` + `transport_protocol_label` + selector
//! bytes. The selector bytes are interpreted per `protocol_id`:
//!
//! - 0x0001: Object Carousel (Table 31)
//! - 0x0003: Interaction / HTTP (Table 32)
//! - other: Unknown (raw bytes preserved)

use crate::descriptors::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for transport_protocol_descriptor (AIT namespace).
pub const TAG: u8 = 0x02;
const HEADER_LEN: usize = 2;
const PROTOCOL_ID_LEN: usize = 2;
const LABEL_LEN: usize = 1;

/// Protocol ID for Object Carousel — ETSI TS 102 809 Table 29.
pub const PROTOCOL_ID_OBJECT_CAROUSEL: u16 = 0x0001;
/// Protocol ID for HTTP interaction channel — ETSI TS 102 809 Table 29.
pub const PROTOCOL_ID_HTTP: u16 = 0x0003;

/// Decoded Object Carousel selector — Table 31.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OcSelector {
    /// 1-bit remote_connection flag.
    pub remote_connection: bool,
    /// Present only when `remote_connection` is true.
    pub remote_connection_info: Option<OcRemoteConnection>,
    /// Component tag.
    pub component_tag: u8,
}

/// Remote connection fields inside the OC selector (Table 31).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OcRemoteConnection {
    /// 16-bit original_network_id.
    pub original_network_id: u16,
    /// 16-bit transport_stream_id.
    pub transport_stream_id: u16,
    /// 16-bit service_id.
    pub service_id: u16,
}

/// One URL entry in the HTTP interaction selector — Table 32.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HttpUrlEntry<'a> {
    /// URL base bytes.
    pub url_base: &'a [u8],
    /// URL extension strings.
    pub url_extensions: Vec<&'a [u8]>,
}

/// Decoded HTTP interaction selector — Table 32.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HttpSelector<'a> {
    /// URL entries.
    pub urls: Vec<HttpUrlEntry<'a>>,
}

/// Typed selector decoded from the raw selector bytes by `protocol_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SelectorKind<'a> {
    /// Object Carousel selector (protocol_id 0x0001).
    ObjectCarousel(OcSelector),
    /// HTTP interaction selector (protocol_id 0x0003).
    Http(HttpSelector<'a>),
    /// Unknown protocol — raw selector bytes.
    Unknown(&'a [u8]),
}

impl SelectorKind<'_> {
    /// Spec name for the selector variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ObjectCarousel(_) => "OBJECT_CAROUSEL",
            Self::Http(_) => "HTTP",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}
dvb_common::impl_spec_display!(SelectorKind<'_>);

/// Transport Protocol Descriptor (AIT tag 0x02).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TransportProtocolDescriptor<'a> {
    /// 16-bit protocol_id (Table 29).
    pub protocol_id: u16,
    /// Transport protocol label.
    pub transport_protocol_label: u8,
    /// Raw selector bytes (after protocol_id + label). Use `selector()`
    /// for typed decoding.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector_bytes: &'a [u8],
}

impl<'a> TransportProtocolDescriptor<'a> {
    /// Decode the selector bytes according to `protocol_id`.
    #[must_use]
    pub fn selector(&self) -> SelectorKind<'a> {
        match self.protocol_id {
            PROTOCOL_ID_OBJECT_CAROUSEL => self.decode_oc_selector(),
            PROTOCOL_ID_HTTP => self.decode_http_selector(),
            _ => SelectorKind::Unknown(self.selector_bytes),
        }
    }

    fn decode_oc_selector(&self) -> SelectorKind<'a> {
        let b = self.selector_bytes;
        if b.len() < 2 {
            return SelectorKind::Unknown(b);
        }
        let remote_connection = (b[0] & 0x80) != 0;
        let mut pos = 1;
        let remote_connection_info = if remote_connection {
            if pos + 6 > b.len() {
                return SelectorKind::Unknown(b);
            }
            let info = OcRemoteConnection {
                original_network_id: u16::from_be_bytes([b[pos], b[pos + 1]]),
                transport_stream_id: u16::from_be_bytes([b[pos + 2], b[pos + 3]]),
                service_id: u16::from_be_bytes([b[pos + 4], b[pos + 5]]),
            };
            pos += 6;
            Some(info)
        } else {
            None
        };
        // component_tag must still be present; a malformed selector that
        // declared remote_connection but stops before the component_tag
        // (e.g. exactly 7 bytes) falls back to Unknown rather than panicking.
        if pos >= b.len() {
            return SelectorKind::Unknown(b);
        }
        let component_tag = b[pos];
        SelectorKind::ObjectCarousel(OcSelector {
            remote_connection,
            remote_connection_info,
            component_tag,
        })
    }

    fn decode_http_selector(&self) -> SelectorKind<'a> {
        let b = self.selector_bytes;
        let mut urls = Vec::new();
        let mut pos = 0;
        while pos < b.len() {
            let url_base_length = b[pos] as usize;
            pos += 1;
            if pos + url_base_length > b.len() {
                break;
            }
            let base_end = pos + url_base_length;
            let url_base = &b[pos..base_end];
            pos = base_end;
            if pos >= b.len() {
                urls.push(HttpUrlEntry {
                    url_base,
                    url_extensions: Vec::new(),
                });
                break;
            }
            let url_extension_count = b[pos] as usize;
            pos += 1;
            let mut url_extensions = Vec::with_capacity(url_extension_count);
            for _ in 0..url_extension_count {
                if pos >= b.len() {
                    break;
                }
                let ext_len = b[pos] as usize;
                pos += 1;
                let ext_end = pos + ext_len;
                if ext_end > b.len() {
                    break;
                }
                url_extensions.push(&b[pos..ext_end]);
                pos = ext_end;
            }
            urls.push(HttpUrlEntry {
                url_base,
                url_extensions,
            });
        }
        SelectorKind::Http(HttpSelector { urls })
    }
}

impl<'a> Parse<'a> for TransportProtocolDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TransportProtocolDescriptor",
            "unexpected tag for transport_protocol_descriptor",
        )?;
        if body.len() < PROTOCOL_ID_LEN + LABEL_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "transport_protocol_descriptor body shorter than minimum 3 bytes",
            });
        }
        let protocol_id = u16::from_be_bytes([body[0], body[1]]);
        let transport_protocol_label = body[2];
        let selector_bytes = &body[PROTOCOL_ID_LEN + LABEL_LEN..];
        Ok(Self {
            protocol_id,
            transport_protocol_label,
            selector_bytes,
        })
    }
}

impl Serialize for TransportProtocolDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + PROTOCOL_ID_LEN + LABEL_LEN + self.selector_bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let body_len = len - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "transport_protocol_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[2..4].copy_from_slice(&self.protocol_id.to_be_bytes());
        buf[4] = self.transport_protocol_label;
        buf[5..5 + self.selector_bytes.len()].copy_from_slice(self.selector_bytes);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TransportProtocolDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TRANSPORT_PROTOCOL";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body: protocol_id(2) + label(1) + remote_conn_rfu(1) + component_tag(1) = 5.
    fn build_oc_local() -> [u8; 7] {
        [TAG, 5, 0x00, 0x01, 0x01, 0x00, 0xB4]
    }

    /// Body: protocol_id(2) + label(1) + remote_conn(1) + org_id(2) + ts_id(2) + service_id(2) + component_tag(1) = 11.
    fn build_oc_remote() -> [u8; 13] {
        [
            TAG, 11, 0x00, 0x01, 0x01, 0x80, // remote_connection=1, rfu=0000000
            0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x04, // component_tag
        ]
    }

    /// Body: protocol_id(2) + label(1) + url_base_len(1) + "http"(4) + ext_count(1) + ext_len(1) + "/app"(4) = 14.
    fn build_http() -> [u8; 16] {
        [
            TAG, 14, 0x00, 0x03, 0x01, 4, b'h', b't', b't', b'p', 1, // url_extension_count
            4, b'/', b'a', b'p', b'p',
        ]
    }

    #[test]
    fn parse_oc_local_selector() {
        let bytes = build_oc_local();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.protocol_id, PROTOCOL_ID_OBJECT_CAROUSEL);
        assert_eq!(d.transport_protocol_label, 0x01);
        assert_eq!(d.selector_bytes, &[0x00, 0xB4]);
        match d.selector() {
            SelectorKind::ObjectCarousel(oc) => {
                assert!(!oc.remote_connection);
                assert!(oc.remote_connection_info.is_none());
                assert_eq!(oc.component_tag, 0xB4);
            }
            other => panic!("expected ObjectCarousel, got {other:?}"),
        }
    }

    #[test]
    fn parse_oc_remote_selector() {
        let bytes = build_oc_remote();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        match d.selector() {
            SelectorKind::ObjectCarousel(oc) => {
                assert!(oc.remote_connection);
                let rc = oc.remote_connection_info.unwrap();
                assert_eq!(rc.original_network_id, 1);
                assert_eq!(rc.transport_stream_id, 2);
                assert_eq!(rc.service_id, 3);
                assert_eq!(oc.component_tag, 0x04);
            }
            other => panic!("expected ObjectCarousel, got {other:?}"),
        }
    }

    #[test]
    fn parse_http_selector() {
        let bytes = build_http();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.protocol_id, PROTOCOL_ID_HTTP);
        match d.selector() {
            SelectorKind::Http(http) => {
                assert_eq!(http.urls.len(), 1);
                assert_eq!(http.urls[0].url_base, b"http");
                assert_eq!(http.urls[0].url_extensions.len(), 1);
                assert_eq!(http.urls[0].url_extensions[0], b"/app");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn oc_remote_selector_missing_component_tag_is_unknown_not_panic() {
        // protocol_id=0x0001, label, then a 7-byte selector that declares
        // remote_connection=1 (+6 bytes of triplet) but omits component_tag.
        // selector() must return Unknown, not panic.
        let bytes = [
            TAG, 10, // body = 10
            0x00, 0x01, // protocol_id = 0x0001 (OC)
            0x01, // transport_protocol_label
            0x80, // remote_connection=1
            0x00, 0x01, 0x00, 0x02, 0x00, 0x03, // 6-byte triplet, then NO component_tag
        ];
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.selector_bytes.len(), 7); // flag(1) + triplet(6), component_tag missing
        assert!(d.selector_bytes[0] & 0x80 != 0); // remote_connection set
        assert!(matches!(d.selector(), SelectorKind::Unknown(_)));
    }

    #[test]
    fn parse_unknown_protocol() {
        let bytes = [TAG, 5, 0x01, 0x00, 0x02, 0xCA, 0xFE];
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.protocol_id, 0x0100);
        assert!(matches!(d.selector(), SelectorKind::Unknown(_)));
    }

    #[test]
    fn serialize_round_trip_oc() {
        let bytes = build_oc_local();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
        let re = TransportProtocolDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_http() {
        let bytes = build_http();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
        let re = TransportProtocolDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_oc_remote() {
        let bytes = build_oc_remote();
        let d = TransportProtocolDescriptor::parse(&bytes).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), &bytes[..]);
    }
}
