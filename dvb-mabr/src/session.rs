//! `MulticastSession` and `PresentationManifestLocator` — ETSI TS 103 769
//! V1.2.1 clauses 10.2.2, 10.2.2.2.
//!
//! One `MulticastSession` groups all multicast transport sessions delivering
//! one linear service's components.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::error::Result;
use crate::parse::{child, children, own_text, require_attr};
use crate::reporting::MulticastGatewaySessionReporting;
use crate::serialize::{push_indent, write_attr, write_opt_attr};
use crate::transport::MulticastTransportSession;

const ELEMENT: &str = "MulticastSession";
const MANIFEST_LOCATOR_ELEMENT: &str = "PresentationManifestLocator";

/// `MulticastSession` (clause 10.2.2, Table 10.2.2.1-1).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MulticastSession {
    /// Unique service ID within the deployment (URI string).
    pub service_identifier: String,
    /// Delay applied to the presentation timeline exposed to playback, to
    /// allow for repair time (ISO 8601 duration); default `"PT0S"`.
    pub content_playback_availability_offset: Option<String>,
    /// One or more presentation-manifest locators (DASH MPD / HLS Master
    /// Playlist), 1..n.
    pub manifest_locators: Vec<PresentationManifestLocator>,
    /// Per-session reporting destinations, if any (document-root reporting
    /// in `config.rs` may also apply simultaneously).
    pub reporting: Option<MulticastGatewaySessionReporting>,
    /// Zero or more multicast transport sessions delivering this service's
    /// components.
    pub transport_sessions: Vec<MulticastTransportSession>,
}

/// `PresentationManifestLocator` (clause 10.2.2.2).
///
/// Element content semantics differ by document type: in a **server**
/// configuration it is the push/pull URL; in a **gateway** configuration it
/// is the unicast retrieval/repair URL, or empty (with
/// `content_playback_path_pattern` then mandatory non-empty) if that
/// reference point is not present in the deployment.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PresentationManifestLocator {
    /// Unique within the parent `MulticastSession`; cross-referenced by
    /// `ServiceComponentIdentifier/@manifestIdRef`.
    pub manifest_id: String,
    /// MPEG-7 mimeType, e.g. `application/dash+xml` or
    /// `application/vnd.apple.mpegURL`.
    pub content_type: String,
    /// Transport object URI to use when this manifest is carouselled
    /// in-band; unique in the document if present.
    pub transport_object_uri: Option<String>,
    /// Wildcard pattern matched against the request path at reference point
    /// `L`, letting the gateway associate an inbound manifest request with
    /// this session.
    pub content_playback_path_pattern: Option<String>,
    /// The manifest locator URL itself (element content); may be empty —
    /// see the struct doc.
    pub locator: String,
}

impl PresentationManifestLocator {
    fn parse(node: Node<'_, '_>) -> Result<Self> {
        Ok(PresentationManifestLocator {
            manifest_id: require_attr(node, MANIFEST_LOCATOR_ELEMENT, "manifestId")?,
            content_type: require_attr(node, MANIFEST_LOCATOR_ELEMENT, "contentType")?,
            transport_object_uri: require_attr(
                node,
                MANIFEST_LOCATOR_ELEMENT,
                "transportObjectURI",
            )
            .ok(),
            content_playback_path_pattern: require_attr(
                node,
                MANIFEST_LOCATOR_ELEMENT,
                "contentPlaybackPathPattern",
            )
            .ok(),
            locator: own_text(node),
        })
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<PresentationManifestLocator");
        write_attr(out, "manifestId", &self.manifest_id);
        write_attr(out, "contentType", &self.content_type);
        write_opt_attr(
            out,
            "transportObjectURI",
            self.transport_object_uri.as_deref(),
        );
        write_opt_attr(
            out,
            "contentPlaybackPathPattern",
            self.content_playback_path_pattern.as_deref(),
        );
        out.push('>');
        out.push_str(&crate::serialize::xml_escape(&self.locator));
        out.push_str("</PresentationManifestLocator>\n");
    }
}

impl MulticastSession {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let mut manifest_locators = Vec::new();
        for n in children(node, MANIFEST_LOCATOR_ELEMENT) {
            manifest_locators.push(PresentationManifestLocator::parse(n)?);
        }
        let mut transport_sessions = Vec::new();
        for n in children(node, "MulticastTransportSession") {
            transport_sessions.push(MulticastTransportSession::parse(n)?);
        }
        Ok(MulticastSession {
            service_identifier: require_attr(node, ELEMENT, "serviceIdentifier")?,
            content_playback_availability_offset: require_attr(
                node,
                ELEMENT,
                "contentPlaybackAvailabilityOffset",
            )
            .ok(),
            manifest_locators,
            reporting: match child(node, "MulticastGatewaySessionReporting") {
                Some(n) => Some(MulticastGatewaySessionReporting::parse(n)?),
                None => None,
            },
            transport_sessions,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<MulticastSession");
        write_attr(out, "serviceIdentifier", &self.service_identifier);
        write_opt_attr(
            out,
            "contentPlaybackAvailabilityOffset",
            self.content_playback_availability_offset.as_deref(),
        );
        out.push_str(">\n");
        for loc in &self.manifest_locators {
            loc.write_xml(out, indent + 1);
        }
        if let Some(rep) = &self.reporting {
            rep.write_xml(out, indent + 1);
        }
        for ts in &self.transport_sessions {
            ts.write_xml(out, indent + 1);
        }
        push_indent(out, indent);
        out.push_str("</MulticastSession>\n");
    }
}
