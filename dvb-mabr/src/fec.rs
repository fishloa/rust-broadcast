//! `ForwardErrorCorrectionParameters` — ETSI TS 103 769 V1.2.1 clause 10.2.3.11.
//!
//! Semantics of an **omitted** `ForwardErrorCorrectionParameters` element are
//! protocol-specific: for FLUTE it means Compact No-Code FEC is in use; for
//! ROUTE it means no Repair Flow protects the session (`mabr-transport.md`
//! §2.1/§3.1 in this crate's `docs/`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::error::Result;
use crate::parse::{children, own_text, parse_u32, require_child};
use crate::serialize::push_indent;
use crate::transport::EndpointAddress;

const ELEMENT: &str = "ForwardErrorCorrectionParameters";

/// AL-FEC parameters for one `MulticastTransportSession` (clause 10.2.3.11).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ForwardErrorCorrectionParameters {
    /// AL-FEC scheme, a `ForwardErrorCorrectionSchemeCS` term (Annex B.2) —
    /// an MPEG-7 term-reference URI, e.g.
    /// `urn:ietf:rmt:fec:encoding:6` (RaptorQ).
    pub scheme_identifier: String,
    /// FEC overhead vs. source packets: `20` = 20 %, `100` = one repair
    /// packet per source packet; values above 100 are permitted.
    pub overhead_percentage: u32,
    /// Only present if repair packets use a *different* endpoint than the
    /// source session's own `EndpointAddress` (clause 10.2.3.9).
    pub endpoints: Vec<EndpointAddress>,
}

impl ForwardErrorCorrectionParameters {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let scheme_id_node = require_child(node, ELEMENT, "SchemeIdentifier")?;
        let overhead_node = require_child(node, ELEMENT, "OverheadPercentage")?;
        let overhead_text = own_text(overhead_node);
        let mut endpoints = Vec::new();
        for ep in children(node, "EndpointAddress") {
            endpoints.push(EndpointAddress::parse(ep)?);
        }
        Ok(ForwardErrorCorrectionParameters {
            scheme_identifier: own_text(scheme_id_node),
            overhead_percentage: parse_u32(ELEMENT, "OverheadPercentage", &overhead_text)?,
            endpoints,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<ForwardErrorCorrectionParameters>\n");
        push_indent(out, indent + 1);
        out.push_str("<SchemeIdentifier>");
        out.push_str(&crate::serialize::xml_escape(&self.scheme_identifier));
        out.push_str("</SchemeIdentifier>\n");
        push_indent(out, indent + 1);
        out.push_str("<OverheadPercentage>");
        {
            use core::fmt::Write as _;
            let _ = write!(out, "{}", self.overhead_percentage);
        }
        out.push_str("</OverheadPercentage>\n");
        for ep in &self.endpoints {
            ep.write_xml(out, indent + 1);
        }
        push_indent(out, indent);
        out.push_str("</ForwardErrorCorrectionParameters>\n");
    }
}
