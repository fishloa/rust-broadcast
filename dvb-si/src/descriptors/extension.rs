//! Extension descriptor — ETSI EN 300 468 §6.2.18.1 (tag `0x7F`).
//!
//! The extension descriptor is a container whose first payload byte,
//! `descriptor_tag_extension`, selects one of a large family of sub-descriptors
//! (EN 300 468 §6.4.0, Table 109 "Possible locations of extended descriptors").
//! The framing is Table 54:
//!
//! ```text
//!  byte 0      descriptor_tag           (0x7F)
//!  byte 1      descriptor_length
//!  byte 2      descriptor_tag_extension (the discriminant)
//!  byte 3..    selector_byte[]          (the sub-descriptor body)
//! ```
//!
//! This type mirrors the SAT precedent (`tables/sat.rs`): a typed discriminant
//! ([`ExtensionDescriptor::tag_extension`], a plain `u8` so unknown values
//! round-trip) plus a typed body enum ([`ExtensionBody`]) with a
//! [`ExtensionBody::Raw`] fall-through. [`ExtensionTag`] names the known
//! discriminant constants but is deliberately NOT used as the stored field type
//! — unknown tags must survive a parse→serialize round-trip, which a
//! `TryFromPrimitive` enum could not guarantee.
//!
//! # Typed vs. raw bodies
//!
//! A body variant is typed only when its syntax table is vendored under
//! `dvb-si/docs/`. For loop-heavy descriptors the first fixed level is typed and
//! the variable-length inner loop is kept as a raw slice (SAT precedent —
//! `tables/sat.rs` keeps bit-packed loops raw). Per-variant section comments
//! cite the governing table + clause.
//!
//! Typed (syntax table vendored in `en_300_468.md`, except TTML in
//! `en_303_560_ttml.md`):
//! - `0x04` T2_delivery_system (Table 133, §6.4.6.3) — first level; cell loop raw.
//! - `0x06` supplementary_audio (Table 153, §6.4.11).
//! - `0x07` network_change_notify (Table 149, §6.4.9) — cell loop raw.
//! - `0x08` message (Table 148, §6.4.9).
//! - `0x09` target_region (Table 156, §6.4.12) — region loop raw.
//! - `0x0A` target_region_name (Table 157, §6.4.13) — region loop raw.
//! - `0x0B` service_relocated (Table 152, §6.4.10).
//! - `0x0D` C2_delivery_system (Table 115, §6.4.6.1).
//! - `0x13` URI_linkage (Table 159, §6.4.16.1) — uri/private split typed.
//! - `0x15` AC-4 (annex D syntax table, §D.5) — first level; toc/extra raw.
//! - `0x16` C2_bundle_delivery_system (Table 139, §6.4.6.4) — full fixed loop.
//! - `0x17` S2X_satellite_delivery_system (Table 140, §6.4.6.5.2) — primary
//!   channel typed; channel-bonding / reserved tail kept raw.
//! - `0x19` audio_preselection (Table 110, §6.4.1) — preselection loop raw.
//! - `0x20` TTML_subtitling (`en_303_560_ttml.md` Table 1, §5.2.1.1).
//!
//! Kept [`ExtensionBody::Raw`] (tag value preserved), with reason:
//! - `0x00` image_icon — syntax vendored (Table 145) but niche (carousel icons); deferred.
//! - `0x01` cpcm_delivery_signalling — spec not vendored (ETSI TS 102 825).
//! - `0x02` CP / `0x03` CP_identifier — spec not vendored (ETSI TS 102 825).
//! - `0x05` SH_delivery_system — niche (satellite-to-handheld); deferred.
//! - `0x0C` XAIT_PID — deferred (TS 102 727 PDF vendored, no extracted syntax table yet).
//! - `0x0E` DTS-HD / `0x0F` DTS_Neural / `0x21` DTS-UHD — spec not vendored (annex G/L).
//! - `0x10` video_depth_range — niche (3D disparity); deferred.
//! - `0x11` T2MI — niche (T2-MI encapsulation); deferred.
//! - `0x14` CI_ancillary_data — spec not vendored (ETSI TS 103 205).
//! - `0x18` protection_message — spec not vendored (ETSI TS 102 809).
//! - `0x22` service_prominence / `0x23` vvc_subpictures / `0x24` S2Xv2 — niche; deferred.
//! - any other value (incl. `0x80`..=`0xFF` user-defined) — unknown; preserved.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for extension_descriptor (EN 300 468 Table 54, §6.2.18.1).
pub const TAG: u8 = 0x7F;
const HEADER_LEN: usize = 2;
/// `descriptor_tag_extension` occupies one byte immediately after the header.
const TAG_EXTENSION_LEN: usize = 1;
/// Minimum body length: just the `descriptor_tag_extension` byte.
const MIN_BODY_LEN: usize = TAG_EXTENSION_LEN;
/// `descriptor_length` is a single byte; a serialized body may not exceed this.
const MAX_DESCRIPTOR_LENGTH: usize = 0xFF;

// Per-variant fixed lengths (bytes after `descriptor_tag_extension`).
const ISO_639_LEN: usize = 3;
const T2_FIXED_PREFIX_LEN: usize = 3; // plp_id(1) + T2_system_id(2)
const T2_FLAGS_BLOCK_LEN: usize = 2; // SISO_MISO..tfs_flag, packed in 2 bytes
const C2_LEN: usize = 7; // plp + data_slice + freq(4) + 1 packed byte
const C2_BUNDLE_ENTRY_LEN: usize = 8; // plp + data_slice + freq(4) + 1 packed + 1 (primary(1)+reserved_zero(7))
const SERVICE_RELOCATED_LEN: usize = 6; // 3 × u16
/// S2X primary-channel block after the 2 flags bytes (excl. scrambling/ISI/timeslice):
/// frequency(4) + orbital_position(2) + 1 packed byte + symbol_rate(4 bytes).
const S2X_PRIMARY_LEN: usize = 11;
const S2X_SCRAMBLING_LEN: usize = 3;
const TTML_FIXED_LEN: usize = ISO_639_LEN + 2; // ISO_639(3) + 2 packed bytes

/// Known `descriptor_tag_extension` values (EN 300 468 Table 109, §6.4.0).
///
/// This is a *naming* aid for callers and parser dispatch; the stored
/// discriminant is the raw [`ExtensionDescriptor::tag_extension`] `u8` so that
/// unknown / reserved / user-defined tags round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum ExtensionTag {
    /// image_icon_descriptor (kept raw — see module docs).
    ImageIcon = 0x00,
    /// T2_delivery_system_descriptor.
    T2DeliverySystem = 0x04,
    /// supplementary_audio_descriptor.
    SupplementaryAudio = 0x06,
    /// network_change_notify_descriptor.
    NetworkChangeNotify = 0x07,
    /// message_descriptor.
    Message = 0x08,
    /// target_region_descriptor.
    TargetRegion = 0x09,
    /// target_region_name_descriptor.
    TargetRegionName = 0x0A,
    /// service_relocated_descriptor.
    ServiceRelocated = 0x0B,
    /// C2_delivery_system_descriptor.
    C2DeliverySystem = 0x0D,
    /// URI_linkage_descriptor.
    UriLinkage = 0x13,
    /// AC-4_descriptor (annex D).
    Ac4 = 0x15,
    /// C2_bundle_delivery_system_descriptor.
    C2BundleDeliverySystem = 0x16,
    /// S2X_satellite_delivery_system_descriptor.
    S2XSatelliteDeliverySystem = 0x17,
    /// audio_preselection_descriptor.
    AudioPreselection = 0x19,
    /// TTML_subtitling_descriptor (ETSI EN 303 560).
    TtmlSubtitling = 0x20,
}

/// Typed body of an extension descriptor, keyed on `descriptor_tag_extension`.
///
/// Unrecognised or not-yet-typed discriminants land in [`ExtensionBody::Raw`],
/// which carries the selector bytes verbatim so the descriptor round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub enum ExtensionBody<'a> {
    /// `0x04` — T2_delivery_system (Table 133, §6.4.6.3).
    T2DeliverySystem(#[cfg_attr(feature = "serde", serde(borrow))] T2DeliverySystem<'a>),
    /// `0x06` — supplementary_audio (Table 153, §6.4.11).
    SupplementaryAudio(#[cfg_attr(feature = "serde", serde(borrow))] SupplementaryAudio<'a>),
    /// `0x07` — network_change_notify (Table 149, §6.4.9).
    NetworkChangeNotify(#[cfg_attr(feature = "serde", serde(borrow))] NetworkChangeNotify<'a>),
    /// `0x08` — message (Table 148, §6.4.9).
    Message(#[cfg_attr(feature = "serde", serde(borrow))] Message<'a>),
    /// `0x09` — target_region (Table 156, §6.4.12).
    TargetRegion(#[cfg_attr(feature = "serde", serde(borrow))] TargetRegion<'a>),
    /// `0x0A` — target_region_name (Table 157, §6.4.13).
    TargetRegionName(#[cfg_attr(feature = "serde", serde(borrow))] TargetRegionName<'a>),
    /// `0x0B` — service_relocated (Table 152, §6.4.10).
    ServiceRelocated(ServiceRelocated),
    /// `0x0D` — C2_delivery_system (Table 115, §6.4.6.1).
    C2DeliverySystem(C2DeliverySystem),
    /// `0x13` — URI_linkage (Table 159, §6.4.16.1).
    UriLinkage(#[cfg_attr(feature = "serde", serde(borrow))] UriLinkage<'a>),
    /// `0x15` — AC-4 (annex D).
    Ac4(#[cfg_attr(feature = "serde", serde(borrow))] Ac4<'a>),
    /// `0x16` — C2_bundle_delivery_system (Table 139, §6.4.6.4).
    C2BundleDeliverySystem(C2BundleDeliverySystem),
    /// `0x17` — S2X_satellite_delivery_system (Table 140, §6.4.6.5.2).
    S2XSatelliteDeliverySystem(
        #[cfg_attr(feature = "serde", serde(borrow))] S2XSatelliteDeliverySystem<'a>,
    ),
    /// `0x19` — audio_preselection (Table 110, §6.4.1).
    AudioPreselection(#[cfg_attr(feature = "serde", serde(borrow))] AudioPreselection<'a>),
    /// `0x20` — TTML_subtitling (EN 303 560 Table 1, §5.2.1.1).
    TtmlSubtitling(#[cfg_attr(feature = "serde", serde(borrow))] TtmlSubtitling<'a>),
    /// Any not-yet-typed / unknown / user-defined discriminant: selector bytes verbatim.
    Raw(#[cfg_attr(feature = "serde", serde(borrow))] &'a [u8]),
}

// ===========================================================================
//  Section 0x04 — T2_delivery_system_descriptor (Table 133, §6.4.6.3)
// ---------------------------------------------------------------------------
//  plp_id(8) T2_system_id(16) then, if descriptor_length > 4, a packed flags
//  block (SISO_MISO 2 | bandwidth 4 | reserved 2 ; guard 3 | tx_mode 3 | off 1 |
//  tfs 1) followed by a variable cell loop (cells carry tfs-conditional
//  frequency lists + subcell loops). The cell loop is length-irregular and is
//  kept raw per the SAT precedent; the always-present prefix is typed.
// ===========================================================================
/// T2_delivery_system body (Table 133). `cell_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct T2DeliverySystem<'a> {
    /// PLP identifier.
    pub plp_id: u8,
    /// T2 system identifier.
    pub t2_system_id: u16,
    /// SISO_MISO(2), present iff `descriptor_length > 4` (flags block present).
    pub siso_miso: Option<u8>,
    /// bandwidth(4), present with `siso_miso`.
    pub bandwidth: Option<u8>,
    /// guard_interval(3), present with `siso_miso`.
    pub guard_interval: Option<u8>,
    /// transmission_mode(3), present with `siso_miso`.
    pub transmission_mode: Option<u8>,
    /// other_frequency_flag(1), present with `siso_miso`.
    pub other_frequency_flag: Option<bool>,
    /// tfs_flag(1), present with `siso_miso`.
    pub tfs_flag: Option<bool>,
    /// Raw cell loop (Table 133 inner `for`), kept raw (SAT precedent).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub cell_loop: &'a [u8],
}

// ===========================================================================
//  Section 0x06 — supplementary_audio_descriptor (Table 153, §6.4.11)
// ===========================================================================
/// supplementary_audio body (Table 153).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct SupplementaryAudio<'a> {
    /// mix_type(1) — Table 154.
    pub mix_type: bool,
    /// editorial_classification(5) — Table 155.
    pub editorial_classification: u8,
    /// language_code_present(1).
    pub language_code_present: bool,
    /// ISO_639_language_code(24), present iff `language_code_present`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub iso_639_language_code: Option<&'a [u8]>,
    /// Trailing private_data_byte run.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

// ===========================================================================
//  Section 0x07 — network_change_notify_descriptor (Table 149, §6.4.9)
// ---------------------------------------------------------------------------
//  Two-level loop: per cell_id a length-delimited inner change loop whose
//  entries carry conditional invariant-TS fields. Kept raw (SAT precedent).
// ===========================================================================
/// network_change_notify body (Table 149); `cell_loop` is the raw outer loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct NetworkChangeNotify<'a> {
    /// Raw `for(cell)` loop body.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub cell_loop: &'a [u8],
}

// ===========================================================================
//  Section 0x08 — message_descriptor (Table 148, §6.4.9)
// ===========================================================================
/// message body (Table 148).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct Message<'a> {
    /// message_id(8).
    pub message_id: u8,
    /// ISO_639_language_code(24).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub iso_639_language_code: &'a [u8],
    /// text_char run (remainder of body).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub text: &'a [u8],
}

// ===========================================================================
//  Section 0x09 — target_region_descriptor (Table 156, §6.4.12)
// ---------------------------------------------------------------------------
//  Leading country_code(24) then a region loop whose entries are
//  region_depth-conditional; the loop is kept raw (SAT precedent).
// ===========================================================================
/// target_region body (Table 156); `region_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct TargetRegion<'a> {
    /// Leading country_code(24).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub country_code: &'a [u8],
    /// Raw region loop.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_loop: &'a [u8],
}

// ===========================================================================
//  Section 0x0A — target_region_name_descriptor (Table 157, §6.4.13)
// ===========================================================================
/// target_region_name body (Table 157); `region_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct TargetRegionName<'a> {
    /// country_code(24).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub country_code: &'a [u8],
    /// ISO_639_language_code(24).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub iso_639_language_code: &'a [u8],
    /// Raw region loop (length-delimited name entries).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub region_loop: &'a [u8],
}

// ===========================================================================
//  Section 0x0B — service_relocated_descriptor (Table 152, §6.4.10)
// ===========================================================================
/// service_relocated body (Table 152) — fully typed, fixed 6 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceRelocated {
    /// old_original_network_id(16).
    pub old_original_network_id: u16,
    /// old_transport_stream_id(16).
    pub old_transport_stream_id: u16,
    /// old_service_id(16).
    pub old_service_id: u16,
}

// ===========================================================================
//  Section 0x0D — C2_delivery_system_descriptor (Table 115, §6.4.6.1)
// ===========================================================================
/// C2_delivery_system body (Table 115) — fully typed, fixed 7 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct C2DeliverySystem {
    /// plp_id(8).
    pub plp_id: u8,
    /// data_slice_id(8).
    pub data_slice_id: u8,
    /// C2_System_tuning_frequency(32).
    pub c2_system_tuning_frequency: u32,
    /// C2_System_tuning_frequency_type(2).
    pub c2_system_tuning_frequency_type: u8,
    /// active_OFDM_symbol_duration(3).
    pub active_ofdm_symbol_duration: u8,
    /// guard_interval(3).
    pub guard_interval: u8,
}

// ===========================================================================
//  Section 0x13 — URI_linkage_descriptor (Table 159, §6.4.16.1)
// ---------------------------------------------------------------------------
//  uri_linkage_type, length-delimited URI, an optional min_polling_interval
//  (only for types 0x00/0x01), then trailing private_data. All typed.
// ===========================================================================
/// URI_linkage body (Table 159).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct UriLinkage<'a> {
    /// uri_linkage_type(8).
    pub uri_linkage_type: u8,
    /// Length-delimited URI bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub uri: &'a [u8],
    /// min_polling_interval(16), present iff `uri_linkage_type` is 0x00 or 0x01.
    pub min_polling_interval: Option<u16>,
    /// Trailing private_data_byte run.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

// ===========================================================================
//  Section 0x15 — AC-4_descriptor (annex D, §D.5)
// ---------------------------------------------------------------------------
//  Two flags + a packed config byte (when ac4_config_flag set), a
//  length-delimited TOC, then additional_info bytes. The TOC + extra are kept
//  raw; flags + config are typed.
// ===========================================================================
/// AC-4 body (annex D). `toc` + `additional_info` are raw.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct Ac4<'a> {
    /// ac4_config_flag(1).
    pub ac4_config_flag: bool,
    /// ac4_toc_flag(1).
    pub ac4_toc_flag: bool,
    /// ac4_dialog_enhancement_enabled(1), present iff `ac4_config_flag`.
    pub ac4_dialog_enhancement_enabled: Option<bool>,
    /// ac4_channel_mode(2), present iff `ac4_config_flag`.
    pub ac4_channel_mode: Option<u8>,
    /// Length-delimited ac4_dsi bytes, present iff `ac4_toc_flag`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub toc: Option<&'a [u8]>,
    /// Trailing additional_info_byte run.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub additional_info: &'a [u8],
}

// ===========================================================================
//  Section 0x16 — C2_bundle_delivery_system_descriptor (Table 139, §6.4.6.4)
// ---------------------------------------------------------------------------
//  A flat array of fixed 9-byte entries; fully typed.
// ===========================================================================
/// One C2 bundle entry (Table 139 inner loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct C2BundleEntry {
    /// plp_id(8).
    pub plp_id: u8,
    /// data_slice_id(8).
    pub data_slice_id: u8,
    /// C2_System_tuning_frequency(32).
    pub c2_system_tuning_frequency: u32,
    /// C2_System_tuning_frequency_type(2).
    pub c2_system_tuning_frequency_type: u8,
    /// active_OFDM_symbol_duration(3).
    pub active_ofdm_symbol_duration: u8,
    /// guard_interval(3).
    pub guard_interval: u8,
    /// primary_channel(1).
    pub primary_channel: bool,
}

/// C2_bundle_delivery_system body (Table 139) — fully typed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct C2BundleDeliverySystem {
    /// Bundle entries in wire order.
    pub entries: Vec<C2BundleEntry>,
}

// ===========================================================================
//  Section 0x17 — S2X_satellite_delivery_system_descriptor (Table 140, §6.4.6.5.2)
// ---------------------------------------------------------------------------
//  Primary-channel fields are typed. The S2X_mode==3 channel-bonding loop and
//  the trailing reserved_future_use bytes are irregular and kept raw (SAT
//  precedent); `tail` holds everything after the primary input_stream_identifier
//  / timeslice_number.
// ===========================================================================
/// S2X_satellite_delivery_system body (Table 140); `tail` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct S2XSatelliteDeliverySystem<'a> {
    /// receiver_profiles(5) — Table 141.
    pub receiver_profiles: u8,
    /// S2X_mode(2) — Table 142.
    pub s2x_mode: u8,
    /// scrambling_sequence_selector(1).
    pub scrambling_sequence_selector: bool,
    /// TS_GS_S2X_mode(2) — Table 143.
    pub ts_gs_s2x_mode: u8,
    /// scrambling_sequence_index(18), present iff `scrambling_sequence_selector`.
    pub scrambling_sequence_index: Option<u32>,
    /// frequency(32) — primary channel.
    pub frequency: u32,
    /// orbital_position(16).
    pub orbital_position: u16,
    /// west_east_flag(1).
    pub west_east_flag: bool,
    /// polarization(2).
    pub polarization: u8,
    /// multiple_input_stream_flag(1).
    pub multiple_input_stream_flag: bool,
    /// roll_off(3) — Table 144.
    pub roll_off: u8,
    /// symbol_rate(28).
    pub symbol_rate: u32,
    /// input_stream_identifier(8), present iff `multiple_input_stream_flag`.
    pub input_stream_identifier: Option<u8>,
    /// timeslice_number(8), present iff `s2x_mode == 2`.
    pub timeslice_number: Option<u8>,
    /// Raw remainder: S2X_mode==3 channel-bond loop + reserved tail.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub tail: &'a [u8],
}

// ===========================================================================
//  Section 0x19 — audio_preselection_descriptor (Table 110, §6.4.1)
// ---------------------------------------------------------------------------
//  num_preselections then a variable preselection loop whose entries carry
//  conditional language / message / aux-component / future-extension fields.
//  The loop is kept raw (SAT precedent); the count byte is typed.
// ===========================================================================
/// audio_preselection body (Table 110); `preselection_loop` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct AudioPreselection<'a> {
    /// num_preselections(5).
    pub num_preselections: u8,
    /// Raw preselection loop.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub preselection_loop: &'a [u8],
}

// ===========================================================================
//  Section 0x20 — TTML_subtitling_descriptor (EN 303 560 Table 1, §5.2.1.1)
// ---------------------------------------------------------------------------
//  Fixed lead-in, a profile array, optional qualifier(32), optional font list,
//  a length-delimited text field, then trailing reserved bytes. The profile
//  list, optional qualifier, font list, text and trailing reserved bytes are
//  kept raw (`tail`); the fixed lead-in is typed.
// ===========================================================================
/// TTML_subtitling body (EN 303 560 Table 1); `tail` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct TtmlSubtitling<'a> {
    /// ISO_639_language_code(24).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub iso_639_language_code: &'a [u8],
    /// subtitle_purpose(6) — Table 2.
    pub subtitle_purpose: u8,
    /// TTS_suitability(2) — Table 3.
    pub tts_suitability: u8,
    /// essential_font_usage_flag(1).
    pub essential_font_usage_flag: bool,
    /// qualifier_present_flag(1).
    pub qualifier_present_flag: bool,
    /// dvb_ttml_profile_count(4).
    pub dvb_ttml_profile_count: u8,
    /// Raw remainder: profile list + optional qualifier + font list + text + reserved.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub tail: &'a [u8],
}

/// Extension descriptor (EN 300 468 Table 54, §6.2.18.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "'de: 'a")))]
pub struct ExtensionDescriptor<'a> {
    /// `descriptor_tag_extension` (raw `u8`; see [`ExtensionTag`] for names).
    pub tag_extension: u8,
    /// Typed body, or [`ExtensionBody::Raw`] for not-yet-typed discriminants.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub body: ExtensionBody<'a>,
}

impl ExtensionDescriptor<'_> {
    /// Typed view of [`Self::tag_extension`], or `None` if not a known tag.
    #[must_use]
    pub fn kind(&self) -> Option<ExtensionTag> {
        Some(match self.tag_extension {
            0x00 => ExtensionTag::ImageIcon,
            0x04 => ExtensionTag::T2DeliverySystem,
            0x06 => ExtensionTag::SupplementaryAudio,
            0x07 => ExtensionTag::NetworkChangeNotify,
            0x08 => ExtensionTag::Message,
            0x09 => ExtensionTag::TargetRegion,
            0x0A => ExtensionTag::TargetRegionName,
            0x0B => ExtensionTag::ServiceRelocated,
            0x0D => ExtensionTag::C2DeliverySystem,
            0x13 => ExtensionTag::UriLinkage,
            0x15 => ExtensionTag::Ac4,
            0x16 => ExtensionTag::C2BundleDeliverySystem,
            0x17 => ExtensionTag::S2XSatelliteDeliverySystem,
            0x19 => ExtensionTag::AudioPreselection,
            0x20 => ExtensionTag::TtmlSubtitling,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Body parsers (each consumes the selector bytes after descriptor_tag_extension)
// ---------------------------------------------------------------------------

fn invalid(reason: &'static str) -> Error {
    Error::InvalidDescriptor { tag: TAG, reason }
}

fn parse_t2(sel: &[u8]) -> Result<T2DeliverySystem<'_>> {
    if sel.len() < T2_FIXED_PREFIX_LEN {
        return Err(invalid("T2_delivery_system: prefix truncated"));
    }
    let plp_id = sel[0];
    let t2_system_id = u16::from_be_bytes([sel[1], sel[2]]);
    let mut pos = T2_FIXED_PREFIX_LEN;
    // descriptor_length > 4 ⇔ the optional packed flags block is present
    // (the body is plp + system_id = 3 bytes when absent; >3 ⇒ block present).
    let (siso_miso, bandwidth, guard_interval, transmission_mode, other_frequency_flag, tfs_flag) =
        if sel.len() > T2_FIXED_PREFIX_LEN {
            if sel.len() < T2_FIXED_PREFIX_LEN + T2_FLAGS_BLOCK_LEN {
                return Err(invalid("T2_delivery_system: flags block truncated"));
            }
            let b0 = sel[pos];
            let b1 = sel[pos + 1];
            pos += T2_FLAGS_BLOCK_LEN;
            (
                Some(b0 >> 6),
                Some((b0 >> 2) & 0x0F),
                Some(b1 >> 5),
                Some((b1 >> 2) & 0x07),
                Some((b1 & 0x02) != 0),
                Some((b1 & 0x01) != 0),
            )
        } else {
            (None, None, None, None, None, None)
        };
    Ok(T2DeliverySystem {
        plp_id,
        t2_system_id,
        siso_miso,
        bandwidth,
        guard_interval,
        transmission_mode,
        other_frequency_flag,
        tfs_flag,
        cell_loop: &sel[pos..],
    })
}

fn parse_supplementary_audio(sel: &[u8]) -> Result<SupplementaryAudio<'_>> {
    if sel.is_empty() {
        return Err(invalid("supplementary_audio: flags byte missing"));
    }
    let flags = sel[0];
    let mix_type = (flags & 0x80) != 0;
    let editorial_classification = (flags >> 2) & 0x1F;
    let language_code_present = (flags & 0x01) != 0;
    let mut pos = 1;
    let iso_639_language_code = if language_code_present {
        if sel.len() < pos + ISO_639_LEN {
            return Err(invalid("supplementary_audio: language code truncated"));
        }
        let lc = &sel[pos..pos + ISO_639_LEN];
        pos += ISO_639_LEN;
        Some(lc)
    } else {
        None
    };
    Ok(SupplementaryAudio {
        mix_type,
        editorial_classification,
        language_code_present,
        iso_639_language_code,
        private_data: &sel[pos..],
    })
}

fn parse_message(sel: &[u8]) -> Result<Message<'_>> {
    if sel.len() < 1 + ISO_639_LEN {
        return Err(invalid("message: header truncated"));
    }
    Ok(Message {
        message_id: sel[0],
        iso_639_language_code: &sel[1..1 + ISO_639_LEN],
        text: &sel[1 + ISO_639_LEN..],
    })
}

fn parse_target_region(sel: &[u8]) -> Result<TargetRegion<'_>> {
    if sel.len() < ISO_639_LEN {
        return Err(invalid("target_region: country_code truncated"));
    }
    Ok(TargetRegion {
        country_code: &sel[..ISO_639_LEN],
        region_loop: &sel[ISO_639_LEN..],
    })
}

fn parse_target_region_name(sel: &[u8]) -> Result<TargetRegionName<'_>> {
    if sel.len() < 2 * ISO_639_LEN {
        return Err(invalid("target_region_name: header truncated"));
    }
    Ok(TargetRegionName {
        country_code: &sel[..ISO_639_LEN],
        iso_639_language_code: &sel[ISO_639_LEN..2 * ISO_639_LEN],
        region_loop: &sel[2 * ISO_639_LEN..],
    })
}

fn parse_service_relocated(sel: &[u8]) -> Result<ServiceRelocated> {
    if sel.len() < SERVICE_RELOCATED_LEN {
        return Err(invalid("service_relocated: truncated"));
    }
    Ok(ServiceRelocated {
        old_original_network_id: u16::from_be_bytes([sel[0], sel[1]]),
        old_transport_stream_id: u16::from_be_bytes([sel[2], sel[3]]),
        old_service_id: u16::from_be_bytes([sel[4], sel[5]]),
    })
}

fn parse_c2(sel: &[u8]) -> Result<C2DeliverySystem> {
    if sel.len() < C2_LEN {
        return Err(invalid("C2_delivery_system: truncated"));
    }
    let packed = sel[6];
    Ok(C2DeliverySystem {
        plp_id: sel[0],
        data_slice_id: sel[1],
        c2_system_tuning_frequency: u32::from_be_bytes([sel[2], sel[3], sel[4], sel[5]]),
        c2_system_tuning_frequency_type: packed >> 6,
        active_ofdm_symbol_duration: (packed >> 3) & 0x07,
        guard_interval: packed & 0x07,
    })
}

fn parse_uri_linkage(sel: &[u8]) -> Result<UriLinkage<'_>> {
    if sel.len() < 2 {
        return Err(invalid("URI_linkage: header truncated"));
    }
    let uri_linkage_type = sel[0];
    let uri_length = sel[1] as usize;
    let mut pos = 2;
    if sel.len() < pos + uri_length {
        return Err(invalid("URI_linkage: uri overruns body"));
    }
    let uri = &sel[pos..pos + uri_length];
    pos += uri_length;
    let min_polling_interval = if uri_linkage_type == 0x00 || uri_linkage_type == 0x01 {
        if sel.len() < pos + 2 {
            return Err(invalid("URI_linkage: min_polling_interval truncated"));
        }
        let v = u16::from_be_bytes([sel[pos], sel[pos + 1]]);
        pos += 2;
        Some(v)
    } else {
        None
    };
    Ok(UriLinkage {
        uri_linkage_type,
        uri,
        min_polling_interval,
        private_data: &sel[pos..],
    })
}

fn parse_ac4(sel: &[u8]) -> Result<Ac4<'_>> {
    if sel.is_empty() {
        return Err(invalid("AC-4: flags byte missing"));
    }
    let flags = sel[0];
    let ac4_config_flag = (flags & 0x80) != 0;
    let ac4_toc_flag = (flags & 0x40) != 0;
    let mut pos = 1;
    let (ac4_dialog_enhancement_enabled, ac4_channel_mode) = if ac4_config_flag {
        if sel.len() < pos + 1 {
            return Err(invalid("AC-4: config byte truncated"));
        }
        let c = sel[pos];
        pos += 1;
        (Some((c & 0x80) != 0), Some((c >> 5) & 0x03))
    } else {
        (None, None)
    };
    let toc = if ac4_toc_flag {
        if sel.len() < pos + 1 {
            return Err(invalid("AC-4: toc length truncated"));
        }
        let toc_len = sel[pos] as usize;
        pos += 1;
        if sel.len() < pos + toc_len {
            return Err(invalid("AC-4: toc overruns body"));
        }
        let t = &sel[pos..pos + toc_len];
        pos += toc_len;
        Some(t)
    } else {
        None
    };
    Ok(Ac4 {
        ac4_config_flag,
        ac4_toc_flag,
        ac4_dialog_enhancement_enabled,
        ac4_channel_mode,
        toc,
        additional_info: &sel[pos..],
    })
}

fn parse_c2_bundle(sel: &[u8]) -> Result<C2BundleDeliverySystem> {
    if sel.len() % C2_BUNDLE_ENTRY_LEN != 0 {
        return Err(invalid(
            "C2_bundle_delivery_system: not a whole number of entries",
        ));
    }
    let mut entries = Vec::with_capacity(sel.len() / C2_BUNDLE_ENTRY_LEN);
    for chunk in sel.chunks_exact(C2_BUNDLE_ENTRY_LEN) {
        let packed = chunk[6];
        entries.push(C2BundleEntry {
            plp_id: chunk[0],
            data_slice_id: chunk[1],
            c2_system_tuning_frequency: u32::from_be_bytes([
                chunk[2], chunk[3], chunk[4], chunk[5],
            ]),
            c2_system_tuning_frequency_type: packed >> 6,
            active_ofdm_symbol_duration: (packed >> 3) & 0x07,
            guard_interval: packed & 0x07,
            primary_channel: (chunk[7] & 0x80) != 0,
        });
    }
    Ok(C2BundleDeliverySystem { entries })
}

fn parse_s2x(sel: &[u8]) -> Result<S2XSatelliteDeliverySystem<'_>> {
    // receiver_profiles byte + S2X mode/flags byte = 2 fixed bytes.
    if sel.len() < 2 {
        return Err(invalid("S2X: flags truncated"));
    }
    let receiver_profiles = sel[0] >> 3;
    let b1 = sel[1];
    // Table 140 byte 1, MSB-first: S2X_mode(2) scrambling_sequence_selector(1)
    // reserved_zero_future_use(3) TS_GS_S2X_mode(2).
    let s2x_mode = (b1 >> 6) & 0x03;
    let scrambling_sequence_selector = (b1 & 0x20) != 0;
    let ts_gs_s2x_mode = b1 & 0x03;
    let mut pos = 2;
    let scrambling_sequence_index = if scrambling_sequence_selector {
        if sel.len() < pos + S2X_SCRAMBLING_LEN {
            return Err(invalid("S2X: scrambling_sequence_index truncated"));
        }
        let idx = (u32::from(sel[pos] & 0x03) << 16)
            | (u32::from(sel[pos + 1]) << 8)
            | u32::from(sel[pos + 2]);
        pos += S2X_SCRAMBLING_LEN;
        Some(idx)
    } else {
        None
    };
    // Primary channel (Table 140): frequency(32) orbital_position(16)
    //   packed byte = west_east(1) polarization(2) mis(1) reserved(1) roll_off(3)
    //   then reserved(4) | symbol_rate[27:24], and 3 bytes symbol_rate[23:0].
    if sel.len() < pos + S2X_PRIMARY_LEN {
        return Err(invalid("S2X: primary channel truncated"));
    }
    let frequency = u32::from_be_bytes([sel[pos], sel[pos + 1], sel[pos + 2], sel[pos + 3]]);
    let orbital_position = u16::from_be_bytes([sel[pos + 4], sel[pos + 5]]);
    let pb = sel[pos + 6];
    let west_east_flag = (pb & 0x80) != 0;
    let polarization = (pb >> 5) & 0x03;
    let multiple_input_stream_flag = (pb & 0x10) != 0;
    let roll_off = pb & 0x07;
    let symbol_rate = (u32::from(sel[pos + 7] & 0x0F) << 24)
        | (u32::from(sel[pos + 8]) << 16)
        | (u32::from(sel[pos + 9]) << 8)
        | u32::from(sel[pos + 10]);
    pos += S2X_PRIMARY_LEN;
    let input_stream_identifier = if multiple_input_stream_flag {
        if sel.len() < pos + 1 {
            return Err(invalid("S2X: input_stream_identifier truncated"));
        }
        let isi = sel[pos];
        pos += 1;
        Some(isi)
    } else {
        None
    };
    let timeslice_number = if s2x_mode == 2 {
        if sel.len() < pos + 1 {
            return Err(invalid("S2X: timeslice_number truncated"));
        }
        let ts = sel[pos];
        pos += 1;
        Some(ts)
    } else {
        None
    };
    Ok(S2XSatelliteDeliverySystem {
        receiver_profiles,
        s2x_mode,
        scrambling_sequence_selector,
        ts_gs_s2x_mode,
        scrambling_sequence_index,
        frequency,
        orbital_position,
        west_east_flag,
        polarization,
        multiple_input_stream_flag,
        roll_off,
        symbol_rate,
        input_stream_identifier,
        timeslice_number,
        tail: &sel[pos..],
    })
}

fn parse_audio_preselection(sel: &[u8]) -> Result<AudioPreselection<'_>> {
    if sel.is_empty() {
        return Err(invalid("audio_preselection: count byte missing"));
    }
    Ok(AudioPreselection {
        num_preselections: sel[0] >> 3,
        preselection_loop: &sel[1..],
    })
}

fn parse_ttml(sel: &[u8]) -> Result<TtmlSubtitling<'_>> {
    if sel.len() < TTML_FIXED_LEN {
        return Err(invalid("TTML_subtitling: header truncated"));
    }
    let b3 = sel[ISO_639_LEN];
    let b4 = sel[ISO_639_LEN + 1];
    Ok(TtmlSubtitling {
        iso_639_language_code: &sel[..ISO_639_LEN],
        subtitle_purpose: b3 >> 2,
        tts_suitability: b3 & 0x03,
        essential_font_usage_flag: (b4 & 0x80) != 0,
        qualifier_present_flag: (b4 & 0x40) != 0,
        dvb_ttml_profile_count: b4 & 0x0F,
        tail: &sel[TTML_FIXED_LEN..],
    })
}

fn parse_body(tag_extension: u8, sel: &[u8]) -> Result<ExtensionBody<'_>> {
    Ok(match tag_extension {
        0x04 => ExtensionBody::T2DeliverySystem(parse_t2(sel)?),
        0x06 => ExtensionBody::SupplementaryAudio(parse_supplementary_audio(sel)?),
        0x07 => ExtensionBody::NetworkChangeNotify(NetworkChangeNotify { cell_loop: sel }),
        0x08 => ExtensionBody::Message(parse_message(sel)?),
        0x09 => ExtensionBody::TargetRegion(parse_target_region(sel)?),
        0x0A => ExtensionBody::TargetRegionName(parse_target_region_name(sel)?),
        0x0B => ExtensionBody::ServiceRelocated(parse_service_relocated(sel)?),
        0x0D => ExtensionBody::C2DeliverySystem(parse_c2(sel)?),
        0x13 => ExtensionBody::UriLinkage(parse_uri_linkage(sel)?),
        0x15 => ExtensionBody::Ac4(parse_ac4(sel)?),
        0x16 => ExtensionBody::C2BundleDeliverySystem(parse_c2_bundle(sel)?),
        0x17 => ExtensionBody::S2XSatelliteDeliverySystem(parse_s2x(sel)?),
        0x19 => ExtensionBody::AudioPreselection(parse_audio_preselection(sel)?),
        0x20 => ExtensionBody::TtmlSubtitling(parse_ttml(sel)?),
        _ => ExtensionBody::Raw(sel),
    })
}

impl<'a> Parse<'a> for ExtensionDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "ExtensionDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for extension_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "ExtensionDescriptor body",
            });
        }
        if length < MIN_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body must contain at least the descriptor_tag_extension byte",
            });
        }
        let tag_extension = bytes[HEADER_LEN];
        let sel = &bytes[HEADER_LEN + TAG_EXTENSION_LEN..end];
        let body = parse_body(tag_extension, sel)?;
        Ok(Self {
            tag_extension,
            body,
        })
    }
}

// ---------------------------------------------------------------------------
//  Body serializers — report selector length + write the selector bytes
// ---------------------------------------------------------------------------

impl ExtensionBody<'_> {
    /// Selector-byte length (everything after `descriptor_tag_extension`).
    fn selector_len(&self) -> usize {
        match self {
            ExtensionBody::T2DeliverySystem(b) => {
                T2_FIXED_PREFIX_LEN
                    + if b.siso_miso.is_some() {
                        T2_FLAGS_BLOCK_LEN
                    } else {
                        0
                    }
                    + b.cell_loop.len()
            }
            ExtensionBody::SupplementaryAudio(b) => {
                1 + b.iso_639_language_code.map_or(0, <[u8]>::len) + b.private_data.len()
            }
            ExtensionBody::NetworkChangeNotify(b) => b.cell_loop.len(),
            ExtensionBody::Message(b) => 1 + ISO_639_LEN + b.text.len(),
            ExtensionBody::TargetRegion(b) => ISO_639_LEN + b.region_loop.len(),
            ExtensionBody::TargetRegionName(b) => 2 * ISO_639_LEN + b.region_loop.len(),
            ExtensionBody::ServiceRelocated(_) => SERVICE_RELOCATED_LEN,
            ExtensionBody::C2DeliverySystem(_) => C2_LEN,
            ExtensionBody::UriLinkage(b) => {
                2 + b.uri.len()
                    + if b.min_polling_interval.is_some() {
                        2
                    } else {
                        0
                    }
                    + b.private_data.len()
            }
            ExtensionBody::Ac4(b) => {
                1 + usize::from(b.ac4_config_flag)
                    + b.toc.map_or(0, |t| 1 + t.len())
                    + b.additional_info.len()
            }
            ExtensionBody::C2BundleDeliverySystem(b) => b.entries.len() * C2_BUNDLE_ENTRY_LEN,
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                2 + if b.scrambling_sequence_selector {
                    S2X_SCRAMBLING_LEN
                } else {
                    0
                } + S2X_PRIMARY_LEN
                    + usize::from(b.input_stream_identifier.is_some())
                    + usize::from(b.timeslice_number.is_some())
                    + b.tail.len()
            }
            ExtensionBody::AudioPreselection(b) => 1 + b.preselection_loop.len(),
            ExtensionBody::TtmlSubtitling(b) => TTML_FIXED_LEN + b.tail.len(),
            ExtensionBody::Raw(s) => s.len(),
        }
    }

    /// Write the selector bytes into `out` (assumed `>= selector_len()`).
    fn write_selector(&self, out: &mut [u8]) {
        match self {
            ExtensionBody::T2DeliverySystem(b) => {
                out[0] = b.plp_id;
                out[1..3].copy_from_slice(&b.t2_system_id.to_be_bytes());
                let mut p = T2_FIXED_PREFIX_LEN;
                if let (Some(sm), Some(bw), Some(gi), Some(tm), Some(off), Some(tfs)) = (
                    b.siso_miso,
                    b.bandwidth,
                    b.guard_interval,
                    b.transmission_mode,
                    b.other_frequency_flag,
                    b.tfs_flag,
                ) {
                    out[p] = (sm << 6) | ((bw & 0x0F) << 2);
                    out[p + 1] =
                        (gi << 5) | ((tm & 0x07) << 2) | (u8::from(off) << 1) | u8::from(tfs);
                    p += T2_FLAGS_BLOCK_LEN;
                }
                out[p..p + b.cell_loop.len()].copy_from_slice(b.cell_loop);
            }
            ExtensionBody::SupplementaryAudio(b) => {
                // Table 153 bit 1 is plain reserved_future_use → emitted as 1.
                out[0] = (u8::from(b.mix_type) << 7)
                    | ((b.editorial_classification & 0x1F) << 2)
                    | 0x02
                    | u8::from(b.language_code_present);
                let mut p = 1;
                if let Some(lc) = b.iso_639_language_code {
                    out[p..p + lc.len()].copy_from_slice(lc);
                    p += lc.len();
                }
                out[p..p + b.private_data.len()].copy_from_slice(b.private_data);
            }
            ExtensionBody::NetworkChangeNotify(b) => {
                out[..b.cell_loop.len()].copy_from_slice(b.cell_loop);
            }
            ExtensionBody::Message(b) => {
                out[0] = b.message_id;
                out[1..1 + ISO_639_LEN].copy_from_slice(b.iso_639_language_code);
                out[1 + ISO_639_LEN..1 + ISO_639_LEN + b.text.len()].copy_from_slice(b.text);
            }
            ExtensionBody::TargetRegion(b) => {
                out[..ISO_639_LEN].copy_from_slice(b.country_code);
                out[ISO_639_LEN..ISO_639_LEN + b.region_loop.len()].copy_from_slice(b.region_loop);
            }
            ExtensionBody::TargetRegionName(b) => {
                out[..ISO_639_LEN].copy_from_slice(b.country_code);
                out[ISO_639_LEN..2 * ISO_639_LEN].copy_from_slice(b.iso_639_language_code);
                out[2 * ISO_639_LEN..2 * ISO_639_LEN + b.region_loop.len()]
                    .copy_from_slice(b.region_loop);
            }
            ExtensionBody::ServiceRelocated(b) => {
                out[0..2].copy_from_slice(&b.old_original_network_id.to_be_bytes());
                out[2..4].copy_from_slice(&b.old_transport_stream_id.to_be_bytes());
                out[4..6].copy_from_slice(&b.old_service_id.to_be_bytes());
            }
            ExtensionBody::C2DeliverySystem(b) => {
                out[0] = b.plp_id;
                out[1] = b.data_slice_id;
                out[2..6].copy_from_slice(&b.c2_system_tuning_frequency.to_be_bytes());
                out[6] = (b.c2_system_tuning_frequency_type << 6)
                    | ((b.active_ofdm_symbol_duration & 0x07) << 3)
                    | (b.guard_interval & 0x07);
            }
            ExtensionBody::UriLinkage(b) => {
                out[0] = b.uri_linkage_type;
                out[1] = b.uri.len() as u8;
                let mut p = 2;
                out[p..p + b.uri.len()].copy_from_slice(b.uri);
                p += b.uri.len();
                if let Some(mpi) = b.min_polling_interval {
                    out[p..p + 2].copy_from_slice(&mpi.to_be_bytes());
                    p += 2;
                }
                out[p..p + b.private_data.len()].copy_from_slice(b.private_data);
            }
            ExtensionBody::Ac4(b) => {
                out[0] = (u8::from(b.ac4_config_flag) << 7) | (u8::from(b.ac4_toc_flag) << 6);
                let mut p = 1;
                if b.ac4_config_flag {
                    let de = b.ac4_dialog_enhancement_enabled.unwrap_or(false);
                    let cm = b.ac4_channel_mode.unwrap_or(0) & 0x03;
                    out[p] = (u8::from(de) << 7) | (cm << 5);
                    p += 1;
                }
                if let Some(t) = b.toc {
                    out[p] = t.len() as u8;
                    p += 1;
                    out[p..p + t.len()].copy_from_slice(t);
                    p += t.len();
                }
                out[p..p + b.additional_info.len()].copy_from_slice(b.additional_info);
            }
            ExtensionBody::C2BundleDeliverySystem(b) => {
                let mut p = 0;
                for e in &b.entries {
                    out[p] = e.plp_id;
                    out[p + 1] = e.data_slice_id;
                    out[p + 2..p + 6].copy_from_slice(&e.c2_system_tuning_frequency.to_be_bytes());
                    out[p + 6] = (e.c2_system_tuning_frequency_type << 6)
                        | ((e.active_ofdm_symbol_duration & 0x07) << 3)
                        | (e.guard_interval & 0x07);
                    out[p + 7] = u8::from(e.primary_channel) << 7;
                    p += C2_BUNDLE_ENTRY_LEN;
                }
            }
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                out[0] = b.receiver_profiles << 3;
                out[1] = ((b.s2x_mode & 0x03) << 6)
                    | (u8::from(b.scrambling_sequence_selector) << 5)
                    | (b.ts_gs_s2x_mode & 0x03);
                let mut p = 2;
                if b.scrambling_sequence_selector {
                    let idx = b.scrambling_sequence_index.unwrap_or(0) & 0x3FFFF;
                    out[p] = (idx >> 16) as u8 & 0x03;
                    out[p + 1] = (idx >> 8) as u8;
                    out[p + 2] = idx as u8;
                    p += S2X_SCRAMBLING_LEN;
                }
                out[p..p + 4].copy_from_slice(&b.frequency.to_be_bytes());
                out[p + 4..p + 6].copy_from_slice(&b.orbital_position.to_be_bytes());
                out[p + 6] = (u8::from(b.west_east_flag) << 7)
                    | ((b.polarization & 0x03) << 5)
                    | (u8::from(b.multiple_input_stream_flag) << 4)
                    | (b.roll_off & 0x07);
                let sr = b.symbol_rate & 0x0FFF_FFFF;
                out[p + 7] = (sr >> 24) as u8 & 0x0F;
                out[p + 8] = (sr >> 16) as u8;
                out[p + 9] = (sr >> 8) as u8;
                out[p + 10] = sr as u8;
                p += S2X_PRIMARY_LEN;
                if let Some(isi) = b.input_stream_identifier {
                    out[p] = isi;
                    p += 1;
                }
                if let Some(ts) = b.timeslice_number {
                    out[p] = ts;
                    p += 1;
                }
                out[p..p + b.tail.len()].copy_from_slice(b.tail);
            }
            ExtensionBody::AudioPreselection(b) => {
                out[0] = b.num_preselections << 3;
                out[1..1 + b.preselection_loop.len()].copy_from_slice(b.preselection_loop);
            }
            ExtensionBody::TtmlSubtitling(b) => {
                out[..ISO_639_LEN].copy_from_slice(b.iso_639_language_code);
                out[ISO_639_LEN] = (b.subtitle_purpose << 2) | (b.tts_suitability & 0x03);
                out[ISO_639_LEN + 1] = (u8::from(b.essential_font_usage_flag) << 7)
                    | (u8::from(b.qualifier_present_flag) << 6)
                    | (b.dvb_ttml_profile_count & 0x0F);
                out[TTML_FIXED_LEN..TTML_FIXED_LEN + b.tail.len()].copy_from_slice(b.tail);
            }
            ExtensionBody::Raw(s) => out[..s.len()].copy_from_slice(s),
        }
    }
}

impl Serialize for ExtensionDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + TAG_EXTENSION_LEN + self.body.selector_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let body_len = len - HEADER_LEN;
        if body_len > MAX_DESCRIPTOR_LENGTH {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length exceeds 255 bytes",
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[HEADER_LEN] = self.tag_extension;
        self.body
            .write_selector(&mut buf[HEADER_LEN + TAG_EXTENSION_LEN..len]);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for ExtensionDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap selector bytes in the extension descriptor framing (Table 54).
    fn wrap(tag_ext: u8, sel: &[u8]) -> Vec<u8> {
        let mut v = vec![TAG, (sel.len() + 1) as u8, tag_ext];
        v.extend_from_slice(sel);
        v
    }

    fn round_trip(d: &ExtensionDescriptor) {
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ExtensionDescriptor::parse(&buf).unwrap();
        assert_eq!(*d, re);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let raw = [0x43, 1, 0x04];
        assert!(matches!(
            ExtensionDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x43, .. }
        ));
    }

    #[test]
    fn parse_rejects_empty_body() {
        let raw = [TAG, 0];
        assert!(matches!(
            ExtensionDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        // declares length 3 but only 1 body byte present
        let raw = [TAG, 3, 0x08];
        assert!(matches!(
            ExtensionDescriptor::parse(&raw).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn unknown_tag_round_trips_as_raw() {
        // 0x42 is reserved/unknown — must survive as Raw with bytes preserved.
        let sel = [0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = wrap(0x42, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.tag_extension, 0x42);
        assert_eq!(d.kind(), None);
        assert!(matches!(d.body, ExtensionBody::Raw(b) if b == sel));
        round_trip(&d);
    }

    #[test]
    fn user_defined_tag_preserved() {
        let bytes = wrap(0x90, &[0x01, 0x02]);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.tag_extension, 0x90);
        assert!(matches!(d.body, ExtensionBody::Raw(_)));
        round_trip(&d);
    }

    #[test]
    fn parse_service_relocated() {
        let sel = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let bytes = wrap(0x0B, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ServiceRelocated));
        match &d.body {
            ExtensionBody::ServiceRelocated(b) => {
                assert_eq!(b.old_original_network_id, 0x1234);
                assert_eq!(b.old_transport_stream_id, 0x5678);
                assert_eq!(b.old_service_id, 0x9ABC);
            }
            other => panic!("expected ServiceRelocated, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_message() {
        let sel = [0x07, b'e', b'n', b'g', b'H', b'i'];
        let bytes = wrap(0x08, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Message(b) => {
                assert_eq!(b.message_id, 0x07);
                assert_eq!(b.iso_639_language_code, b"eng");
                assert_eq!(b.text, b"Hi");
            }
            other => panic!("expected Message, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_supplementary_audio_with_language() {
        // mix_type=1, editorial=0x17, reserved=1, language_code_present=1,
        // then "fre", private 0xAA
        let flags = 0x80 | (0x17 << 2) | 0x02 | 0x01;
        let sel = [flags, b'f', b'r', b'e', 0xAA];
        let bytes = wrap(0x06, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::SupplementaryAudio(b) => {
                assert!(b.mix_type);
                assert_eq!(b.editorial_classification, 0x17);
                assert!(b.language_code_present);
                assert_eq!(b.iso_639_language_code, Some(b"fre".as_slice()));
                assert_eq!(b.private_data, &[0xAA]);
            }
            other => panic!("expected SupplementaryAudio, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_supplementary_audio_no_language() {
        let flags = ((0x01 << 2) & 0x7C) | 0x02; // mix=0, editorial=1, reserved=1, lang=0
        let sel = [flags];
        let bytes = wrap(0x06, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::SupplementaryAudio(b) => {
                assert!(!b.language_code_present);
                assert_eq!(b.iso_639_language_code, None);
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected SupplementaryAudio, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_delivery_system() {
        let packed = (0x02 << 6) | (0x01 << 3) | 0x01;
        let sel = [0x05, 0x09, 0x12, 0x34, 0x56, 0x78, packed];
        let bytes = wrap(0x0D, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x05);
                assert_eq!(b.data_slice_id, 0x09);
                assert_eq!(b.c2_system_tuning_frequency, 0x1234_5678);
                assert_eq!(b.c2_system_tuning_frequency_type, 0x02);
                assert_eq!(b.active_ofdm_symbol_duration, 0x01);
                assert_eq!(b.guard_interval, 0x01);
            }
            other => panic!("expected C2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_bundle_two_entries() {
        let entry = |off: u8| {
            let packed = (0x01u8 << 6) | 0x01; // freq_type=1, ofdm=0, guard=1
                                               // 8 bytes per Table 139: ... + primary_channel(1)+reserved_zero(7)
            [off, off + 1, 0x00, 0x00, 0x10, 0x00, packed, 0x80]
        };
        let mut sel = Vec::new();
        sel.extend_from_slice(&entry(0x01));
        sel.extend_from_slice(&entry(0x05));
        let bytes = wrap(0x16, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::C2BundleDeliverySystem(b) => {
                assert_eq!(b.entries.len(), 2);
                assert_eq!(b.entries[0].plp_id, 0x01);
                assert!(b.entries[0].primary_channel);
                assert_eq!(b.entries[1].plp_id, 0x05);
                assert_eq!(b.entries[1].guard_interval, 0x01);
            }
            other => panic!("expected C2BundleDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_c2_bundle_rejects_partial_entry() {
        let sel = [0x01, 0x02, 0x03]; // 3 bytes, not a multiple of 8
        let bytes = wrap(0x16, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_uri_linkage_with_polling() {
        let uri = b"http://x";
        let mut sel = vec![0x00, uri.len() as u8];
        sel.extend_from_slice(uri);
        sel.extend_from_slice(&0x1234u16.to_be_bytes());
        sel.push(0xFE); // private
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.uri_linkage_type, 0x00);
                assert_eq!(b.uri, uri);
                assert_eq!(b.min_polling_interval, Some(0x1234));
                assert_eq!(b.private_data, &[0xFE]);
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_no_polling() {
        // type 0x02 ⇒ no min_polling_interval
        let uri = b"dvb:";
        let mut sel = vec![0x02, uri.len() as u8];
        sel.extend_from_slice(uri);
        let bytes = wrap(0x13, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::UriLinkage(b) => {
                assert_eq!(b.min_polling_interval, None);
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected UriLinkage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_uri_linkage_rejects_overrun() {
        let sel = [0x02, 0x10, 0xAA]; // uri_length 16 but 1 byte present
        let bytes = wrap(0x13, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_ac4_full() {
        // config_flag=1, toc_flag=1; config byte de=1 cm=2; toc len 2 = [0x11,0x22]; extra 0x33
        let sel = [0xC0, 0x80 | (0x02 << 5), 0x02, 0x11, 0x22, 0x33];
        let bytes = wrap(0x15, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Ac4(b) => {
                assert!(b.ac4_config_flag);
                assert!(b.ac4_toc_flag);
                assert_eq!(b.ac4_dialog_enhancement_enabled, Some(true));
                assert_eq!(b.ac4_channel_mode, Some(0x02));
                assert_eq!(b.toc, Some([0x11u8, 0x22].as_slice()));
                assert_eq!(b.additional_info, &[0x33]);
            }
            other => panic!("expected Ac4, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_ac4_minimal() {
        let sel = [0x00]; // no config, no toc, no extra
        let bytes = wrap(0x15, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Ac4(b) => {
                assert!(!b.ac4_config_flag);
                assert!(!b.ac4_toc_flag);
                assert_eq!(b.toc, None);
                assert!(b.additional_info.is_empty());
            }
            other => panic!("expected Ac4, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_t2_minimal() {
        // body = plp + system_id = 3 bytes ⇒ no flags block
        let sel = [0x07, 0x12, 0x34];
        let bytes = wrap(0x04, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2DeliverySystem(b) => {
                assert_eq!(b.plp_id, 0x07);
                assert_eq!(b.t2_system_id, 0x1234);
                assert_eq!(b.siso_miso, None);
                assert!(b.cell_loop.is_empty());
            }
            other => panic!("expected T2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_t2_with_flags_and_cells() {
        // prefix + flags block (siso=1, bw=2, gi=3, tm=4, off=1, tfs=0) + cell loop
        let b0 = (0x01 << 6) | (0x02 << 2);
        let b1 = (0x03 << 5) | (0x04 << 2) | 0x02; // off=1, tfs=0
        let sel = [0x07, 0x12, 0x34, b0, b1, 0xCA, 0xFE];
        let bytes = wrap(0x04, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2DeliverySystem(b) => {
                assert_eq!(b.siso_miso, Some(0x01));
                assert_eq!(b.bandwidth, Some(0x02));
                assert_eq!(b.guard_interval, Some(0x03));
                assert_eq!(b.transmission_mode, Some(0x04));
                assert_eq!(b.other_frequency_flag, Some(true));
                assert_eq!(b.tfs_flag, Some(false));
                assert_eq!(b.cell_loop, &[0xCA, 0xFE]);
            }
            other => panic!("expected T2DeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_primary_with_isi_and_timeslice() {
        // receiver_profiles=0x05; s2x_mode=2, scram_sel=0, ts_gs=1; ISI + timeslice
        let b0 = 0x05 << 3;
        let b1 = (0x02 << 6) | 0x01; // mode 2 [7:6], no scrambling, ts_gs 1 [1:0]
        let mut sel = vec![b0, b1];
        sel.extend_from_slice(&0x0102_0304u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0x00C8u16.to_be_bytes()); // orbital_position
        sel.push((1 << 7) | (0x02 << 5) | (1 << 4) | 0x03); // we=1 pol=2 mis=1 roll=3
        let sr: u32 = 0x0AB_CDEF; // symbol_rate (28-bit)
        sel.push((sr >> 24) as u8 & 0x0F);
        sel.push((sr >> 16) as u8);
        sel.push((sr >> 8) as u8);
        sel.push(sr as u8);
        sel.push(0x42); // input_stream_identifier (mis=1)
        sel.push(0x09); // timeslice_number (mode==2)
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.receiver_profiles, 0x05);
                assert_eq!(b.s2x_mode, 2);
                assert!(!b.scrambling_sequence_selector);
                assert_eq!(b.ts_gs_s2x_mode, 1);
                assert_eq!(b.frequency, 0x0102_0304);
                assert_eq!(b.orbital_position, 0x00C8);
                assert!(b.west_east_flag);
                assert_eq!(b.polarization, 2);
                assert!(b.multiple_input_stream_flag);
                assert_eq!(b.roll_off, 3);
                assert_eq!(b.symbol_rate, 0x0AB_CDEF);
                assert_eq!(b.input_stream_identifier, Some(0x42));
                assert_eq!(b.timeslice_number, Some(0x09));
                assert!(b.tail.is_empty());
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_with_scrambling_index() {
        let b0 = 0x01 << 3;
        let b1 = (0x01 << 6) | 0x20; // mode 1 [7:6], scrambling selector set [5]
        let mut sel = vec![b0, b1];
        // scrambling index 0x2ABCD (18-bit)
        sel.push(0x02);
        sel.push(0xAB);
        sel.push(0xCD);
        sel.extend_from_slice(&0u32.to_be_bytes()); // frequency
        sel.extend_from_slice(&0u16.to_be_bytes()); // orbital
        sel.push(0x00); // packed (mis=0)
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert!(b.scrambling_sequence_selector);
                assert_eq!(b.scrambling_sequence_index, Some(0x2ABCD));
                assert_eq!(b.input_stream_identifier, None);
                assert_eq!(b.timeslice_number, None);
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_s2x_mode3_tail_preserved() {
        // mode 3 (channel bonding) — the bond loop lands in `tail` (raw).
        let b0 = 0x01 << 3;
        let b1 = 0x03 << 6; // mode 3 [7:6], no scrambling, ts_gs 0
        let mut sel = vec![b0, b1];
        sel.extend_from_slice(&0u32.to_be_bytes());
        sel.extend_from_slice(&0u16.to_be_bytes());
        sel.push(0x00); // mis=0
        sel.extend_from_slice(&[0, 0, 0, 0]); // symbol_rate
        sel.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // raw channel-bond tail
        let bytes = wrap(0x17, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::S2XSatelliteDeliverySystem(b) => {
                assert_eq!(b.s2x_mode, 3);
                assert_eq!(b.timeslice_number, None);
                assert_eq!(b.tail, &[0xAA, 0xBB, 0xCC]);
            }
            other => panic!("expected S2X, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_audio_preselection_keeps_loop_raw() {
        // num_preselections=3 then raw loop
        let sel = [0x03 << 3, 0xAA, 0xBB, 0xCC];
        let bytes = wrap(0x19, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::AudioPreselection(b) => {
                assert_eq!(b.num_preselections, 3);
                assert_eq!(b.preselection_loop, &[0xAA, 0xBB, 0xCC]);
            }
            other => panic!("expected AudioPreselection, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_ttml_subtitling() {
        // ISO "eng", subtitle_purpose=0x10, tts=0x1, font=0, qualifier=0, count=1, then tail
        let b3 = (0x10 << 2) | 0x01;
        let b4 = 0x01; // font=0 qual=0 reserved=0 count=1
        let sel = [b'e', b'n', b'g', b3, b4, 0x00, 0x02, b'h', b'i'];
        let bytes = wrap(0x20, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TtmlSubtitling(b) => {
                assert_eq!(b.iso_639_language_code, b"eng");
                assert_eq!(b.subtitle_purpose, 0x10);
                assert_eq!(b.tts_suitability, 0x01);
                assert!(!b.essential_font_usage_flag);
                assert!(!b.qualifier_present_flag);
                assert_eq!(b.dvb_ttml_profile_count, 1);
                assert_eq!(b.tail, &[0x00, 0x02, b'h', b'i']);
            }
            other => panic!("expected TtmlSubtitling, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_target_region_loop_raw() {
        let sel = [b'g', b'b', b'r', 0x01, 0x02, 0x03];
        let bytes = wrap(0x09, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegion(b) => {
                assert_eq!(b.country_code, b"gbr");
                assert_eq!(b.region_loop, &[0x01, 0x02, 0x03]);
            }
            other => panic!("expected TargetRegion, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_target_region_name_loop_raw() {
        let sel = [b'g', b'b', b'r', b'e', b'n', b'g', 0x44, 0x55];
        let bytes = wrap(0x0A, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegionName(b) => {
                assert_eq!(b.country_code, b"gbr");
                assert_eq!(b.iso_639_language_code, b"eng");
                assert_eq!(b.region_loop, &[0x44, 0x55]);
            }
            other => panic!("expected TargetRegionName, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_network_change_notify_loop_raw() {
        let sel = [0x00, 0x01, 0x05, 0xAA, 0xBB];
        let bytes = wrap(0x07, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::NetworkChangeNotify(b) => {
                assert_eq!(b.cell_loop, &sel);
            }
            other => panic!("expected NetworkChangeNotify, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ExtensionDescriptor {
            tag_extension: 0x0B,
            body: ExtensionBody::ServiceRelocated(ServiceRelocated {
                old_original_network_id: 1,
                old_transport_stream_id: 2,
                old_service_id: 3,
            }),
        };
        let mut tiny = [0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut tiny).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn descriptor_length_matches_body() {
        let d = ExtensionDescriptor {
            tag_extension: 0x08,
            body: ExtensionBody::Message(Message {
                message_id: 1,
                iso_639_language_code: b"eng",
                text: b"hello",
            }),
        };
        // tag_ext(1) + message_id(1) + iso(3) + text(5) = 10
        assert_eq!(d.descriptor_length(), 10);
    }

    /// JSON round-trip for an all-owned typed body (no borrowed slices). Bodies
    /// carrying borrowed `&[u8]` cannot zero-copy-deserialize from JSON (serde
    /// renders `&[u8]` as a numeric sequence) — that is a property of the
    /// borrow, not this type, and matches the rest of the crate which only
    /// `serde(borrow)`-annotates without JSON-round-tripping byte slices. For
    /// the borrowed case we assert serialization succeeds.
    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_owned_body() {
        let typed = ExtensionDescriptor {
            tag_extension: 0x0D,
            body: ExtensionBody::C2DeliverySystem(C2DeliverySystem {
                plp_id: 1,
                data_slice_id: 2,
                c2_system_tuning_frequency: 0xDEAD_BEEF,
                c2_system_tuning_frequency_type: 1,
                active_ofdm_symbol_duration: 2,
                guard_interval: 3,
            }),
        };
        let json = serde_json::to_string(&typed).unwrap();
        let back: ExtensionDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(typed, back);
    }

    /// Borrowed bodies (Raw, Message, …) serialize cleanly; the discriminant +
    /// tag survive the JSON encoding.
    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_borrowed_body() {
        let raw = ExtensionDescriptor {
            tag_extension: 0x42,
            body: ExtensionBody::Raw(&[0x01, 0x02, 0x03]),
        };
        let json = serde_json::to_string(&raw).unwrap();
        assert!(json.contains("\"tag_extension\":66"));
        assert!(json.contains("\"Raw\""));

        let msg = ExtensionDescriptor {
            tag_extension: 0x08,
            body: ExtensionBody::Message(Message {
                message_id: 7,
                iso_639_language_code: b"eng",
                text: b"hi",
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"message_id\":7"));
    }
}
