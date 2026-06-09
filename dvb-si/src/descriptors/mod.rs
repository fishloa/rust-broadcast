//! DVB + MPEG-2 descriptors. Each descriptor tag gets its own submodule file.
//!
//! The usual way to consume a descriptor is to walk a table's raw descriptor
//! loop with [`parse_loop`], which yields a typed [`AnyDescriptor`] per entry
//! (or [`AnyDescriptor::Unknown`] for tags with no implementation). To handle a
//! single known tag, call the specific module's `parse` directly (e.g.
//! [`satellite_delivery_system`], [`service`], [`linkage`]).

pub mod aac;
pub mod ac3;
pub mod adaptation_field_data;
pub mod ancillary_data;
pub mod announcement_support;
pub mod any;
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
pub mod registry;
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

pub use any::{parse_loop, AnyDescriptor, DescriptorIter, DescriptorLoop};
pub use registry::{DescriptorObject, DescriptorRegistry};

/// Encode `value` to `nibbles` packed-BCD digits for a `set_*` accessor,
/// mapping overflow to a [`ValueOutOfRange`](crate::Error::ValueOutOfRange)
/// error tagged with the descriptor `field`. Shared by the delivery-system
/// descriptors.
pub(crate) fn encode_bcd_field(value: u64, nibbles: u8, field: &'static str) -> crate::Result<u64> {
    dvb_common::bcd::decimal_to_bcd(value, nibbles).ok_or(crate::Error::ValueOutOfRange {
        field,
        reason: "value exceeds the BCD field width",
    })
}
