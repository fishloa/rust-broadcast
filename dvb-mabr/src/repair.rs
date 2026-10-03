//! `UnicastRepairParameters` and `BaseURL` — ETSI TS 103 769 V1.2.1 clauses
//! 10.2.3.12-10.2.3.13.
//!
//! If no `BaseURL` is present, the repair URL is built directly per the
//! (protocol-specific) construction rules in `docs/mabr-transport.md` §5.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use std::io;

use crate::error::Result;
use crate::parse::{
    Events, StartTag, for_each_child, opt_attr_u32, opt_attr_u64, req_attr_u64, require_attr,
};
use crate::serialize::{Out, element, empty, num_attr, opt_attr, opt_num_attr, tag, text_element};

const ELEMENT: &str = "UnicastRepairParameters";
const BASE_URL_ELEMENT: &str = "BaseURL";

/// Unicast repair configuration for one `MulticastTransportSession`
/// (clauses 10.2.3.12-10.2.3.13).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct UnicastRepairParameters {
    /// Prefix stripped from the transport object URI before repair-URL
    /// construction. Absolute URI, no query/fragment.
    pub transport_object_base_uri: Option<String>,
    /// Wait time (ms) before assuming object transmission is over.
    pub transport_object_reception_timeout: u64,
    /// Fixed delay (ms) before repair; default `0`.
    pub fixed_back_off_period: Option<u64>,
    /// Upper bound (ms) of an additional random per-object delay; default `0`.
    pub random_back_off_period: Option<u64>,
    /// Unicast repair endpoint prefixes, in document order.
    pub base_urls: Vec<BaseUrl>,
}

/// A single `BaseURL` candidate (clause 10.2.3.13).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BaseUrl {
    /// Absolute URI, no query/fragment.
    pub uri: String,
    /// Selection weight; `0` disables this `BaseURL`. Omitted on every
    /// `BaseURL` under the parent => equal weight.
    pub relative_weight: Option<u32>,
}

impl UnicastRepairParameters {
    pub(crate) fn parse(ev: &mut Events<'_>, tag: &StartTag) -> Result<Self> {
        let transport_object_base_uri = require_attr(tag, ELEMENT, "transportObjectBaseURI").ok();
        let transport_object_reception_timeout =
            req_attr_u64(tag, ELEMENT, "transportObjectReceptionTimeout")?;
        let fixed_back_off_period = opt_attr_u64(tag, ELEMENT, "fixedBackOffPeriod")?;
        let random_back_off_period = opt_attr_u64(tag, ELEMENT, "randomBackOffPeriod")?;
        let mut base_urls = Vec::new();
        for_each_child(ev, tag, |ev, child| {
            if !child.is(BASE_URL_ELEMENT) {
                return Ok(false);
            }
            let relative_weight = opt_attr_u32(child, BASE_URL_ELEMENT, "relativeWeight")?;
            base_urls.push(BaseUrl {
                uri: ev.text(child)?,
                relative_weight,
            });
            Ok(true)
        })?;
        Ok(UnicastRepairParameters {
            transport_object_base_uri,
            transport_object_reception_timeout,
            fixed_back_off_period,
            random_back_off_period,
            base_urls,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(ELEMENT);
        opt_attr(
            &mut t,
            "transportObjectBaseURI",
            self.transport_object_base_uri.as_deref(),
        );
        num_attr(
            &mut t,
            "transportObjectReceptionTimeout",
            self.transport_object_reception_timeout,
        );
        opt_num_attr(&mut t, "fixedBackOffPeriod", self.fixed_back_off_period);
        opt_num_attr(&mut t, "randomBackOffPeriod", self.random_back_off_period);
        if self.base_urls.is_empty() {
            return empty(w, t);
        }
        element(w, t, |w| {
            for bu in &self.base_urls {
                let mut b = tag(BASE_URL_ELEMENT);
                opt_num_attr(&mut b, "relativeWeight", bu.relative_weight);
                text_element(w, b, &bu.uri)?;
            }
            Ok(())
        })
    }
}
