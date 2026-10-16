#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod error;

// Everything below reads or writes XML through `quick-xml`, which needs `std`;
// a `no_std` build exposes only [`error`].
#[cfg(feature = "std")]
pub mod carousel;
#[cfg(feature = "std")]
pub mod component;
#[cfg(feature = "std")]
pub mod config;
#[cfg(feature = "std")]
pub mod fec;
#[cfg(feature = "std")]
pub mod gateway;
#[cfg(feature = "std")]
mod parse;
#[cfg(feature = "std")]
pub mod repair;
#[cfg(feature = "std")]
pub mod reporting;
#[cfg(feature = "std")]
mod serialize;
#[cfg(feature = "std")]
pub mod session;
#[cfg(feature = "std")]
pub mod transport;

#[cfg(feature = "std")]
pub use carousel::{InitSegments, ObjectCarousel, PresentationManifests, ResourceLocator};
#[cfg(feature = "std")]
pub use component::ServiceComponentIdentifier;
#[cfg(feature = "std")]
pub use config::{BaselineNamespace, MulticastGatewayConfiguration, MulticastServerConfiguration};
pub use error::{Error, Result};
#[cfg(feature = "std")]
pub use fec::ForwardErrorCorrectionParameters;
#[cfg(feature = "std")]
pub use gateway::{ConfigurationMacro, MulticastGatewayConfigurationTransportSession};
#[cfg(feature = "std")]
pub use parse::{
    NS_EXTENSIBILITY_2024, NS_MULTICAST_SESSION_CONFIGURATION_2019,
    NS_MULTICAST_SESSION_CONFIGURATION_2024, NS_XSI,
};
#[cfg(feature = "std")]
pub use repair::{BaseUrl, UnicastRepairParameters};
#[cfg(feature = "std")]
pub use reporting::{MulticastGatewaySessionReporting, ReportingLocator};
#[cfg(feature = "std")]
pub use session::{MulticastSession, PresentationManifestLocator};
#[cfg(feature = "std")]
pub use transport::{
    BitRate, ContentIngestMethod, EndpointAddress, MulticastTransportSession, TransmissionMode,
    TransportProtocol, TransportSecurity,
};
