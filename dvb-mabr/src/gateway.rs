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

use std::io;

use crate::carousel::ObjectCarousel;
use crate::error::Result;
use crate::fec::ForwardErrorCorrectionParameters;
use crate::parse::{Events, StartTag, for_each_child, missing_element, opt_attr_u64, require_attr};
use crate::repair::UnicastRepairParameters;
use crate::serialize::{Out, attr, element, opt_attr, opt_num_attr, tag, text_element};
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
    pub(crate) fn parse(
        ev: &mut Events<'_>,
        node: &StartTag,
        element: &'static str,
    ) -> Result<Self> {
        let key = require_attr(node, element, "key")?;
        Ok(ConfigurationMacro {
            key,
            value: ev.text(node)?,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out, element: &str) -> io::Result<()> {
        let mut t = tag(element);
        attr(&mut t, "key", &self.key);
        text_element(w, t, &self.value)
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
    /// as inactive/unsubscribe. Optional — not present on
    /// `MulticastGatewayConfigurationTransportSession` per Table 10.2.5.1-1
    /// (only `MulticastTransportSession` carries it mandatorily).
    pub session_idle_timeout: Option<u64>,
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
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let service_class = require_attr(node, ELEMENT, "serviceClass").ok();
        let transport_security = match require_attr(node, ELEMENT, "transportSecurity") {
            Ok(v) => Some(TransportSecurity::parse(&v)?),
            Err(_) => None,
        };
        let session_idle_timeout = opt_attr_u64(node, ELEMENT, "sessionIdleTimeout")?;
        let tags: Vec<String> = require_attr(node, ELEMENT, "tags")
            .map(|t| t.split_whitespace().map(ToString::to_string).collect())
            .unwrap_or_default();

        let mut transport_protocol = None;
        let mut bit_rate = None;
        let mut endpoints = Vec::new();
        let mut fec_params = Vec::new();
        let mut unicast_repair = None;
        let mut object_carousel = None;
        let mut macros = Vec::new();
        for_each_child(ev, node, |ev, child| {
            if child.is("TransportProtocol") && transport_protocol.is_none() {
                transport_protocol = Some(TransportProtocol::parse(ev, child)?);
            } else if child.is("BitRate") && bit_rate.is_none() {
                bit_rate = Some(BitRate::parse(ev, child)?);
            } else if child.is("EndpointAddress") {
                endpoints.push(EndpointAddress::parse(ev, child)?);
            } else if child.is("ForwardErrorCorrectionParameters") {
                fec_params.push(ForwardErrorCorrectionParameters::parse(ev, child)?);
            } else if child.is("UnicastRepairParameters") && unicast_repair.is_none() {
                unicast_repair = Some(UnicastRepairParameters::parse(ev, child)?);
            } else if child.is("ObjectCarousel") && object_carousel.is_none() {
                object_carousel = Some(ObjectCarousel::parse(ev, child)?);
            } else if child.is(MACRO_ELEMENT) {
                macros.push(ConfigurationMacro::parse(ev, child, MACRO_ELEMENT)?);
            } else {
                return Ok(false);
            }
            Ok(true)
        })?;

        Ok(MulticastGatewayConfigurationTransportSession {
            service_class,
            transport_security,
            session_idle_timeout,
            transport_protocol: transport_protocol
                .ok_or_else(|| missing_element(ELEMENT, "TransportProtocol"))?,
            endpoints,
            bit_rate: bit_rate.ok_or_else(|| missing_element(ELEMENT, "BitRate"))?,
            fec_params,
            unicast_repair,
            object_carousel,
            tags,
            macros,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(ELEMENT);
        opt_attr(&mut t, "serviceClass", self.service_class.as_deref());
        if let Some(s) = self.transport_security {
            attr(&mut t, "transportSecurity", s.name());
        }
        opt_num_attr(&mut t, "sessionIdleTimeout", self.session_idle_timeout);
        if !self.tags.is_empty() {
            attr(&mut t, "tags", &self.tags.join(" "));
        }
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
            for m in &self.macros {
                m.write_xml(w, MACRO_ELEMENT)?;
            }
            Ok(())
        })
    }
}
