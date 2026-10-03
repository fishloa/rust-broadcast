//! `MulticastTransportSession` and its scalar-attribute enums — ETSI TS 103
//! 769 V1.2.1 clause 10.2.3 (Table 10.2.3.1-1, the core element of the whole
//! data model) plus clauses 10.2.3.9 (`EndpointAddress`) and 10.2.3.10
//! (`BitRate`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use alloc::string::ToString;
use std::io;

use crate::carousel::ObjectCarousel;
use crate::component::ServiceComponentIdentifier;
use crate::error::{Error, Result};
use crate::fec::ForwardErrorCorrectionParameters;
use crate::parse::{
    Events, StartTag, for_each_child, missing_element, opt_attr_u64, req_attr_u32, req_attr_u64,
    require_attr,
};
use crate::repair::UnicastRepairParameters;
use crate::serialize::{
    Out, attr, element, empty, num_attr, opt_attr, opt_num_attr, tag, text_element,
};

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
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let parsed = TransportProtocol {
            protocol_identifier: require_attr(
                node,
                TRANSPORT_PROTOCOL_ELEMENT,
                "protocolIdentifier",
            )?,
            protocol_version: req_attr_u32(node, TRANSPORT_PROTOCOL_ELEMENT, "protocolVersion")?,
        };
        ev.skip(node)?;
        Ok(parsed)
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(TRANSPORT_PROTOCOL_ELEMENT);
        attr(&mut t, "protocolIdentifier", &self.protocol_identifier);
        num_attr(&mut t, "protocolVersion", self.protocol_version);
        empty(w, t)
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
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let mut source = None;
        let mut group = None;
        let mut port_text = None;
        let mut session_id_text = None;
        for_each_child(ev, node, |ev, child| {
            if child.is("NetworkSourceAddress") && source.is_none() {
                source = Some(ev.text(child)?);
            } else if child.is("NetworkDestinationGroupAddress") && group.is_none() {
                group = Some(ev.text(child)?);
            } else if child.is("TransportDestinationPort") && port_text.is_none() {
                port_text = Some(ev.text(child)?);
            } else if child.is("MediaTransportSessionIdentifier") && session_id_text.is_none() {
                session_id_text = Some(ev.text(child)?);
            } else {
                return Ok(false);
            }
            Ok(true)
        })?;
        let group = group
            .ok_or_else(|| missing_element(ENDPOINT_ELEMENT, "NetworkDestinationGroupAddress"))?;
        let port_text = port_text
            .ok_or_else(|| missing_element(ENDPOINT_ELEMENT, "TransportDestinationPort"))?;
        Ok(EndpointAddress {
            source,
            group,
            port: crate::parse::parse_u16(
                ENDPOINT_ELEMENT,
                "TransportDestinationPort",
                &port_text,
            )?,
            transport_session_id: match session_id_text {
                Some(t) => Some(crate::parse::parse_u64(
                    ENDPOINT_ELEMENT,
                    "MediaTransportSessionIdentifier",
                    &t,
                )?),
                None => None,
            },
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        element(w, tag(ENDPOINT_ELEMENT), |w| {
            if let Some(source) = &self.source {
                text_element(w, tag("NetworkSourceAddress"), source)?;
            }
            text_element(w, tag("NetworkDestinationGroupAddress"), &self.group)?;
            text_element(w, tag("TransportDestinationPort"), &self.port.to_string())?;
            if let Some(id) = self.transport_session_id {
                text_element(w, tag("MediaTransportSessionIdentifier"), &id.to_string())?;
            }
            Ok(())
        })
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
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let parsed = BitRate {
            average: opt_attr_u64(node, BIT_RATE_ELEMENT, "average")?,
            maximum: req_attr_u64(node, BIT_RATE_ELEMENT, "maximum")?,
        };
        ev.skip(node)?;
        Ok(parsed)
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(BIT_RATE_ELEMENT);
        opt_num_attr(&mut t, "average", self.average);
        num_attr(&mut t, "maximum", self.maximum);
        empty(w, t)
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
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let id = require_attr(node, ELEMENT, "id")?;
        let service_class = require_attr(node, ELEMENT, "serviceClass").ok();
        let start = require_attr(node, ELEMENT, "start").ok();
        let duration = require_attr(node, ELEMENT, "duration").ok();
        let content_ingest_method = match require_attr(node, ELEMENT, "contentIngestMethod") {
            Ok(v) => Some(ContentIngestMethod::parse(&v)?),
            Err(_) => None,
        };
        let transmission_mode = match require_attr(node, ELEMENT, "transmissionMode") {
            Ok(v) => Some(TransmissionMode::parse(&v)?),
            Err(_) => None,
        };
        let transport_security = match require_attr(node, ELEMENT, "transportSecurity") {
            Ok(v) => Some(TransportSecurity::parse(&v)?),
            Err(_) => None,
        };
        let session_idle_timeout = req_attr_u64(node, ELEMENT, "sessionIdleTimeout")?;

        let mut transport_protocol = None;
        let mut bit_rate = None;
        let mut endpoints = Vec::new();
        let mut fec_params = Vec::new();
        let mut unicast_repair = None;
        let mut object_carousel = None;
        let mut service_component_ids = Vec::new();
        for_each_child(ev, node, |ev, child| {
            if child.is(TRANSPORT_PROTOCOL_ELEMENT) && transport_protocol.is_none() {
                transport_protocol = Some(TransportProtocol::parse(ev, child)?);
            } else if child.is(BIT_RATE_ELEMENT) && bit_rate.is_none() {
                bit_rate = Some(BitRate::parse(ev, child)?);
            } else if child.is(ENDPOINT_ELEMENT) {
                endpoints.push(EndpointAddress::parse(ev, child)?);
            } else if child.is("ForwardErrorCorrectionParameters") {
                fec_params.push(ForwardErrorCorrectionParameters::parse(ev, child)?);
            } else if child.is("UnicastRepairParameters") && unicast_repair.is_none() {
                unicast_repair = Some(UnicastRepairParameters::parse(ev, child)?);
            } else if child.is("ObjectCarousel") && object_carousel.is_none() {
                object_carousel = Some(ObjectCarousel::parse(ev, child)?);
            } else if child.is("ServiceComponentIdentifier") {
                service_component_ids.push(ServiceComponentIdentifier::parse(ev, child)?);
            } else {
                return Ok(false);
            }
            Ok(true)
        })?;

        Ok(MulticastTransportSession {
            id,
            service_class,
            start,
            duration,
            content_ingest_method,
            transmission_mode,
            transport_security,
            session_idle_timeout,
            transport_protocol: transport_protocol
                .ok_or_else(|| missing_element(ELEMENT, TRANSPORT_PROTOCOL_ELEMENT))?,
            endpoints,
            bit_rate: bit_rate.ok_or_else(|| missing_element(ELEMENT, BIT_RATE_ELEMENT))?,
            fec_params,
            unicast_repair,
            object_carousel,
            service_component_ids,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(ELEMENT);
        attr(&mut t, "id", &self.id);
        opt_attr(&mut t, "serviceClass", self.service_class.as_deref());
        opt_attr(&mut t, "start", self.start.as_deref());
        opt_attr(&mut t, "duration", self.duration.as_deref());
        if let Some(m) = self.content_ingest_method {
            attr(&mut t, "contentIngestMethod", m.name());
        }
        if let Some(m) = self.transmission_mode {
            attr(&mut t, "transmissionMode", m.name());
        }
        if let Some(s) = self.transport_security {
            attr(&mut t, "transportSecurity", s.name());
        }
        num_attr(&mut t, "sessionIdleTimeout", self.session_idle_timeout);
        element(w, t, |w| {
            self.transport_protocol.write_xml(w)?;
            for ep in &self.endpoints {
                ep.write_xml(w)?;
            }
            self.bit_rate.write_xml(w)?;
            for fp in &self.fec_params {
                fp.write_xml(w)?;
            }
            if let Some(ur) = &self.unicast_repair {
                ur.write_xml(w)?;
            }
            if let Some(oc) = &self.object_carousel {
                oc.write_xml(w)?;
            }
            for sc in &self.service_component_ids {
                sc.write_xml(w)?;
            }
            Ok(())
        })
    }
}
