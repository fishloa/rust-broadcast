//! `MulticastGatewaySessionReporting` / `ReportingLocator` — ETSI TS 103 769
//! V1.2.1 clauses 10.2.1.0, 10.2.2.3.
//!
//! Declared at the document root (applies to all sessions) and/or per
//! `MulticastSession` (that session only); both may be active
//! simultaneously. The report body itself is a JSON document (clause 11.1)
//! — out of scope of this crate; see `docs/mabr-reporting.md`.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::error::Result;
use crate::parse::{children, opt_attr_bool, opt_attr_f64, own_text, req_attr_u64, require_attr};
use crate::serialize::{push_indent, write_num_attr, write_opt_bool_attr, write_opt_num_attr};

const LOCATOR_ELEMENT: &str = "ReportingLocator";

/// `MulticastGatewaySessionReporting` — one or more reporting destinations.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastGatewaySessionReporting {
    /// Reporting destinations, 1..n.
    pub locators: Vec<ReportingLocator>,
}

/// `ReportingLocator` (clause 10.2.1.0) — a single reporting destination.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ReportingLocator {
    /// The reporting endpoint URL (element content).
    pub uri: String,
    /// Sampled fraction of gateways that report to this endpoint, `(0.0,
    /// 1.0]`; default `1.0`.
    pub proportion: Option<f64>,
    /// Gap between periodic reports (ISO 8601 duration); `"PT0S"` disables
    /// periodic reporting (event-only).
    pub period: String,
    /// Extra random delay (ms) added after `period`.
    pub random_delay: u64,
    /// Whether "running" events (heartbeats etc.) are included; default
    /// `false`.
    pub report_session_running_events: Option<bool>,
}

impl ReportingLocator {
    fn parse(node: Node<'_, '_>) -> Result<Self> {
        Ok(ReportingLocator {
            uri: own_text(node),
            proportion: opt_attr_f64(node, LOCATOR_ELEMENT, "proportion")?,
            period: require_attr(node, LOCATOR_ELEMENT, "period")?,
            random_delay: req_attr_u64(node, LOCATOR_ELEMENT, "randomDelay")?,
            report_session_running_events: opt_attr_bool(
                node,
                LOCATOR_ELEMENT,
                "reportSessionRunningEvents",
            )?,
        })
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<ReportingLocator");
        write_opt_num_attr(out, "proportion", self.proportion);
        {
            use crate::serialize::write_attr;
            write_attr(out, "period", &self.period);
        }
        write_num_attr(out, "randomDelay", self.random_delay);
        write_opt_bool_attr(
            out,
            "reportSessionRunningEvents",
            self.report_session_running_events,
        );
        out.push('>');
        out.push_str(&crate::serialize::xml_escape(&self.uri));
        out.push_str("</ReportingLocator>\n");
    }
}

impl MulticastGatewaySessionReporting {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let mut locators = Vec::new();
        for n in children(node, LOCATOR_ELEMENT) {
            locators.push(ReportingLocator::parse(n)?);
        }
        Ok(MulticastGatewaySessionReporting { locators })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<MulticastGatewaySessionReporting>\n");
        for loc in &self.locators {
            loc.write_xml(out, indent + 1);
        }
        push_indent(out, indent);
        out.push_str("</MulticastGatewaySessionReporting>\n");
    }
}
