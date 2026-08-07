//! `ServiceComponentIdentifier` — ETSI TS 103 769 V1.2.1 clause 10.2.4.
//!
//! Every `MulticastTransportSession` carries 1..n of these, each pointing at
//! one component of one manifest referenced by the parent `MulticastSession`
//! (via `@manifestIdRef` -> `PresentationManifestLocator/@manifestId`). The
//! concrete shape is selected by `@xsi:type` (Tables 10.2.4.1-1, 10.2.4.2-1,
//! and the third, `GenericComponentIdentifierType`, clause 10.2.4).

extern crate alloc;

use alloc::string::String;

use roxmltree::Node;

use crate::error::{Error, Result};
use crate::parse::{require_attr, xsi_type};
use crate::serialize::write_attr;

const ELEMENT: &str = "ServiceComponentIdentifier";

const XSI_TYPE_DASH: &str = "DASHComponentIdentifierType";
const XSI_TYPE_HLS: &str = "HLSComponentIdentifierType";
const XSI_TYPE_GENERIC: &str = "GenericComponentIdentifierType";

/// One service-component reference, typed by `xsi:type` (clause 10.2.4).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ServiceComponentIdentifier {
    /// `DASHComponentIdentifierType` (Table 10.2.4.1-1) — a DASH MPD
    /// Representation, addressed by Period/AdaptationSet/Representation id.
    Dash {
        /// -> `PresentationManifestLocator/@manifestId` of an MPD.
        manifest_id_ref: String,
        /// -> `Period/@id`.
        period_id: String,
        /// -> `AdaptationSet/@id`.
        adaptation_set_id: u32,
        /// -> `Representation/@id`.
        representation_id: String,
    },
    /// `HLSComponentIdentifierType` (Table 10.2.4.2-1) — an HLS Media
    /// Playlist referenced from a Master Playlist.
    Hls {
        /// -> `PresentationManifestLocator/@manifestId` of an HLS Master Playlist.
        manifest_id_ref: String,
        /// Absolute URL of the referenced HLS Media Playlist.
        media_playlist_locator: String,
    },
    /// `GenericComponentIdentifierType` — manifest types outside DASH/HLS.
    Generic {
        /// -> `PresentationManifestLocator/@manifestId`.
        manifest_id_ref: String,
        /// Implementation-defined component identifier.
        component_id: String,
    },
}

impl ServiceComponentIdentifier {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            ServiceComponentIdentifier::Dash { .. } => "dash",
            ServiceComponentIdentifier::Hls { .. } => "hls",
            ServiceComponentIdentifier::Generic { .. } => "generic",
        }
    }

    pub(crate) fn parse(node: Node<'_, '_>) -> Result<Self> {
        let ty = xsi_type(node).ok_or(Error::MissingComponentType)?;
        match ty.as_str() {
            XSI_TYPE_DASH => Ok(ServiceComponentIdentifier::Dash {
                manifest_id_ref: require_attr(node, ELEMENT, "manifestIdRef")?,
                period_id: require_attr(node, ELEMENT, "periodIdentifier")?,
                adaptation_set_id: crate::parse::req_attr_u32(
                    node,
                    ELEMENT,
                    "adaptationSetIdentifier",
                )?,
                representation_id: require_attr(node, ELEMENT, "representationIdentifier")?,
            }),
            XSI_TYPE_HLS => Ok(ServiceComponentIdentifier::Hls {
                manifest_id_ref: require_attr(node, ELEMENT, "manifestIdRef")?,
                media_playlist_locator: require_attr(node, ELEMENT, "mediaPlaylistLocator")?,
            }),
            XSI_TYPE_GENERIC => Ok(ServiceComponentIdentifier::Generic {
                manifest_id_ref: require_attr(node, ELEMENT, "manifestIdRef")?,
                component_id: require_attr(node, ELEMENT, "componentIdentifier")?,
            }),
            other => Err(Error::UnknownComponentType(other.into())),
        }
    }

    pub(crate) fn write_xml(&self, out: &mut String, indent: usize) {
        crate::serialize::push_indent(out, indent);
        out.push_str("<ServiceComponentIdentifier");
        match self {
            ServiceComponentIdentifier::Dash {
                manifest_id_ref,
                period_id,
                adaptation_set_id,
                representation_id,
            } => {
                write_attr(out, "xsi:type", XSI_TYPE_DASH);
                write_attr(out, "manifestIdRef", manifest_id_ref);
                write_attr(out, "periodIdentifier", period_id);
                crate::serialize::write_num_attr(
                    out,
                    "adaptationSetIdentifier",
                    *adaptation_set_id,
                );
                write_attr(out, "representationIdentifier", representation_id);
            }
            ServiceComponentIdentifier::Hls {
                manifest_id_ref,
                media_playlist_locator,
            } => {
                write_attr(out, "xsi:type", XSI_TYPE_HLS);
                write_attr(out, "manifestIdRef", manifest_id_ref);
                write_attr(out, "mediaPlaylistLocator", media_playlist_locator);
            }
            ServiceComponentIdentifier::Generic {
                manifest_id_ref,
                component_id,
            } => {
                write_attr(out, "xsi:type", XSI_TYPE_GENERIC);
                write_attr(out, "manifestIdRef", manifest_id_ref);
                write_attr(out, "componentIdentifier", component_id);
            }
        }
        out.push_str("/>\n");
    }
}

broadcast_common::impl_spec_display!(ServiceComponentIdentifier);
