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

use std::io;

use crate::error::{Error, Result};
use crate::parse::{
    Events, StartTag, for_each_child, opt_attr_bool, opt_attr_f64, req_attr_u64, require_attr,
};
use crate::serialize::{
    Out, attr, element, num_attr, opt_bool_attr, opt_num_attr, tag, text_element,
};

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
    fn parse(ev: &mut Events<'_>, tag: &StartTag) -> Result<Self> {
        let proportion = opt_attr_f64(tag, LOCATOR_ELEMENT, "proportion")?;
        // Documented range `(0.0, 1.0]` (clause 10.2.1.0), never checked
        // (MABR-W3, #1121): `parse_f64` already rejects NaN/infinity, but not
        // an out-of-range finite value like `0.0` or `1.5`.
        if let Some(p) = proportion
            && !(p > 0.0 && p <= 1.0)
        {
            return Err(Error::InvalidAttribute {
                element: LOCATOR_ELEMENT,
                attr: "proportion",
                value: alloc::format!("{p}"),
                reason: "must be in the range (0.0, 1.0]",
            });
        }
        let period = require_attr(tag, LOCATOR_ELEMENT, "period")?;
        let random_delay = req_attr_u64(tag, LOCATOR_ELEMENT, "randomDelay")?;
        let report_session_running_events =
            opt_attr_bool(tag, LOCATOR_ELEMENT, "reportSessionRunningEvents")?;
        Ok(ReportingLocator {
            uri: ev.text(tag)?,
            proportion,
            period,
            random_delay,
            report_session_running_events,
        })
    }

    fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(LOCATOR_ELEMENT);
        opt_num_attr(&mut t, "proportion", self.proportion);
        attr(&mut t, "period", &self.period);
        num_attr(&mut t, "randomDelay", self.random_delay);
        opt_bool_attr(
            &mut t,
            "reportSessionRunningEvents",
            self.report_session_running_events,
        );
        text_element(w, t, &self.uri)
    }
}

impl MulticastGatewaySessionReporting {
    pub(crate) fn parse(ev: &mut Events<'_>, tag: &StartTag) -> Result<Self> {
        let mut locators = Vec::new();
        for_each_child(ev, tag, |ev, child| {
            if !child.is(LOCATOR_ELEMENT) {
                return Ok(false);
            }
            locators.push(ReportingLocator::parse(ev, child)?);
            Ok(true)
        })?;
        Ok(MulticastGatewaySessionReporting { locators })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        element(w, tag("MulticastGatewaySessionReporting"), |w| {
            for loc in &self.locators {
                loc.write_xml(w)?;
            }
            Ok(())
        })
    }
}
