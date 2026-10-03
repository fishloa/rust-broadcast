//! `MulticastSession` and `PresentationManifestLocator` — ETSI TS 103 769
//! V1.2.1 clauses 10.2.2, 10.2.2.2.
//!
//! One `MulticastSession` groups all multicast transport sessions delivering
//! one linear service's components.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use std::io;

use crate::error::Result;
use crate::parse::{Events, StartTag, for_each_child, require_attr};
use crate::reporting::MulticastGatewaySessionReporting;
use crate::serialize::{Out, attr, element, opt_attr, tag, text_element};
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
    fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let manifest_id = require_attr(node, MANIFEST_LOCATOR_ELEMENT, "manifestId")?;
        let content_type = require_attr(node, MANIFEST_LOCATOR_ELEMENT, "contentType")?;
        let transport_object_uri =
            require_attr(node, MANIFEST_LOCATOR_ELEMENT, "transportObjectURI").ok();
        let content_playback_path_pattern =
            require_attr(node, MANIFEST_LOCATOR_ELEMENT, "contentPlaybackPathPattern").ok();
        Ok(PresentationManifestLocator {
            manifest_id,
            content_type,
            transport_object_uri,
            content_playback_path_pattern,
            locator: ev.text(node)?,
        })
    }

    fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(MANIFEST_LOCATOR_ELEMENT);
        attr(&mut t, "manifestId", &self.manifest_id);
        attr(&mut t, "contentType", &self.content_type);
        opt_attr(
            &mut t,
            "transportObjectURI",
            self.transport_object_uri.as_deref(),
        );
        opt_attr(
            &mut t,
            "contentPlaybackPathPattern",
            self.content_playback_path_pattern.as_deref(),
        );
        text_element(w, t, &self.locator)
    }
}

impl MulticastSession {
    pub(crate) fn parse(ev: &mut Events<'_>, node: &StartTag) -> Result<Self> {
        let service_identifier = require_attr(node, ELEMENT, "serviceIdentifier")?;
        let content_playback_availability_offset =
            require_attr(node, ELEMENT, "contentPlaybackAvailabilityOffset").ok();
        let mut manifest_locators = Vec::new();
        let mut reporting = None;
        let mut transport_sessions = Vec::new();
        for_each_child(ev, node, |ev, child| {
            if child.is(MANIFEST_LOCATOR_ELEMENT) {
                manifest_locators.push(PresentationManifestLocator::parse(ev, child)?);
            } else if child.is("MulticastGatewaySessionReporting") && reporting.is_none() {
                reporting = Some(MulticastGatewaySessionReporting::parse(ev, child)?);
            } else if child.is("MulticastTransportSession") {
                transport_sessions.push(MulticastTransportSession::parse(ev, child)?);
            } else {
                return Ok(false);
            }
            Ok(true)
        })?;
        Ok(MulticastSession {
            service_identifier,
            content_playback_availability_offset,
            manifest_locators,
            reporting,
            transport_sessions,
        })
    }

    pub(crate) fn write_xml(&self, w: &mut Out) -> io::Result<()> {
        let mut t = tag(ELEMENT);
        attr(&mut t, "serviceIdentifier", &self.service_identifier);
        opt_attr(
            &mut t,
            "contentPlaybackAvailabilityOffset",
            self.content_playback_availability_offset.as_deref(),
        );
        element(w, t, |w| {
            for loc in &self.manifest_locators {
                loc.write_xml(w)?;
            }
            if let Some(rep) = &self.reporting {
                rep.write_xml(w)?;
            }
            for ts in &self.transport_sessions {
                ts.write_xml(w)?;
            }
            Ok(())
        })
    }
}
