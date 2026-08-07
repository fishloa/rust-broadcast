//! `MulticastTransportSession` and its scalar-attribute enums — ETSI TS 103
//! 769 V1.2.1 clause 10.2.3 (Table 10.2.3.1-1, the core element of the whole
//! data model) plus clauses 10.2.3.9 (`EndpointAddress`) and 10.2.3.10
//! (`BitRate`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::carousel::ObjectCarousel;
use crate::component::ServiceComponentIdentifier;
use crate::error::{Error, Result};
use crate::fec::ForwardErrorCorrectionParameters;
use crate::parse::{
    child, child_text, children, opt_attr_u64, req_attr_u32, req_attr_u64, require_attr,
    require_child,
};
use crate::repair::UnicastRepairParameters;
use crate::serialize::{push_indent, write_attr, write_num_attr, write_opt_attr};

const ELEMENT: &str = "MulticastTransportSession";
const ENDPOINT_ELEMENT: &str = "EndpointAddress";
const TRANSPORT_PROTOCOL_ELEMENT: &str = "TransportProtocol";
const BIT_RATE_ELEMENT: &str = "BitRate";

/// `@contentIngestMethod` (clause 10.2.3.1) — server-configuration only; a
/// gateway must ignore it if present. XSD `contentAcquisitionMethodType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContentIngestMethod {
    /// Network control pushes content to the Multicast server.
    Push,
    /// The Multicast server pulls content by polling. Default value.
    Pull,
}

impl ContentIngestMethod {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            ContentIngestMethod::Push => "push",
            ContentIngestMethod::Pull => "pull",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "push" => Ok(ContentIngestMethod::Push),
            "pull" => Ok(ContentIngestMethod::Pull),
            _ => Err(Error::InvalidAttribute {
                element: ELEMENT,
                attr: "contentIngestMethod",
                value: value.into(),
                reason: "expected 'push' or 'pull'",
            }),
        }
    }
}

broadcast_common::impl_spec_display!(ContentIngestMethod);

/// `@transmissionMode` (clause 10.2.3.1) — see `docs/mabr-transport.md` §1.
/// XSD `transmissionModeType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransmissionMode {
    /// Transport objects are addressed as whole resources. Default value.
    Resource,
    /// Transport objects are addressed as a stream of chunks.
    Chunked,
}

impl TransmissionMode {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            TransmissionMode::Resource => "resource",
            TransmissionMode::Chunked => "chunked",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "resource" => Ok(TransmissionMode::Resource),
            "chunked" => Ok(TransmissionMode::Chunked),
            _ => Err(Error::InvalidAttribute {
                element: ELEMENT,
                attr: "transmissionMode",
                value: value.into(),
                reason: "expected 'resource' or 'chunked'",
            }),
        }
    }
}

broadcast_common::impl_spec_display!(TransmissionMode);

/// `@transportSecurity` (clause 10.2.3.1) — see `docs/mabr-transport.md` §4.
/// XSD `transportSecurityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportSecurity {
    /// No integrity or authenticity protection. Default value.
    None,
    /// Integrity protection only.
    Integrity,
    /// Integrity and authenticity protection.
    IntegrityAndAuthenticity,
}

impl TransportSecurity {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            TransportSecurity::None => "none",
            TransportSecurity::Integrity => "integrity",
            TransportSecurity::IntegrityAndAuthenticity => "integrityAndAuthenticity",
        }
    }

    /// Parse the `xs:string` enumeration value. `pub(crate)` (rather than
    /// private like the sibling enums' `parse`) because `gateway.rs` also
    /// needs it for `MulticastGatewayConfigurationTransportSession`.
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(TransportSecurity::None),
            "integrity" => Ok(TransportSecurity::Integrity),
            "integrityAndAuthenticity" => Ok(TransportSecurity::IntegrityAndAuthenticity),
            _ => Err(Error::InvalidAttribute {
                element: ELEMENT,
                attr: "transportSecurity",
                value: value.into(),
                reason: "expected 'none', 'integrity', or 'integrityAndAuthenticity'",
            }),
        }
    }
}

broadcast_common::impl_spec_display!(TransportSecurity);

/// `TransportProtocol` (clause 10.2.3.1) — identifies the multicast
/// transport protocol carrying this session's objects.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TransportProtocol {
    /// A `MulticastTransportProtocolCS` term (Annex B.1), e.g.
    /// `urn:dvb:metadata:cs:MulticastTransportProtocolCS:2019:FLUTE`.
    pub protocol_identifier: String,
    /// Major protocol version number. ⚠️ The prose table (10.2.3.1-1) says
    /// "String" but the XSD (`MulticastTransportProtocolType`, Annex A.2)
    /// says `xs:positiveInteger` — this crate follows the XSD.
    pub protocol_version: u32,
}

impl TransportProtocol {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        Ok(TransportProtocol {
            protocol_identifier: require_attr(
                node,
                TRANSPORT_PROTOCOL_ELEMENT,
                "protocolIdentifier",
            )?,
            protocol_version: req_attr_u32(node, TRANSPORT_PROTOCOL_ELEMENT, "protocolVersion")?,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<TransportProtocol");
        write_attr(out, "protocolIdentifier", &self.protocol_identifier);
        write_num_attr(out, "protocolVersion", self.protocol_version);
        out.push_str("/>\n");
    }
}

/// `EndpointAddress` (clause 10.2.3.9) — one multicast destination (or, for
/// FEC repair packets, an alternate endpoint).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EndpointAddress {
    /// Source address for source-specific multicast (IPv4 dotted-decimal or
    /// IPv6 per RFC 5952).
    pub source: Option<String>,
    /// Multicast group (destination) IP address.
    pub group: String,
    /// UDP destination port (1-65535).
    pub port: u16,
    /// Protocol-specific demux id (e.g. the LCT Transport Session
    /// Identifier / Channel).
    pub transport_session_id: Option<u64>,
}

impl EndpointAddress {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let group_node = require_child(node, ENDPOINT_ELEMENT, "NetworkDestinationGroupAddress")?;
        let port_node = require_child(node, ENDPOINT_ELEMENT, "TransportDestinationPort")?;
        let port_text = crate::parse::own_text(port_node);
        Ok(EndpointAddress {
            source: child_text(node, "NetworkSourceAddress"),
            group: crate::parse::own_text(group_node),
            port: crate::parse::parse_u16(
                ENDPOINT_ELEMENT,
                "TransportDestinationPort",
                &port_text,
            )?,
            transport_session_id: match child_text(node, "MediaTransportSessionIdentifier") {
                Some(t) => Some(crate::parse::parse_u64(
                    ENDPOINT_ELEMENT,
                    "MediaTransportSessionIdentifier",
                    &t,
                )?),
                None => None,
            },
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<EndpointAddress>\n");
        if let Some(source) = &self.source {
            push_indent(out, indent + 1);
            out.push_str("<NetworkSourceAddress>");
            out.push_str(&crate::serialize::xml_escape(source));
            out.push_str("</NetworkSourceAddress>\n");
        }
        push_indent(out, indent + 1);
        out.push_str("<NetworkDestinationGroupAddress>");
        out.push_str(&crate::serialize::xml_escape(&self.group));
        out.push_str("</NetworkDestinationGroupAddress>\n");
        push_indent(out, indent + 1);
        {
            use core::fmt::Write as _;
            let _ = writeln!(
                out,
                "<TransportDestinationPort>{}</TransportDestinationPort>",
                self.port
            );
        }
        if let Some(id) = self.transport_session_id {
            push_indent(out, indent + 1);
            use core::fmt::Write as _;
            let _ = writeln!(
                out,
                "<MediaTransportSessionIdentifier>{id}</MediaTransportSessionIdentifier>"
            );
        }
        push_indent(out, indent);
        out.push_str("</EndpointAddress>\n");
    }
}

/// `BitRate` (clause 10.2.3.10) — across all endpoints declared for this
/// session, including any FEC repair packets addressed to the *same*
/// destination group network address. If FEC uses a different endpoint
/// address, its bit rate is not included here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BitRate {
    /// Average bit rate, bit/s.
    pub average: Option<u64>,
    /// Maximum bit rate, bit/s.
    pub maximum: u64,
}

impl BitRate {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        Ok(BitRate {
            average: opt_attr_u64(node, BIT_RATE_ELEMENT, "average")?,
            maximum: req_attr_u64(node, BIT_RATE_ELEMENT, "maximum")?,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<BitRate");
        crate::serialize::write_opt_num_attr(out, "average", self.average);
        write_num_attr(out, "maximum", self.maximum);
        out.push_str("/>\n");
    }
}

/// `MulticastTransportSession` (clause 10.2.3, Table 10.2.3.1-1) — the core
/// element: one multicast object-delivery session carrying one or more
/// service components.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastTransportSession {
    /// Unique within the parent `MulticastSession`.
    pub id: String,
    /// Content-class term, e.g. from the TS 103 770 §9 vocabulary (DVB-I).
    /// A gateway should ignore the session if the term is unknown to it.
    pub service_class: Option<String>,
    /// Session start (MPEG-7 `TimePoint`).
    pub start: Option<String>,
    /// Session duration (ISO 8601 duration).
    pub duration: Option<String>,
    /// Server-configuration only; a gateway must ignore it if present.
    pub content_ingest_method: Option<ContentIngestMethod>,
    /// See `docs/mabr-transport.md` §1.
    pub transmission_mode: Option<TransmissionMode>,
    /// See `docs/mabr-transport.md` §4.
    pub transport_security: Option<TransportSecurity>,
    /// Max inter-packet gap (ms) before the gateway may treat the session
    /// as inactive/unsubscribe. Takes precedence over other timeouts.
    pub session_idle_timeout: u64,
    /// The multicast transport protocol carrying this session.
    pub transport_protocol: TransportProtocol,
    /// One or more multicast endpoints.
    pub endpoints: Vec<EndpointAddress>,
    /// Aggregate bit rate across `endpoints`.
    pub bit_rate: BitRate,
    /// AL-FEC parameters, zero or more (clause 10.2.3.11).
    pub fec_params: Vec<ForwardErrorCorrectionParameters>,
    /// Unicast repair configuration, if any.
    pub unicast_repair: Option<UnicastRepairParameters>,
    /// In-band object carousel, if any.
    pub object_carousel: Option<ObjectCarousel>,
    /// One or more service-component references (clause 10.2.4).
    pub service_component_ids: Vec<ServiceComponentIdentifier>,
}

impl MulticastTransportSession {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let transport_protocol_node = require_child(node, ELEMENT, TRANSPORT_PROTOCOL_ELEMENT)?;
        let bit_rate_node = require_child(node, ELEMENT, BIT_RATE_ELEMENT)?;

        let mut endpoints = Vec::new();
        for ep in children(node, ENDPOINT_ELEMENT) {
            endpoints.push(EndpointAddress::parse(ep)?);
        }
        let mut fec_params = Vec::new();
        for fp in children(node, "ForwardErrorCorrectionParameters") {
            fec_params.push(ForwardErrorCorrectionParameters::parse(fp)?);
        }
        let mut service_component_ids = Vec::new();
        for sc in children(node, "ServiceComponentIdentifier") {
            service_component_ids.push(ServiceComponentIdentifier::parse(sc)?);
        }

        Ok(MulticastTransportSession {
            id: require_attr(node, ELEMENT, "id")?,
            service_class: require_attr(node, ELEMENT, "serviceClass").ok(),
            start: require_attr(node, ELEMENT, "start").ok(),
            duration: require_attr(node, ELEMENT, "duration").ok(),
            content_ingest_method: match require_attr(node, ELEMENT, "contentIngestMethod") {
                Ok(v) => Some(ContentIngestMethod::parse(&v)?),
                Err(_) => None,
            },
            transmission_mode: match require_attr(node, ELEMENT, "transmissionMode") {
                Ok(v) => Some(TransmissionMode::parse(&v)?),
                Err(_) => None,
            },
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
            service_component_ids,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<MulticastTransportSession");
        write_attr(out, "id", &self.id);
        write_opt_attr(out, "serviceClass", self.service_class.as_deref());
        write_opt_attr(out, "start", self.start.as_deref());
        write_opt_attr(out, "duration", self.duration.as_deref());
        if let Some(m) = self.content_ingest_method {
            write_attr(out, "contentIngestMethod", m.name());
        }
        if let Some(m) = self.transmission_mode {
            write_attr(out, "transmissionMode", m.name());
        }
        if let Some(s) = self.transport_security {
            write_attr(out, "transportSecurity", s.name());
        }
        write_num_attr(out, "sessionIdleTimeout", self.session_idle_timeout);
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
        for sc in &self.service_component_ids {
            sc.write_xml(out, indent + 1);
        }
        push_indent(out, indent);
        out.push_str("</MulticastTransportSession>\n");
    }
}
