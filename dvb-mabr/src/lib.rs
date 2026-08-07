//! DVB multicast ABR (DVB-MABR) session configuration XML.
//!
//! Parses and serializes the multicast session configuration instance
//! document defined by ETSI TS 103 769 V1.2.1 clause 10 — the one XML
//! document format the whole DVB-MABR system (Multicast server, Multicast
//! gateway, and the client-facing rendezvous service) is coordinated by.
//! Two document flavours share the same schema: [`MulticastServerConfiguration`]
//! (root `MulticastServerConfiguration`, sent to the Multicast server) and
//! [`MulticastGatewayConfiguration`] (root `MulticastGatewayConfiguration`,
//! sent to the Multicast gateway). See this crate's `docs/mabr-signalling.md`
//! for the full field-level transcription this implementation follows.
//!
//! Presentation manifests (DASH MPD / HLS Master Playlist) are referenced
//! by URL only — never parsed; their own formats (ISO/IEC 23009-1, IETF RFC
//! 8216) are out of scope, as is the multicast *transport* of the objects
//! themselves (`dvb-flute` covers the ALC/LCT/FLUTE wire format TS 103 769
//! Annex F profiles).
//!
//! ## Round-trip guarantee
//!
//! `parse_str → to_xml → parse_str` yields an equal document (fields
//! compare equal via `PartialEq`). The serialized XML is **not**
//! byte-identical to the input:
//! - Attribute order is fixed by this crate's serializer, not preserved from the input.
//! - Whitespace/indentation is normalized to 2-space indentation.
//! - Comments and processing instructions (other than the XML declaration
//!   this crate always emits) are dropped.
//! - Empty elements are always self-closed (`<Foo/>`), regardless of how
//!   the input wrote them.
//! - **Extension elements/attributes** (Annex A.1: `xs:any`/`xs:anyAttribute`
//!   at the closed set of extension points listed there) are recognized
//!   and silently skipped on parse for forward compatibility, but are
//!   **not stored** and so are **not re-emitted** by `to_xml` — round-tripping
//!   a document that uses extensions will lose them. This is a known,
//!   documented limitation, not a bug: modeling every possible
//!   third-party extension namespace is out of scope.
//! - The `ReferencingObjectCarouselType` vs. base `ObjectCarouselType`
//!   cardinality distinction (clause 10.2.5) is not enforced by the type
//!   system — see `carousel.rs` module doc for detail.
//!
//! ## Example
//!
//! ```
//! use dvb_mabr::MulticastGatewayConfiguration;
//!
//! let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
//! <MulticastGatewayConfiguration schemaVersion="2"
//!     xmlns="urn:dvb:metadata:MulticastSessionConfiguration:2024"
//!     xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
//!   <MulticastSession serviceIdentifier="urn:example:service:1">
//!     <PresentationManifestLocator manifestId="mpd1" contentType="application/dash+xml"
//!         >https://cdn.example/service1/manifest.mpd</PresentationManifestLocator>
//!     <MulticastTransportSession id="ts1" sessionIdleTimeout="30000">
//!       <TransportProtocol protocolIdentifier="urn:dvb:metadata:cs:MulticastTransportProtocolCS:2019:FLUTE" protocolVersion="1"/>
//!       <EndpointAddress>
//!         <NetworkDestinationGroupAddress>239.1.1.1</NetworkDestinationGroupAddress>
//!         <TransportDestinationPort>6000</TransportDestinationPort>
//!       </EndpointAddress>
//!       <BitRate maximum="5000000"/>
//!       <ServiceComponentIdentifier xsi:type="GenericComponentIdentifierType"
//!           manifestIdRef="mpd1" componentIdentifier="video"/>
//!     </MulticastTransportSession>
//!   </MulticastSession>
//! </MulticastGatewayConfiguration>"#;
//!
//! let config = MulticastGatewayConfiguration::parse_str(xml).unwrap();
//! assert_eq!(config.sessions.len(), 1);
//! assert_eq!(config.sessions[0].service_identifier, "urn:example:service:1");
//!
//! let round_tripped = MulticastGatewayConfiguration::parse_str(&config.to_xml()).unwrap();
//! assert_eq!(config, round_tripped);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod carousel;
pub mod component;
pub mod config;
pub mod error;
pub mod fec;
pub mod gateway;
mod parse;
pub mod repair;
pub mod reporting;
mod serialize;
pub mod session;
pub mod transport;

pub use carousel::{InitSegments, ObjectCarousel, PresentationManifests, ResourceLocator};
pub use component::ServiceComponentIdentifier;
pub use config::{MulticastGatewayConfiguration, MulticastServerConfiguration};
pub use error::{Error, Result};
pub use fec::ForwardErrorCorrectionParameters;
pub use gateway::{ConfigurationMacro, MulticastGatewayConfigurationTransportSession};
pub use parse::{
    NS_EXTENSIBILITY_2024, NS_MULTICAST_SESSION_CONFIGURATION_2019,
    NS_MULTICAST_SESSION_CONFIGURATION_2024, NS_XSI,
};
pub use repair::{BaseUrl, UnicastRepairParameters};
pub use reporting::{MulticastGatewaySessionReporting, ReportingLocator};
pub use session::{MulticastSession, PresentationManifestLocator};
pub use transport::{
    BitRate, ContentIngestMethod, EndpointAddress, MulticastTransportSession, TransmissionMode,
    TransportProtocol, TransportSecurity,
};
