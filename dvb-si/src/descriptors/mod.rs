//! DVB + MPEG-2 descriptors.
//!
//! Each descriptor tag gets its own submodule file (see the file tree in
//! `docs/superpowers/specs/2026-04-20-dvb-si-crate-design.md`).
//!
//! The public [`Descriptor`] enum dispatches by tag; unknown tags are
//! preserved as [`Descriptor::Unknown`] for forward compatibility.

pub mod ac3;
pub mod bouquet_name;
pub mod ca;
pub mod cable_delivery_system;
pub mod component;
pub mod content;
pub mod content_identifier;
pub mod data_stream_alignment;
pub mod default_authority;
pub mod enhanced_ac3;
pub mod extended_event;
pub mod frequency_list;
pub mod iso_639_language;
pub mod linkage;
pub mod local_time_offset;
pub mod logical_channel;
pub mod network_name;
pub mod parental_rating;
pub mod private_data_indicator;
pub mod registration;
pub mod s2_satellite_delivery_system;
pub mod satellite_delivery_system;
pub mod service;
pub mod service_list;
pub mod short_event;
pub mod stream_identifier;
pub mod subtitling;
pub mod teletext;
pub mod terrestrial_delivery_system;

pub use ca::CaDescriptor;
pub use data_stream_alignment::DataStreamAlignmentDescriptor;
pub use private_data_indicator::PrivateDataIndicatorDescriptor;
pub use registration::RegistrationDescriptor;

/// Unified descriptor variant. Variants land as per-descriptor phases complete.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Descriptor<'a> {
    /// Conditional Access descriptor (tag 0x09).
    Ca(CaDescriptor<'a>),
    /// Data stream alignment descriptor (tag 0x06).
    DataStreamAlignment(DataStreamAlignmentDescriptor),
    /// Private Data Indicator descriptor (tag 0x0F).
    PrivateDataIndicator(PrivateDataIndicatorDescriptor),
    /// Registration descriptor (tag 0x05).
    Registration(RegistrationDescriptor<'a>),
    /// Forward-compatible fallthrough for tags we don't recognise.
    Unknown {
        /// The raw tag byte.
        tag: u8,
        /// The raw payload (descriptor_length bytes, not including the 2-byte header).
        #[cfg_attr(feature = "serde", serde(borrow))]
        bytes: &'a [u8],
    },
}

impl<'a> Descriptor<'a> {
    /// The underlying tag byte.
    #[must_use]
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Ca(_) => ca::TAG,
            Self::DataStreamAlignment(_) => data_stream_alignment::TAG,
            Self::PrivateDataIndicator(_) => private_data_indicator::TAG,
            Self::Registration(_) => registration::TAG,
            Self::Unknown { tag, .. } => *tag,
        }
    }
}
