//! `ObjectCarousel`, `PresentationManifests`, `InitSegments`,
//! `ResourceLocator` — ETSI TS 103 769 V1.2.1 clause 10.2.3.14.
//!
//! `@targetAcquisitionLatency` omitted on any of the three children means
//! "repeat as often as the session bit rate allows".
//!
//! **Simplification vs. the XSD**: clause 10.2.5 (§7 of `docs/mabr-signalling.md`)
//! defines a *second* carousel type, `ReferencingObjectCarouselType`, used only
//! by `MulticastGatewayConfigurationTransportSession` (`gateway.rs`). It
//! differs from the base `ObjectCarouselType` used here in two ways: (1)
//! `PresentationManifests`/`InitSegments` are `0..n` there vs. `0..1` here,
//! and (2) each carries two extra reference attributes, `@serviceIdRef` /
//! `@transportSessionIdRef`, meaningless outside that context. Rather than
//! duplicate near-identical types, this crate models both children as `Vec`
//! everywhere and always carries the two reference attributes (`None` when
//! not applicable) — the base `MulticastTransportSession`/`ObjectCarousel`
//! context simply never populates them and the `0..1` cardinality is not
//! enforced by the type system (only by the spec, which this parser does
//! not re-validate).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use roxmltree::Node;

use crate::error::Result;
use crate::parse::{children, opt_attr_bool, opt_attr_u64, own_text, require_attr};
use crate::serialize::{push_indent, write_opt_attr, write_opt_bool_attr, write_opt_num_attr};

const OBJECT_CAROUSEL: &str = "ObjectCarousel";
const PRESENTATION_MANIFESTS: &str = "PresentationManifests";
const INIT_SEGMENTS: &str = "InitSegments";
const RESOURCE_LOCATOR: &str = "ResourceLocator";

/// `ObjectCarousel` (clause 10.2.3.14) — the in-band multicast carousel
/// attached to one transport session.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct ObjectCarousel {
    /// Combined size as transmitted at `M` (excl. metadata/protocol overhead).
    pub aggregate_transport_size: Option<u64>,
    /// Combined size after removing content-encoding (e.g. compression);
    /// `None` if no content-encoding was applied.
    pub aggregate_content_size: Option<u64>,
    /// Presence of an entry => carousel all manifests for this session's
    /// service components. See the module doc for the `0..1` vs `0..n`
    /// simplification.
    pub presentation_manifests: Vec<PresentationManifests>,
    /// Presence of an entry => carousel current-Period init segments (DASH)
    /// / `EXT-X-MAP` sections (HLS).
    pub init_segments: Vec<InitSegments>,
    /// Arbitrary resource URLs to carousel.
    pub resource_locators: Vec<ResourceLocator>,
}

/// `PresentationManifests` (clause 10.2.3.14) / the `ReferencingObjectCarouselType`
/// variant used under `gateway.rs` (clause 10.2.5).
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct PresentationManifests {
    /// Target acquisition latency (ISO 8601 duration); omitted => repeat as
    /// often as the session bit rate allows.
    pub target_acquisition_latency: Option<String>,
    /// Server-config only: whether content-encoding compression is preferred.
    pub compression_preferred: Option<bool>,
    /// `ReferencingObjectCarouselType` only: target `MulticastSession/@serviceIdentifier`;
    /// omitted = all active sessions.
    pub service_id_ref: Option<String>,
    /// `ReferencingObjectCarouselType` only: target `MulticastTransportSession/@id`
    /// within that session; illegal without `service_id_ref`.
    pub transport_session_id_ref: Option<String>,
}

/// `InitSegments` (clause 10.2.3.14) — same attribute set as
/// [`PresentationManifests`].
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct InitSegments {
    /// Target acquisition latency (ISO 8601 duration).
    pub target_acquisition_latency: Option<String>,
    /// Server-config only: whether content-encoding compression is preferred.
    pub compression_preferred: Option<bool>,
    /// `ReferencingObjectCarouselType` only: target service.
    pub service_id_ref: Option<String>,
    /// `ReferencingObjectCarouselType` only: target transport session.
    pub transport_session_id_ref: Option<String>,
}

/// `ResourceLocator` (clause 10.2.3.14) — an arbitrary resource URL to
/// carousel.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ResourceLocator {
    /// The resource URL (element content).
    pub uri: String,
    /// Target acquisition latency (ISO 8601 duration).
    pub target_acquisition_latency: Option<String>,
    /// Origin revalidation interval (ISO 8601 duration).
    pub revalidation_period: Option<String>,
    /// Whether content-encoding compression is preferred.
    pub compression_preferred: Option<bool>,
}

/// The four attributes shared identically by `PresentationManifests` and
/// `InitSegments` (see the module doc's `0..1` vs `0..n` simplification).
/// Returned as [`PresentationManifests`] itself and destructured by
/// `InitSegments::parse` — a named type instead of a 4-tuple keeps clippy's
/// `type_complexity` lint happy and documents each field once.
fn parse_manifest_attrs(
    node: Node<'_, '_>,
    element: &'static str,
) -> Result<PresentationManifests> {
    Ok(PresentationManifests {
        target_acquisition_latency: require_attr(node, element, "targetAcquisitionLatency").ok(),
        compression_preferred: opt_attr_bool(node, element, "compressionPreferred")?,
        service_id_ref: require_attr(node, element, "serviceIdRef").ok(),
        transport_session_id_ref: require_attr(node, element, "transportSessionIdRef").ok(),
    })
}

fn write_manifest_attrs(
    out: &mut String,
    target_acquisition_latency: &Option<String>,
    compression_preferred: Option<bool>,
    service_id_ref: &Option<String>,
    transport_session_id_ref: &Option<String>,
) {
    write_opt_attr(
        out,
        "targetAcquisitionLatency",
        target_acquisition_latency.as_deref(),
    );
    write_opt_bool_attr(out, "compressionPreferred", compression_preferred);
    write_opt_attr(out, "serviceIdRef", service_id_ref.as_deref());
    write_opt_attr(
        out,
        "transportSessionIdRef",
        transport_session_id_ref.as_deref(),
    );
}

impl PresentationManifests {
    fn parse(node: Node<'_, '_>) -> Result<Self> {
        parse_manifest_attrs(node, PRESENTATION_MANIFESTS)
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<PresentationManifests");
        write_manifest_attrs(
            out,
            &self.target_acquisition_latency,
            self.compression_preferred,
            &self.service_id_ref,
            &self.transport_session_id_ref,
        );
        out.push_str("/>\n");
    }
}

impl InitSegments {
    fn parse(node: Node<'_, '_>) -> Result<Self> {
        let attrs = parse_manifest_attrs(node, INIT_SEGMENTS)?;
        Ok(InitSegments {
            target_acquisition_latency: attrs.target_acquisition_latency,
            compression_preferred: attrs.compression_preferred,
            service_id_ref: attrs.service_id_ref,
            transport_session_id_ref: attrs.transport_session_id_ref,
        })
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<InitSegments");
        write_manifest_attrs(
            out,
            &self.target_acquisition_latency,
            self.compression_preferred,
            &self.service_id_ref,
            &self.transport_session_id_ref,
        );
        out.push_str("/>\n");
    }
}

impl ResourceLocator {
    fn parse(node: Node<'_, '_>) -> Result<Self> {
        Ok(ResourceLocator {
            uri: own_text(node),
            target_acquisition_latency: require_attr(
                node,
                RESOURCE_LOCATOR,
                "targetAcquisitionLatency",
            )
            .ok(),
            revalidation_period: require_attr(node, RESOURCE_LOCATOR, "revalidationPeriod").ok(),
            compression_preferred: opt_attr_bool(node, RESOURCE_LOCATOR, "compressionPreferred")?,
        })
    }

    fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<ResourceLocator");
        write_opt_attr(
            out,
            "targetAcquisitionLatency",
            self.target_acquisition_latency.as_deref(),
        );
        write_opt_attr(
            out,
            "revalidationPeriod",
            self.revalidation_period.as_deref(),
        );
        write_opt_bool_attr(out, "compressionPreferred", self.compression_preferred);
        out.push('>');
        out.push_str(&crate::serialize::xml_escape(&self.uri));
        out.push_str("</ResourceLocator>\n");
    }
}

impl ObjectCarousel {
    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let mut presentation_manifests = Vec::new();
        for n in children(node, PRESENTATION_MANIFESTS) {
            presentation_manifests.push(PresentationManifests::parse(n)?);
        }
        let mut init_segments = Vec::new();
        for n in children(node, INIT_SEGMENTS) {
            init_segments.push(InitSegments::parse(n)?);
        }
        let mut resource_locators = Vec::new();
        for n in children(node, RESOURCE_LOCATOR) {
            resource_locators.push(ResourceLocator::parse(n)?);
        }
        Ok(ObjectCarousel {
            aggregate_transport_size: opt_attr_u64(
                node,
                OBJECT_CAROUSEL,
                "aggregateTransportSize",
            )?,
            aggregate_content_size: opt_attr_u64(node, OBJECT_CAROUSEL, "aggregateContentSize")?,
            presentation_manifests,
            init_segments,
            resource_locators,
        })
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        push_indent(out, indent);
        out.push_str("<ObjectCarousel");
        write_opt_num_attr(out, "aggregateTransportSize", self.aggregate_transport_size);
        write_opt_num_attr(out, "aggregateContentSize", self.aggregate_content_size);
        let empty = self.presentation_manifests.is_empty()
            && self.init_segments.is_empty()
            && self.resource_locators.is_empty();
        if empty {
            out.push_str("/>\n");
            return;
        }
        out.push_str(">\n");
        for pm in &self.presentation_manifests {
            pm.write_xml(out, indent + 1);
        }
        for is in &self.init_segments {
            is.write_xml(out, indent + 1);
        }
        for rl in &self.resource_locators {
            rl.write_xml(out, indent + 1);
        }
        push_indent(out, indent);
        out.push_str("</ObjectCarousel>\n");
    }
}
