//! `MulticastGatewayConfigurationTransportSession` and the macro-expansion
//! elements — ETSI TS 103 769 V1.2.1 clause 10.2.5 (Table 10.2.5.1-1) and
//! clause 10.2.5.2.
//!
//! Used only for the in-band gateway-configuration transport method
//! (`docs/mabr-signalling.md` §1 method 3): the Multicast server carousels
//! the current gateway configuration document as a multicast transport
//! object on a dedicated session. Same element/attribute set as
//! `MulticastTransportSession` (`transport.rs`) **except**: no
//! `@id`/`@start`/`@duration`/`@contentIngestMethod`/`@transmissionMode`, no
//! `ServiceComponentIdentifier`; instead adds `@tags` and
//! `MulticastGatewayConfigurationMacro` children.

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use roxmltree::Node;

use crate::carousel::ObjectCarousel;
use crate::error::Result;
use crate::fec::ForwardErrorCorrectionParameters;
use crate::parse::{child, children, own_text, req_attr_u64, require_attr, require_child};
use crate::repair::UnicastRepairParameters;
use crate::serialize::{push_indent, write_attr, write_num_attr, write_opt_attr};
use crate::transport::{BitRate, EndpointAddress, TransportProtocol, TransportSecurity};

const ELEMENT: &str = "MulticastGatewayConfigurationTransportSession";
const MACRO_ELEMENT: &str = "MulticastGatewayConfigurationMacro";

/// A macro-expansion key/value pair: `MulticastServerConfigurationMacro`
/// (clause 10.2.1, server-configuration document root, `config.rs`) or
/// `MulticastGatewayConfigurationMacro` (clause 10.2.5.2, per-transport-session,
/// this module) — both share the same shape: `@key` (NameToken) names the
/// `$key$` token substituted elsewhere in the document; the element content
/// is the substitution value.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ConfigurationMacro {
    /// The macro key (a NameToken); occurrences of `$key$` elsewhere in the
    /// document are replaced with `value`.
    pub key: String,
    /// The substitution value.
    pub value: String,
}

impl ConfigurationMacro {
    pub(crate) fn parse(node: Node<'_, '_>, element: &'static str) -> Result<Self> {
        Ok(ConfigurationMacro {
            key: require_attr(node, element, "key")?,
            value: own_text(node),
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize, tag: &str) {
        push_indent(out, indent);
        out.push('<');
        out.push_str(tag);
        write_attr(out, "key", &self.key);
        out.push('>');
        out.push_str(&crate::serialize::xml_escape(&self.value));
        out.push_str("</");
        out.push_str(tag);
        out.push_str(">\n");
    }
}

/// `MulticastGatewayConfigurationTransportSession` (clause 10.2.5, Table
/// 10.2.5.1-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastGatewayConfigurationTransportSession {
    /// Content-class term (same semantics as `MulticastTransportSession::service_class`).
    pub service_class: Option<String>,
    /// See `docs/mabr-transport.md` §4.
    pub transport_security: Option<TransportSecurity>,
    /// Max inter-packet gap (ms) before the gateway may treat the session
    /// as inactive/unsubscribe.
    pub session_idle_timeout: u64,
    /// The multicast transport protocol carrying this session.
    pub transport_protocol: TransportProtocol,
    /// One or more multicast endpoints.
    pub endpoints: Vec<EndpointAddress>,
    /// Aggregate bit rate across `endpoints`.
    pub bit_rate: BitRate,
    /// AL-FEC parameters, zero or more.
    pub fec_params: Vec<ForwardErrorCorrectionParameters>,
    /// Unicast repair configuration, if any.
    pub unicast_repair: Option<UnicastRepairParameters>,
    /// In-band object carousel (`ReferencingObjectCarouselType` — see the
    /// simplification noted in `carousel.rs`).
    pub object_carousel: Option<ObjectCarousel>,
    /// Applicability tags a gateway can filter on (`@tags`,
    /// whitespace-separated URI list on the wire; split into a `Vec` here).
    pub tags: Vec<String>,
    /// Per-transport-session macro overrides (clause 10.2.5.2).
    pub macros: Vec<ConfigurationMacro>,
}

impl MulticastGatewayConfigurationTransportSession {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let transport_protocol_node = require_child(node, ELEMENT, "TransportProtocol")?;
        let bit_rate_node = require_child(node, ELEMENT, "BitRate")?;

        let mut endpoints = Vec::new();
        for ep in children(node, "EndpointAddress") {
            endpoints.push(EndpointAddress::parse(ep)?);
        }
        let mut fec_params = Vec::new();
        for fp in children(node, "ForwardErrorCorrectionParameters") {
            fec_params.push(ForwardErrorCorrectionParameters::parse(fp)?);
        }
        let mut macros = Vec::new();
        for m in children(node, MACRO_ELEMENT) {
            macros.push(ConfigurationMacro::parse(m, MACRO_ELEMENT)?);
        }
        let tags: Vec<String> = require_attr(node, ELEMENT, "tags")
            .map(|t| t.split_whitespace().map(ToString::to_string).collect())
            .unwrap_or_default();

        Ok(MulticastGatewayConfigurationTransportSession {
            service_class: require_attr(node, ELEMENT, "serviceClass").ok(),
            transport_security: match require_attr(node, ELEMENT, "transportSecurity") {
                Ok(v) => Some(TransportSecurity::parse(&v)?),
                Err(_) => None,
            },
            session_idle_timeout: req_attr_u64(node, ELEMENT, "sessionIdleTimeout")?,
            transport_protocol: TransportProtocol::parse(transport_protocol_node)?,
            endpoints,
            bit_rate: BitRate::parse(bit_rate_node)?,
            fec_params,
            unicast_repair: match child(node, "UnicastRepairParameters") {
                Some(n) => Some(UnicastRepairParameters::parse(n)?),
                None => None,
            },
            object_carousel: match child(node, "ObjectCarousel") {
                Some(n) => Some(ObjectCarousel::parse(n)?),
                None => None,
            },
            tags,
            macros,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<MulticastGatewayConfigurationTransportSession");
        write_opt_attr(out, "serviceClass", self.service_class.as_deref());
        if let Some(s) = self.transport_security {
            write_attr(out, "transportSecurity", s.name());
        }
        write_num_attr(out, "sessionIdleTimeout", self.session_idle_timeout);
        if !self.tags.is_empty() {
            write_attr(out, "tags", &self.tags.join(" "));
        }
        out.push_str(">\n");
        self.transport_protocol.write_xml(out, indent + 1);
        for ep in &self.endpoints {
            ep.write_xml(out, indent + 1);
        }
        self.bit_rate.write_xml(out, indent + 1);
        for fp in &self.fec_params {
            fp.write_xml(out, indent + 1);
        }
        if let Some(ur) = &self.unicast_repair {
            ur.write_xml(out, indent + 1);
        }
        if let Some(oc) = &self.object_carousel {
            oc.write_xml(out, indent + 1);
        }
        for m in &self.macros {
            m.write_xml(out, indent + 1, MACRO_ELEMENT);
        }
        push_indent(out, indent);
        out.push_str("</MulticastGatewayConfigurationTransportSession>\n");
    }
}
