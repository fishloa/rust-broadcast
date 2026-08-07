//! Root document elements — ETSI TS 103 769 V1.2.1 clause 10.2.1 (Tables
//! 10.2.1.1-1, 10.2.1.2-1).
//!
//! Two flavours share the same schema and clause numbering: a Multicast
//! server configuration (root `MulticastServerConfiguration`, clause
//! 10.2.1.1) sent to the Multicast server at reference point `CMS`, and a
//! Multicast gateway configuration (root `MulticastGatewayConfiguration`,
//! clause 10.2.1.2) sent to the Multicast gateway at reference point `CMR`
//! (or piggybacked at `B`/`M`).

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use roxmltree::Document;

use crate::error::{Error, Result};
use crate::gateway::{ConfigurationMacro, MulticastGatewayConfigurationTransportSession};
use crate::parse::{child, children, require_attr};
use crate::reporting::MulticastGatewaySessionReporting;
use crate::serialize::{write_attr, write_opt_attr};
use crate::session::MulticastSession;

const ROOT_SERVER: &str = "MulticastServerConfiguration";
const ROOT_GATEWAY: &str = "MulticastGatewayConfiguration";
const SERVER_MACRO_ELEMENT: &str = "MulticastServerConfigurationMacro";

/// The fields common to both document roots (clause 10.2.1).
struct CommonRoot {
    schema_version: u32,
    validity_period: Option<String>,
    valid_until: Option<String>,
    gateway_config_transport_sessions: Vec<MulticastGatewayConfigurationTransportSession>,
    sessions: Vec<MulticastSession>,
    reporting: Option<MulticastGatewaySessionReporting>,
}

fn parse_common_root(node: roxmltree::Node<'_, '_>, element: &'static str) -> Result<CommonRoot> {
    let mut gateway_config_transport_sessions = Vec::new();
    for n in children(node, "MulticastGatewayConfigurationTransportSession") {
        gateway_config_transport_sessions
            .push(MulticastGatewayConfigurationTransportSession::parse(n)?);
    }
    let mut sessions = Vec::new();
    for n in children(node, "MulticastSession") {
        sessions.push(MulticastSession::parse(n)?);
    }
    Ok(CommonRoot {
        schema_version: crate::parse::req_attr_u32(node, element, "schemaVersion")?,
        validity_period: require_attr(node, element, "validityPeriod").ok(),
        valid_until: require_attr(node, element, "validUntil").ok(),
        gateway_config_transport_sessions,
        sessions,
        reporting: match child(node, "MulticastGatewaySessionReporting") {
            Some(n) => Some(MulticastGatewaySessionReporting::parse(n)?),
            None => None,
        },
    })
}

fn write_common_root(
    out: &mut String,
    schema_version: u32,
    validity_period: &Option<String>,
    valid_until: &Option<String>,
) {
    write_attr(out, "schemaVersion", &schema_version.to_string());
    write_opt_attr(out, "validityPeriod", validity_period.as_deref());
    write_opt_attr(out, "validUntil", valid_until.as_deref());
}

fn write_common_body(
    out: &mut String,
    gateway_config_transport_sessions: &[MulticastGatewayConfigurationTransportSession],
    sessions: &[MulticastSession],
    reporting: &Option<MulticastGatewaySessionReporting>,
    indent: usize,
) {
    for ts in gateway_config_transport_sessions {
        ts.write_xml(out, indent);
    }
    for s in sessions {
        s.write_xml(out, indent);
    }
    if let Some(r) = reporting {
        r.write_xml(out, indent);
    }
}

/// `MulticastServerConfiguration` — the root of a Multicast server
/// configuration document (clause 10.2.1.1, Table 10.2.1.1-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastServerConfiguration {
    /// Schema version (Annex A.0); current baseline value `2`.
    pub schema_version: u32,
    /// Relative expiry (ISO 8601 duration).
    pub validity_period: Option<String>,
    /// Absolute expiry (MPEG-7 `TimePoint`). If both `validity_period` and
    /// `valid_until` are present, the later expiry wins.
    pub valid_until: Option<String>,
    /// In-band gateway-configuration carousel sessions (clause 10.2.5).
    pub gateway_config_transport_sessions: Vec<MulticastGatewayConfigurationTransportSession>,
    /// The linear services this server configuration describes.
    pub sessions: Vec<MulticastSession>,
    /// Document-wide reporting destinations (all sessions); a per-session
    /// `MulticastGatewaySessionReporting` may also apply simultaneously.
    pub reporting: Option<MulticastGatewaySessionReporting>,
    /// Macro-expansion values (clause 10.2.5.2) — server-configuration only.
    pub macros: Vec<ConfigurationMacro>,
}

impl MulticastServerConfiguration {
    /// Parse a `MulticastServerConfiguration` XML document.
    pub fn parse_str(xml: &str) -> Result<Self> {
        let doc = Document::parse(xml).map_err(|e| Error::XmlParse(e.to_string()))?;
        let root = doc.root_element();
        if root.tag_name().name() != ROOT_SERVER {
            return Err(Error::UnexpectedRoot(root.tag_name().name().into()));
        }
        let common = parse_common_root(root, ROOT_SERVER)?;
        let mut macros = Vec::new();
        for n in children(root, SERVER_MACRO_ELEMENT) {
            macros.push(ConfigurationMacro::parse(n, SERVER_MACRO_ELEMENT)?);
        }
        Ok(MulticastServerConfiguration {
            schema_version: common.schema_version,
            validity_period: common.validity_period,
            valid_until: common.valid_until,
            gateway_config_transport_sessions: common.gateway_config_transport_sessions,
            sessions: common.sessions,
            reporting: common.reporting,
            macros,
        })
    }

    /// Serialize back to a well-formed XML document (structural round-trip;
    /// see the crate root doc for what is and isn't preserved).
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push('<');
        out.push_str(ROOT_SERVER);
        write_attr(
            &mut out,
            "xmlns",
            crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2024,
        );
        write_attr(&mut out, "xmlns:xsi", crate::parse::NS_XSI);
        write_common_root(
            &mut out,
            self.schema_version,
            &self.validity_period,
            &self.valid_until,
        );
        out.push_str(">\n");
        write_common_body(
            &mut out,
            &self.gateway_config_transport_sessions,
            &self.sessions,
            &self.reporting,
            1,
        );
        for m in &self.macros {
            m.write_xml(&mut out, 1, SERVER_MACRO_ELEMENT);
        }
        out.push_str("</");
        out.push_str(ROOT_SERVER);
        out.push_str(">\n");
        out
    }
}

/// `MulticastGatewayConfiguration` — the root of a Multicast gateway
/// configuration document (clause 10.2.1.2, Table 10.2.1.2-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastGatewayConfiguration {
    /// Schema version (Annex A.0); current baseline value `2`.
    pub schema_version: u32,
    /// Relative expiry (ISO 8601 duration). A document delivered via the
    /// in-band carousel method must not carry this.
    pub validity_period: Option<String>,
    /// Absolute expiry (MPEG-7 `TimePoint`).
    pub valid_until: Option<String>,
    /// In-band gateway-configuration carousel sessions (clause 10.2.5) — a
    /// "bootstrap" document (Annex C.2) carries only these, with no
    /// `sessions`.
    pub gateway_config_transport_sessions: Vec<MulticastGatewayConfigurationTransportSession>,
    /// The linear services this gateway configuration describes.
    pub sessions: Vec<MulticastSession>,
    /// Document-wide reporting destinations.
    pub reporting: Option<MulticastGatewaySessionReporting>,
}

impl MulticastGatewayConfiguration {
    /// Parse a `MulticastGatewayConfiguration` XML document.
    pub fn parse_str(xml: &str) -> Result<Self> {
        let doc = Document::parse(xml).map_err(|e| Error::XmlParse(e.to_string()))?;
        let root = doc.root_element();
        if root.tag_name().name() != ROOT_GATEWAY {
            return Err(Error::UnexpectedRoot(root.tag_name().name().into()));
        }
        let common = parse_common_root(root, ROOT_GATEWAY)?;
        Ok(MulticastGatewayConfiguration {
            schema_version: common.schema_version,
            validity_period: common.validity_period,
            valid_until: common.valid_until,
            gateway_config_transport_sessions: common.gateway_config_transport_sessions,
            sessions: common.sessions,
            reporting: common.reporting,
        })
    }

    /// Serialize back to a well-formed XML document (structural round-trip;
    /// see the crate root doc for what is and isn't preserved).
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push('<');
        out.push_str(ROOT_GATEWAY);
        write_attr(
            &mut out,
            "xmlns",
            crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2024,
        );
        write_attr(&mut out, "xmlns:xsi", crate::parse::NS_XSI);
        write_common_root(
            &mut out,
            self.schema_version,
            &self.validity_period,
            &self.valid_until,
        );
        out.push_str(">\n");
        write_common_body(
            &mut out,
            &self.gateway_config_transport_sessions,
            &self.sessions,
            &self.reporting,
            1,
        );
        out.push_str("</");
        out.push_str(ROOT_GATEWAY);
        out.push_str(">\n");
        out
    }
}
