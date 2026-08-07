//! `UnicastRepairParameters` and `BaseURL` — ETSI TS 103 769 V1.2.1 clauses
//! 10.2.3.12-10.2.3.13.
//!
//! If no `BaseURL` is present, the repair URL is built directly per the
//! (protocol-specific) construction rules in `docs/mabr-transport.md` §5.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::error::Result;
use crate::parse::{children, opt_attr_u32, opt_attr_u64, own_text, req_attr_u64, require_attr};
use crate::serialize::{push_indent, write_opt_num_attr};

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
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let mut base_urls = Vec::new();
        for bu in children(node, BASE_URL_ELEMENT) {
            base_urls.push(BaseUrl {
                uri: own_text(bu),
                relative_weight: opt_attr_u32(bu, BASE_URL_ELEMENT, "relativeWeight")?,
            });
        }
        Ok(UnicastRepairParameters {
            transport_object_base_uri: require_attr(node, ELEMENT, "transportObjectBaseURI").ok(),
            transport_object_reception_timeout: req_attr_u64(
                node,
                ELEMENT,
                "transportObjectReceptionTimeout",
            )?,
            fixed_back_off_period: opt_attr_u64(node, ELEMENT, "fixedBackOffPeriod")?,
            random_back_off_period: opt_attr_u64(node, ELEMENT, "randomBackOffPeriod")?,
            base_urls,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<UnicastRepairParameters");
        crate::serialize::write_opt_attr(
            out,
            "transportObjectBaseURI",
            self.transport_object_base_uri.as_deref(),
        );
        crate::serialize::write_num_attr(
            out,
            "transportObjectReceptionTimeout",
            self.transport_object_reception_timeout,
        );
        write_opt_num_attr(out, "fixedBackOffPeriod", self.fixed_back_off_period);
        write_opt_num_attr(out, "randomBackOffPeriod", self.random_back_off_period);
        if self.base_urls.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push_str(">\n");
        for bu in &self.base_urls {
            push_indent(out, indent + 1);
            out.push_str("<BaseURL");
            write_opt_num_attr(out, "relativeWeight", bu.relative_weight);
            out.push('>');
            out.push_str(&crate::serialize::xml_escape(&bu.uri));
            out.push_str("</BaseURL>\n");
        }
        push_indent(out, indent);
        out.push_str("</UnicastRepairParameters>\n");
    }
}
