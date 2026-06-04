//! DVB + MPEG-2 descriptors. Each descriptor tag gets its own submodule file.
//!
//! The usual way to consume a descriptor is to walk a table's raw descriptor
//! loop and call the specific descriptor module's `parse` for the tags you
//! care about (e.g. [`satellite_delivery_system`], [`service`], [`linkage`]).
//!
//! The [`Descriptor`] enum covers only the small set of MPEG-2 descriptors
//! that appear in contexts without surrounding table semantics (CA,
//! data_stream_alignment, private_data_indicator, registration); every other
//! tag lands as [`Descriptor::Unknown`] with its raw bytes preserved — it is
//! NOT a dispatcher over the full typed-descriptor set.

pub mod aac;
pub mod ac3;
pub mod adaptation_field_data;
pub mod ancillary_data;
pub mod announcement_support;
pub mod application_signalling;
pub mod bouquet_name;
pub mod ca;
pub mod ca_identifier;
pub mod cable_delivery_system;
pub mod cell_frequency_link;
pub mod cell_list;
pub mod component;
pub mod content;
pub mod content_identifier;
pub mod country_availability;
pub mod data_broadcast;
pub mod data_broadcast_id;
pub mod data_stream_alignment;
pub mod default_authority;
pub mod dsng;
pub mod dts;
pub mod ecm_repetition_rate;
pub mod enhanced_ac3;
pub mod extended_event;
pub mod extension;
pub mod frequency_list;
pub mod fta_content_management;
pub mod iso_639_language;
pub mod linkage;
pub mod local_time_offset;
pub mod logical_channel;
pub mod mosaic;
pub mod multilingual_bouquet_name;
pub mod multilingual_component;
pub mod multilingual_network_name;
pub mod multilingual_service_name;
pub mod network_name;
pub mod nvod_reference;
pub mod parental_rating;
pub mod partial_transport_stream;
pub mod pdc;
pub mod private_data_indicator;
pub mod private_data_specifier;
pub mod registration;
pub mod related_content;
pub mod s2_satellite_delivery_system;
pub mod satellite_delivery_system;
pub mod scrambling;
pub mod service;
pub mod service_availability;
pub mod service_identifier;
pub mod service_list;
pub mod service_move;
pub mod short_event;
pub mod short_smoothing_buffer;
pub mod stream_identifier;
pub mod stuffing;
pub mod subtitling;
pub mod telephone;
pub mod teletext;
pub mod terrestrial_delivery_system;
pub mod time_shifted_event;
pub mod time_shifted_service;
pub mod time_slice_fec_identifier;
pub mod transport_stream;
pub mod tva_id;
pub mod vbi_data;
pub mod vbi_teletext;
pub mod xait_location;

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
