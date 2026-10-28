//! DVB + MPEG-2 descriptors. Each descriptor tag gets its own submodule file.
//!
//! The usual way to consume a descriptor is to walk a table's raw descriptor
//! loop with [`parse_loop`], which yields a typed [`AnyDescriptor`] per entry
//! (or [`AnyDescriptor::Unknown`] for tags with no implementation). To handle a
//! single known tag, call the specific module's `parse` directly (e.g.
//! [`satellite_delivery_system`], [`service`], [`linkage`]).
//!
//! # Tag namespaces
//!
//! | Namespace | Entry point | Allocated by |
//! |---|---|---|
//! | MPEG-2 systems + DSM-CC, tags 0x02–0x3F | [`AnyDescriptor`] (auto-dispatched) | Rec. ITU-T H.222.0 Table 2-45, ISO/IEC 13818-6 |
//! | DVB, tags 0x40–0x7F | [`AnyDescriptor`] (auto-dispatched) | ETSI EN 300 468 Table 12 |
//! | DVB `extension_descriptor` (0x7F) bodies | [`extension::ExtensionBody`], [`ExtensionRegistry`] | ETSI EN 300 468 Table 109 |
//! | MPEG-2 `Extension_descriptor` (0x3F) bodies | [`mpeg2_extension::Mpeg2ExtensionBody`] (MPEG-H 3D audio) | Rec. ITU-T H.222.0 Table 2-110 |
//! | Private / regional (PDS- or registration-scoped) | [`DescriptorRegistry`] opt-ins: [`logical_channel`], [`nordig`], [`avs2_video`], [`avs2_audio`], [`avs3_video`], [`avs3_audio`], [`tuwa_avs3_video`], [`china_caption_service`] | EACEM / NorDig / GY/T / T/AI / T/UWA |
//! | AIT descriptors | [`parse_ait_loop`] → [`AnyAitDescriptor`] | ETSI TS 102 809 Table 38, TS 102 727 |
//! | INT / UNT / RNT descriptor loops | [`parse_int_loop`] → [`AnyIntDescriptor`], [`parse_unt_loop`] → [`AnyUntDescriptor`], [`parse_rnt_loop`] → [`AnyRntDescriptor`] (SI fall-through where the spec hands back to EN 300 468) | ETSI EN 301 192 Table 19, ETSI TS 102 006 Table 19, ETSI TS 102 323 Table 4 |
//! | DSM-CC data-carousel descriptors | [`parse_carousel_loop`] → [`AnyCarouselDescriptor`] | ETSI EN 301 192 Table 48, TS 102 006 |
//!
//! Every tag, its verified clause, Rust type and status — and every allocated
//! tag with no implementation yet — is listed in the generated
//! [`dvb-si/docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md).

pub mod aac;
pub mod aac_additional_info;
pub mod ac3;
pub mod adaptation_field_data;
pub mod ait;
pub mod ancillary_data;
pub mod announcement_support;
pub mod any;
pub mod application_signalling;
pub mod association_tag;
pub mod audio_stream;
pub mod auxiliary_video_stream;
pub mod avc_timing_and_hrd;
pub mod avc_video;
pub mod avs2_audio;
pub mod avs2_video;
pub mod avs3_audio;
pub mod avs3_video;
pub mod bouquet_name;
pub mod ca;
pub mod ca_identifier;
pub mod cable_delivery_system;
pub mod carousel;
pub mod carousel_identifier;
pub mod cell_frequency_link;
pub mod cell_list;
pub mod china_caption_service;
pub mod component;
pub mod content;
pub mod content_identifier;
pub mod content_labeling;
pub mod content_time_base_indicator;
pub mod copyright;
pub mod country_availability;
pub mod data_broadcast;
pub mod data_broadcast_id;
pub mod data_stream_alignment;
pub mod decoder_config_flags;
pub mod default_authority;
pub mod deferred_association_tags;
pub mod dsng;
pub mod dts;
pub mod ecm_repetition_rate;
pub mod enhanced_ac3;
mod entry_list;
pub mod evc_video;
pub mod extended_event;
pub mod extension;
pub mod external_es_id;
pub mod flex_mux_timing;
pub mod fmc;
pub mod fmx_buffer_size;
pub mod frequency_list;
pub mod fta_content_management;
pub mod h222_timing;
pub mod hdr_wcg_idc;
pub mod hevc_video;
pub mod hierarchy;
pub mod ibp;
pub mod int;
pub mod iod;
pub mod iso_639_language;
pub mod j2k_video;
mod lang_text;
pub mod linkage;
pub mod local_time_offset;
pub mod logical_channel;
pub mod maximum_bitrate;
pub mod metadata;
pub mod metadata_format;
pub mod metadata_pointer;
pub mod metadata_std;
pub mod mjd_time;
pub mod mosaic;
pub mod mpeg2_aac_audio;
pub mod mpeg2_ext_codecs;
pub mod mpeg2_ext_hevc;
pub mod mpeg2_ext_misc;
pub mod mpeg2_extension;
pub mod mpeg4_audio;
pub mod mpeg4_audio_extension;
pub mod mpeg4_text;
pub mod mpeg4_video;
pub mod mpeg_carriage_flags;
pub mod mpegh_3d_audio;
pub mod multilingual_bouquet_name;
pub mod multilingual_component;
pub mod multilingual_network_name;
pub mod multilingual_service_name;
pub mod multiplex_buffer;
pub mod multiplex_buffer_utilization;
pub mod muxcode;
pub mod mvc_extension;
pub mod mvc_operation_point;
pub mod network_name;
pub mod nordig;
pub mod nvod_reference;
pub mod parental_rating;
pub mod partial_transport_stream;
pub mod pdc;
pub mod private_data_indicator;
pub mod private_data_specifier;
pub mod registration;
pub mod registry;
pub mod related_content;
pub mod rnt;
pub mod s2_satellite_delivery_system;
pub mod satellite_delivery_system;
pub mod scoped;
pub mod scrambling;
pub mod service;
pub mod service_availability;
pub mod service_identifier;
pub mod service_list;
pub mod service_move;
pub mod short_event;
pub mod short_smoothing_buffer;
pub mod sl;
pub mod smoothing_buffer;
pub mod std;
pub mod stereoscopic;
pub mod stream_identifier;
pub mod stuffing;
pub mod subtitling;
pub mod svc_extension;
pub mod system_clock;
pub mod target;
pub mod target_background_grid;
pub mod telephone;
pub mod teletext;
pub mod terrestrial_delivery_system;
pub mod time_shifted_event;
pub mod time_shifted_service;
pub mod time_slice_fec_identifier;
pub mod transport_profile;
pub mod transport_stream;
pub mod tuwa_avs3_video;
pub mod tva_id;
pub mod unt;
pub mod vbi_data;
pub mod vbi_teletext;
pub mod video_stream;
pub mod video_window;
pub mod vvc_video;
pub mod xait_location;

pub use ca::CaDescriptor;
pub use data_stream_alignment::DataStreamAlignmentDescriptor;
pub use entry_list::MacAddress;
pub use int::{AnyIntDescriptor, IntDescriptorIter, IntDescriptorLoop, parse_int_loop};
pub use mjd_time::MjdBcdTime;
pub use private_data_indicator::PrivateDataIndicatorDescriptor;
pub use registration::RegistrationDescriptor;
pub use rnt::{AnyRntDescriptor, RntDescriptorIter, RntDescriptorLoop, parse_rnt_loop};
pub use unt::{AnyUntDescriptor, UntDescriptorIter, UntDescriptorLoop, parse_unt_loop};

/// Write a descriptor's 2-byte header — `tag` then a range-checked 8-bit
/// `descriptor_length` — into `buf[0..2]`.
///
/// `buf` must already be known to hold at least `2 + body_len` bytes (callers
/// check this via `serialized_len()` before calling). Returns
/// [`Error::FieldOverflow`](crate::Error::FieldOverflow) instead of silently
/// narrowing (`as u8`) a `body_len` that does not fit the 8-bit field, which
/// would otherwise emit a misframed descriptor loop (#1129).
pub(crate) fn write_descriptor_header(
    buf: &mut [u8],
    tag: u8,
    body_len: usize,
) -> crate::Result<()> {
    buf[0] = tag;
    buf[1] = broadcast_common::len::fit_u8(body_len, "descriptor_length")?;
    Ok(())
}

pub(crate) fn descriptor_body<'a>(
    bytes: &'a [u8],
    tag: u8,
    what: &'static str,
    reason: &'static str,
) -> crate::Result<&'a [u8]> {
    const HEADER_LEN: usize = 2;
    if bytes.len() < HEADER_LEN {
        return Err(crate::Error::BufferTooShort {
            need: HEADER_LEN,
            have: bytes.len(),
            what,
        });
    }
    if bytes[0] != tag {
        return Err(crate::Error::InvalidDescriptor {
            tag: bytes[0],
            reason,
        });
    }
    let length = bytes[1] as usize;
    let end = HEADER_LEN + length;
    if bytes.len() < end {
        return Err(crate::Error::BufferTooShort {
            need: end,
            have: bytes.len(),
            what,
        });
    }
    Ok(&bytes[HEADER_LEN..end])
}

pub use ait::{AitDescriptorIter, AitDescriptorLoop, AnyAitDescriptor, parse_ait_loop};
pub use any::{AnyDescriptor, DescriptorIter, DescriptorLoop, parse_loop};
pub use carousel::{
    AnyCarouselDescriptor, CarouselDescriptorIter, CarouselDescriptorLoop, parse_carousel_loop,
};
pub use extension::registry::{ExtensionObject, ExtensionRegistry, RegisteredExtension};
pub use registry::{
    DescriptorObject, DescriptorRegistry, ExtIterItem, ExtRegistryIter, RegistryIter,
};

pub use private_data_specifier::{PDS_AVSA, PDS_AVSV, PDS_EACEM, PDS_NORDIG};

/// Encode `value` to `nibbles` packed-BCD digits for a `set_*` accessor,
/// mapping overflow to a [`ValueOutOfRange`](crate::Error::ValueOutOfRange)
/// error tagged with the descriptor `field`. Shared by the delivery-system
/// descriptors.
pub(crate) fn encode_bcd_field(value: u64, nibbles: u8, field: &'static str) -> crate::Result<u64> {
    broadcast_common::bcd::decimal_to_bcd(value, nibbles).ok_or(crate::Error::ValueOutOfRange {
        field,
        reason: "value exceeds the BCD field width",
    })
}

#[cfg(test)]
mod header_tests {
    use super::write_descriptor_header;

    /// Boundary: a 255-byte body is the largest an 8-bit `descriptor_length`
    /// can hold, and must still serialize.
    #[test]
    fn write_descriptor_header_accepts_max_body_len() {
        let mut buf = [0u8; 2];
        write_descriptor_header(&mut buf, 0x40, 255).unwrap();
        assert_eq!(buf, [0x40, 255]);
    }

    /// A 256-byte body does not fit the 8-bit field and must return `Err`
    /// rather than wrapping `256 as u8 == 0` into a misframed descriptor
    /// (#1129) — this is the template bug every descriptor serializer had.
    #[test]
    fn write_descriptor_header_rejects_body_len_over_255() {
        let mut buf = [0u8; 2];
        let err = write_descriptor_header(&mut buf, 0x40, 256).unwrap_err();
        assert!(
            matches!(err, crate::Error::FieldOverflow(_)),
            "expected FieldOverflow, got {err:?}"
        );
    }
}
