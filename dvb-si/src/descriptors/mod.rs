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
pub mod stuffing;
pub mod vbi_data;
pub mod vbi_teletext;
pub mod country_availability;
pub mod nvod_reference;
pub mod time_shifted_service;
pub mod time_shifted_event;
pub mod mosaic;
pub mod ca_identifier;
pub mod telephone;
pub mod multilingual_network_name;
pub mod multilingual_bouquet_name;
pub mod multilingual_service_name;
pub mod multilingual_component;
pub mod private_data_specifier;
pub mod service_move;
pub mod short_smoothing_buffer;
pub mod partial_transport_stream;
pub mod data_broadcast;
pub mod scrambling;
pub mod data_broadcast_id;
pub mod transport_stream;
pub mod dsng;
pub mod pdc;
pub mod ancillary_data;
pub mod cell_list;
pub mod cell_frequency_link;
pub mod announcement_support;
pub mod application_signalling;
pub mod adaptation_field_data;
pub mod service_identifier;
pub mod service_availability;
pub mod related_content;
pub mod tva_id;
pub mod time_slice_fec_identifier;
pub mod ecm_repetition_rate;
pub mod dts;
pub mod aac;
pub mod xait_location;
pub mod fta_content_management;
pub mod extension;

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
