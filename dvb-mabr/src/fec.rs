//! `ForwardErrorCorrectionParameters` — ETSI TS 103 769 V1.2.1 clause 10.2.3.11.
//!
//! Semantics of an **omitted** `ForwardErrorCorrectionParameters` element are
//! protocol-specific: for FLUTE it means Compact No-Code FEC is in use; for
//! ROUTE it means no Repair Flow protects the session (`mabr-transport.md`
//! §2.1/§3.1 in this crate's `docs/`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use alloc::string::ToString;
use std::io;

use crate::error::Result;
use crate::parse::{Events, StartTag, for_each_child, missing_element, parse_u32};
use crate::serialize::{Out, element, tag, text_element};
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
    pub(crate) fn parse(ev: &mut Events<'_>, tag: &StartTag) -> Result<Self> {
        let mut scheme_identifier = None;
        let mut overhead_text = None;
        let mut endpoints = Vec::new();
        for_each_child(ev, tag, |ev, child| {
            if child.is("SchemeIdentifier") && scheme_identifier.is_none() {
                scheme_identifier = Some(ev.text(child)?);
            } else if child.is("OverheadPercentage") && overhead_text.is_none() {
                overhead_text = Some(ev.text(child)?);
            } else if child.is("EndpointAddress") {
                endpoints.push(EndpointAddress::parse(ev, child)?);
            } else {
                return Ok(false);
            }
            Ok(true)
        })?;
        let scheme_identifier =
            scheme_identifier.ok_or_else(|| missing_element(ELEMENT, "SchemeIdentifier"))?;
        let overhead_text =
            overhead_text.ok_or_else(|| missing_element(ELEMENT, "OverheadPercentage"))?;
        Ok(ForwardErrorCorrectionParameters {
            scheme_identifier,
            overhead_percentage: parse_u32(ELEMENT, "OverheadPercentage", &overhead_text)?,
            endpoints,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        element(w, tag(ELEMENT), |w| {
            text_element(w, tag("SchemeIdentifier"), &self.scheme_identifier)?;
            text_element(
                w,
                tag("OverheadPercentage"),
                &self.overhead_percentage.to_string(),
            )?;
            for ep in &self.endpoints {
                ep.write_xml(w)?;
            }
            Ok(())
        })
    }
}
