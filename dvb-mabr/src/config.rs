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

use alloc::string::String;
use alloc::vec::Vec;

use std::io;

use quick_xml::events::BytesStart;

use crate::error::{Error, Result};
use crate::gateway::{ConfigurationMacro, MulticastGatewayConfigurationTransportSession};
use crate::parse::{Events, StartTag, for_each_child, is_baseline_namespace, require_attr};
use crate::reporting::MulticastGatewaySessionReporting;
use crate::serialize::{Out, attr, document, element, num_attr, opt_attr, tag};
use crate::session::MulticastSession;

const ROOT_SERVER: &str = "MulticastServerConfiguration";
const ROOT_GATEWAY: &str = "MulticastGatewayConfiguration";
const SERVER_MACRO_ELEMENT: &str = "MulticastServerConfigurationMacro";

/// The fields common to both document roots (clause 10.2.1).
struct CommonRoot {
    schema_version: u32,
    namespace: BaselineNamespace,
    validity_period: Option<String>,
    valid_until: Option<String>,
    gateway_config_transport_sessions: Vec<MulticastGatewayConfigurationTransportSession>,
    sessions: Vec<MulticastSession>,
    reporting: Option<MulticastGatewaySessionReporting>,
}

/// The baseline `MulticastSessionConfiguration` namespace a document declares
/// on its root (Annex A Table A.0-1): the 2019 namespace goes with
/// `schemaVersion="1"`, the 2024 namespace with the current version.
///
/// It is recorded at parse time and re-emitted verbatim by `to_xml`, instead
/// of being derived from `schema_version`: a `schemaVersion="2"` document that
/// still declares the 2019 namespace would otherwise be silently re-serialized
/// under 2024, so the round trip would not be byte-stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum BaselineNamespace {
    /// `urn:dvb:metadata:MulticastSessionConfiguration:2019`.
    V2019,
    /// `urn:dvb:metadata:MulticastSessionConfiguration:2024` (current).
    #[default]
    V2024,
}

impl BaselineNamespace {
    /// The namespace URI.
    pub fn uri(&self) -> &'static str {
        match self {
            BaselineNamespace::V2019 => crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2019,
            BaselineNamespace::V2024 => crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2024,
        }
    }

    /// A short label (`"2019"` / `"2024"`).
    pub fn name(&self) -> &'static str {
        match self {
            BaselineNamespace::V2019 => "2019",
            BaselineNamespace::V2024 => "2024",
        }
    }

    /// Classify a root element's namespace URI; `None` for a non-baseline one.
    fn from_uri(uri: Option<&str>) -> Option<Self> {
        match uri {
            Some(crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2019) => Some(Self::V2019),
            Some(crate::parse::NS_MULTICAST_SESSION_CONFIGURATION_2024) => Some(Self::V2024),
            _ => None,
        }
    }
}

broadcast_common::impl_spec_display!(BaselineNamespace);

/// Reject a root element that isn't `expected` in a recognized MABR baseline
/// namespace (2019 or 2024). Previously only the local name was checked, so
/// a `<MulticastServerConfiguration>` in any other (or no) namespace was
/// silently accepted (MABR-W1, #1121).
fn check_root(root: &StartTag, expected: &'static str) -> Result<()> {
    if root.local() != expected || !is_baseline_namespace(root.namespace()) {
        return Err(Error::UnexpectedRoot(alloc::format!(
            "{} (namespace {:?})",
            root.local(),
            root.namespace()
        )));
    }
    Ok(())
}

fn parse_common_root(
    ev: &mut Events<'_>,
    node: &StartTag,
    element: &'static str,
    mut on_extra: impl FnMut(&mut Events<'_>, &StartTag) -> Result<bool>,
) -> Result<CommonRoot> {
    let schema_version = crate::parse::req_attr_u32(node, element, "schemaVersion")?;
    let namespace = BaselineNamespace::from_uri(node.namespace()).ok_or_else(|| {
        Error::UnexpectedRoot(alloc::format!(
            "{} (namespace {:?})",
            node.local(),
            node.namespace()
        ))
    })?;
    let validity_period = require_attr(node, element, "validityPeriod").ok();
    let valid_until = require_attr(node, element, "validUntil").ok();
    let mut gateway_config_transport_sessions = Vec::new();
    let mut sessions = Vec::new();
    let mut reporting = None;
    for_each_child(ev, node, |ev, child| {
        if child.is("MulticastGatewayConfigurationTransportSession") {
            gateway_config_transport_sessions.push(
                MulticastGatewayConfigurationTransportSession::parse(ev, child)?,
            );
        } else if child.is("MulticastSession") {
            sessions.push(MulticastSession::parse(ev, child)?);
        } else if child.is("MulticastGatewaySessionReporting") && reporting.is_none() {
            reporting = Some(MulticastGatewaySessionReporting::parse(ev, child)?);
        } else {
            return on_extra(ev, child);
        }
        Ok(true)
    })?;
    Ok(CommonRoot {
        schema_version,
        namespace,
        validity_period,
        valid_until,
        gateway_config_transport_sessions,
        sessions,
        reporting,
    })
}

/// Parse a whole document: the root element via `parse_root`, then the rest of
/// the input. A well-formedness error anywhere in the document takes
/// precedence over a semantic error found earlier in it.
fn parse_document<T>(
    xml: &str,
    parse_root: impl FnOnce(&mut Events<'_>, &StartTag) -> Result<T>,
) -> Result<T> {
    let mut ev = Events::new(xml);
    let root = ev.root()?;
    match parse_root(&mut ev, &root) {
        Ok(parsed) => {
            ev.finish()?;
            Ok(parsed)
        }
        Err(e) => {
            // Surface a syntax error later in the document before a semantic one.
            ev.drain()?;
            Err(e)
        }
    }
}

fn write_common_root(
    t: &mut BytesStart<'_>,
    schema_version: u32,
    validity_period: &Option<String>,
    valid_until: &Option<String>,
) {
    num_attr(t, "schemaVersion", schema_version);
    opt_attr(t, "validityPeriod", validity_period.as_deref());
    opt_attr(t, "validUntil", valid_until.as_deref());
}

fn write_common_body(
    w: &mut Out,
    gateway_config_transport_sessions: &[MulticastGatewayConfigurationTransportSession],
    sessions: &[MulticastSession],
    reporting: &Option<MulticastGatewaySessionReporting>,
) -> io::Result<()> {
    for ts in gateway_config_transport_sessions {
        ts.write_xml(w)?;
    }
    for s in sessions {
        s.write_xml(w)?;
    }
    if let Some(r) = reporting {
        r.write_xml(w)?;
    }
    Ok(())
}

/// `MulticastServerConfiguration` — the root of a Multicast server
/// configuration document (clause 10.2.1.1, Table 10.2.1.1-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastServerConfiguration {
    /// Schema version (Annex A.0); current baseline value `2`.
    pub schema_version: u32,
    /// The baseline namespace the root declared, re-emitted as-is by `to_xml`.
    pub namespace: BaselineNamespace,
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
        parse_document(xml, |ev, root| {
            check_root(root, ROOT_SERVER)?;
            let mut macros = Vec::new();
            let common = parse_common_root(ev, root, ROOT_SERVER, |ev, child| {
                if child.is(SERVER_MACRO_ELEMENT) {
                    macros.push(ConfigurationMacro::parse(ev, child, SERVER_MACRO_ELEMENT)?);
                    Ok(true)
                } else {
                    Ok(false)
                }
            })?;
            Ok(MulticastServerConfiguration {
                schema_version: common.schema_version,
                namespace: common.namespace,
                validity_period: common.validity_period,
                valid_until: common.valid_until,
                gateway_config_transport_sessions: common.gateway_config_transport_sessions,
                sessions: common.sessions,
                reporting: common.reporting,
                macros,
            })
        })
    }

    /// Serialize back to a well-formed XML document (structural round-trip;
    /// see the crate root doc for what is and isn't preserved).
    pub fn to_xml(&self) -> String {
        document(|w| {
            let mut t = tag(ROOT_SERVER);
            attr(&mut t, "xmlns", self.namespace.uri());
            attr(&mut t, "xmlns:xsi", crate::parse::NS_XSI);
            write_common_root(
                &mut t,
                self.schema_version,
                &self.validity_period,
                &self.valid_until,
            );
            element(w, t, |w| {
                write_common_body(
                    w,
                    &self.gateway_config_transport_sessions,
                    &self.sessions,
                    &self.reporting,
                )?;
                for m in &self.macros {
                    m.write_xml(w, SERVER_MACRO_ELEMENT)?;
                }
                Ok(())
            })
        })
    }
}

/// `MulticastGatewayConfiguration` — the root of a Multicast gateway
/// configuration document (clause 10.2.1.2, Table 10.2.1.2-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastGatewayConfiguration {
    /// Schema version (Annex A.0); current baseline value `2`.
    pub schema_version: u32,
    /// The baseline namespace the root declared, re-emitted as-is by `to_xml`.
    pub namespace: BaselineNamespace,
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
        parse_document(xml, |ev, root| {
            check_root(root, ROOT_GATEWAY)?;
            let common = parse_common_root(ev, root, ROOT_GATEWAY, |_, _| Ok(false))?;
            Ok(MulticastGatewayConfiguration {
                schema_version: common.schema_version,
                namespace: common.namespace,
                validity_period: common.validity_period,
                valid_until: common.valid_until,
                gateway_config_transport_sessions: common.gateway_config_transport_sessions,
                sessions: common.sessions,
                reporting: common.reporting,
            })
        })
    }

    /// Serialize back to a well-formed XML document (structural round-trip;
    /// see the crate root doc for what is and isn't preserved).
    pub fn to_xml(&self) -> String {
        document(|w| {
            let mut t = tag(ROOT_GATEWAY);
            attr(&mut t, "xmlns", self.namespace.uri());
            attr(&mut t, "xmlns:xsi", crate::parse::NS_XSI);
            write_common_root(
                &mut t,
                self.schema_version,
                &self.validity_period,
                &self.valid_until,
            );
            element(w, t, |w| {
                write_common_body(
                    w,
                    &self.gateway_config_transport_sessions,
                    &self.sessions,
                    &self.reporting,
                )
            })
        })
    }
}
