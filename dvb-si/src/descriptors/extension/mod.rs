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
//! - `0x05` SH_delivery_system (Table 119, §6.4.6.2) — fully typed modulation loop.
//! - `0x06` supplementary_audio (Table 153, §6.4.11).
//! - `0x07` network_change_notify (Table 149, §6.4.9) — cell loop raw.
//! - `0x08` message (Table 148, §6.4.9).
//! - `0x09` target_region (Table 156, §6.4.12) — region loop raw.
//! - `0x0A` target_region_name (Table 157, §6.4.13) — region loop raw.
//! - `0x0B` service_relocated (Table 152, §6.4.10).
//! - `0x0D` C2_delivery_system (Table 115, §6.4.6.1).
//! - `0x11` T2MI (Table 158, §6.4.14).
//! - `0x10` video_depth_range (Table 160, §6.4.16.1) — fully typed range loop.
//! - `0x13` URI_linkage (Table 159, §6.4.16.1) — uri/private split typed.
//! - `0x15` AC-4 (annex D syntax table, §D.5) — first level; toc/extra raw.
//! - `0x16` C2_bundle_delivery_system (Table 139, §6.4.6.4) — full fixed loop.
//! - `0x17` S2X_satellite_delivery_system (Table 140, §6.4.6.5.2) — primary
//!   channel typed; channel-bonding / reserved tail kept raw.
//! - `0x19` audio_preselection (Table 110, §6.4.1) — preselection loop raw.
//! - `0x20` TTML_subtitling (`en_303_560_ttml.md` Table 1, §5.2.1.1).
//! - `0x22` service_prominence (Table 162c, §6.4.18) — SOGI loop typed; target_region loop raw.
//! - `0x23` vvc_subpictures (Table 162a, §6.4.17) — fully typed.
//!
//! Kept [`ExtensionBody::Raw`] (tag value preserved), with reason:
//! - `0x00` image_icon — syntax vendored (Table 145) but niche (carousel icons); deferred.
//! - `0x01` cpcm_delivery_signalling — spec not vendored (ETSI TS 102 825).
//! - `0x02` CP / `0x03` CP_identifier — spec not vendored (ETSI TS 102 825).
//! - `0x0C` XAIT_PID — deferred (TS 102 727 PDF vendored, no extracted syntax table yet).
//! - `0x0E` DTS-HD / `0x0F` DTS_Neural / `0x21` DTS-UHD — spec not vendored (annex G/L).
//! - `0x14` CI_ancillary_data — spec not vendored (ETSI TS 103 205).
//! - `0x18` protection_message — spec not vendored (ETSI TS 102 809).
//! - `0x24` S2Xv2 — niche; deferred.
//! - any other value (incl. `0x80`..=`0xFF` user-defined) — unknown; preserved.

use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

mod ac4;
mod audio_preselection;
mod c2_bundle_delivery_system;
mod c2_delivery_system;
mod image_icon;
mod message;
mod network_change_notify;
mod s2x_satellite_delivery_system;
mod service_prominence;
mod service_relocated;
mod sh_delivery_system;
mod supplementary_audio;
mod t2_delivery_system;
mod t2mi;
mod target_region;
mod target_region_name;
mod ttml_subtitling;
mod uri_linkage;
mod video_depth_range;
mod vvc_subpictures;

pub use ac4::*;
pub use audio_preselection::*;
pub use c2_bundle_delivery_system::*;
pub use c2_delivery_system::*;
pub use image_icon::*;
pub use message::*;
pub use network_change_notify::*;
pub use s2x_satellite_delivery_system::*;
pub use service_prominence::*;
pub use service_relocated::*;
pub use sh_delivery_system::*;
pub use supplementary_audio::*;
pub use t2_delivery_system::*;
pub use t2mi::*;
pub use target_region::*;
pub use target_region_name::*;
pub use ttml_subtitling::*;
pub use uri_linkage::*;
pub use video_depth_range::*;
pub use vvc_subpictures::*;

/// Descriptor tag for extension_descriptor (EN 300 468 Table 54, §6.2.18.1).
pub const TAG: u8 = 0x7F;
pub(crate) const HEADER_LEN: usize = 2;
/// `descriptor_tag_extension` occupies one byte immediately after the header.
pub(crate) const TAG_EXTENSION_LEN: usize = 1;
/// Minimum body length: just the `descriptor_tag_extension` byte.
pub(crate) const MIN_BODY_LEN: usize = TAG_EXTENSION_LEN;
/// `descriptor_length` is a single byte; a serialized body may not exceed this.
pub(crate) const MAX_DESCRIPTOR_LENGTH: usize = 0xFF;

// Per-variant fixed lengths (bytes after `descriptor_tag_extension`).
pub(crate) const ISO_639_LEN: usize = 3;
pub(crate) const T2_FIXED_PREFIX_LEN: usize = 3; // plp_id(1) + T2_system_id(2)
pub(crate) const T2_FLAGS_BLOCK_LEN: usize = 2; // SISO_MISO..tfs_flag, packed in 2 bytes
pub(crate) const C2_LEN: usize = 7; // plp + data_slice + freq(4) + 1 packed byte
pub(crate) const C2_BUNDLE_ENTRY_LEN: usize = 8; // plp + data_slice + freq(4) + 1 packed + 1 (primary(1)+reserved_zero(7))
pub(crate) const SERVICE_RELOCATED_LEN: usize = 6; // 3 × u16
/// S2X primary-channel block after the 2 flags bytes (excl. scrambling/ISI/timeslice):
/// frequency(4) + orbital_position(2) + 1 packed byte + symbol_rate(4 bytes).
pub(crate) const S2X_PRIMARY_LEN: usize = 11;
pub(crate) const S2X_SCRAMBLING_LEN: usize = 3;
pub(crate) const TTML_FIXED_LEN: usize = ISO_639_LEN + 2; // ISO_639(3) + 2 packed bytes
/// Minimum T2MI selector length (Table 158 §6.4.14): 3 fixed bytes before the reserved tail.
pub(crate) const T2MI_MIN_LEN: usize = 3;
/// Range header bytes per depth-range entry (Table 160): `range_type` + `range_length`.
pub(crate) const VD_RANGE_HDR_LEN: usize = 2;
/// Production disparity hint body length (Table 162): 3 bytes — two 12-bit signed values.
pub(crate) const VD_DISPARITY_LEN: usize = 3;

/// Known `descriptor_tag_extension` values (EN 300 468 Table 109, §6.4.0).
///
/// This is a *naming* aid for callers and parser dispatch; the stored
/// discriminant is the raw [`ExtensionDescriptor::tag_extension`] `u8` so that
/// unknown / reserved / user-defined tags round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum ExtensionTag {
    /// image_icon_descriptor (kept raw — see module docs).
    ImageIcon = 0x00,
    /// T2_delivery_system_descriptor.
    T2DeliverySystem = 0x04,
    /// SH_delivery_system_descriptor (Table 119, §6.4.6.2).
    ShDeliverySystem = 0x05,
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
    /// T2-MI_descriptor (Table 158, §6.4.14).
    T2mi = 0x11,
    /// video_depth_range_descriptor (Table 160, §6.4.16.1).
    VideoDepthRange = 0x10,
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
    /// service_prominence_descriptor (Table 162c, §6.4.18).
    ServiceProminence = 0x22,
    /// vvc_subpictures_descriptor (Table 162a, §6.4.17).
    VvcSubpictures = 0x23,
}

/// Generates the extension-body dispatch from one list (ADR-0001): the
/// [`ExtensionBody`] enum (+ a `Raw` fall-through), the `parse_body`
/// dispatcher, the `selector_len`/`write_selector` serialize delegation, and
/// a drift test pinning each `descriptor_tag_extension` literal to the body
/// type's [`ExtensionBodyDef::TAG_EXTENSION`] and its [`ExtensionTag`] variant.
/// One line per typed body — the single source of truth for the sub-dispatch.
macro_rules! declare_extension_bodies {
    (
        $lt:lifetime;
        $( $(#[doc = $doc:literal])* $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Typed body of an extension descriptor, keyed on `descriptor_tag_extension`.
        ///
        /// Unrecognised or not-yet-typed discriminants land in [`ExtensionBody::Raw`],
        /// which carries the selector bytes verbatim so the descriptor round-trips.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        pub enum ExtensionBody<$lt> {
            $(
                $(#[doc = $doc])*
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// Any not-yet-typed / unknown / user-defined discriminant: selector bytes verbatim.
            Raw(&$lt [u8]),
        }

        /// Parse the selector bytes (everything after `descriptor_tag_extension`)
        /// into a typed body, falling through to [`ExtensionBody::Raw`].
        fn parse_body(tag_extension: u8, sel: &[u8]) -> Result<ExtensionBody<'_>> {
            Ok(match tag_extension {
                $(
                    $tag => ExtensionBody::$variant(<$($path)::+>::parse(sel)?),
                )+
                _ => ExtensionBody::Raw(sel),
            })
        }

        impl ExtensionBody<'_> {
            /// Selector-byte length (everything after `descriptor_tag_extension`).
            fn selector_len(&self) -> usize {
                match self {
                    $(
                        ExtensionBody::$variant(b) => b.serialized_len(),
                    )+
                    ExtensionBody::Raw(s) => s.len(),
                }
            }

            /// Write the selector bytes into `out` (assumed `>= selector_len()`).
            fn write_selector(&self, out: &mut [u8]) {
                match self {
                    $(
                        ExtensionBody::$variant(b) => {
                            // `ExtensionDescriptor::serialize_into` sizes `out` from
                            // `selector_len()`, so `serialize_into` cannot fail.
                            b.serialize_into(out)
                                .expect("caller pre-sizes out to selector_len");
                        }
                    )+
                    ExtensionBody::Raw(s) => out[..s.len()].copy_from_slice(s),
                }
            }
        }

        #[cfg(test)]
        mod dispatch_drift {
            use super::*;

            /// The macro list is the single source of truth: each tag literal must
            /// equal the body's `ExtensionBodyDef::TAG_EXTENSION`, its `NAME` must be
            /// non-empty, and `kind()` must map the tag to the matching `ExtensionTag`.
            #[test]
            fn ext_dispatch_single_source() {
                $(
                    assert_eq!(
                        $tag,
                        <$($path)::+ as ExtensionBodyDef>::TAG_EXTENSION,
                        concat!("TAG_EXTENSION drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as ExtensionBodyDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                    assert_eq!(
                        ExtensionDescriptor {
                            tag_extension: $tag,
                            body: ExtensionBody::Raw(&[]),
                        }
                        .kind(),
                        Some(ExtensionTag::$variant),
                        concat!("kind() drift for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_extension_bodies! {'a;
    /// `0x00` — image_icon (Table 145, §6.4.7; icon_transport_mode Table 146, §6.4.8;
    /// coordinate_system Table 147, §6.4.8).
    ImageIcon = 0x00 => ImageIcon<'a>,
    /// `0x04` — T2_delivery_system (Table 133, §6.4.6.3).
    T2DeliverySystem = 0x04 => T2DeliverySystem<'a>,
    /// `0x05` — SH_delivery_system (Table 119, §6.4.6.2).
    ShDeliverySystem = 0x05 => ShDeliverySystem,
    /// `0x06` — supplementary_audio (Table 153, §6.4.11).
    SupplementaryAudio = 0x06 => SupplementaryAudio<'a>,
    /// `0x07` — network_change_notify (Table 149, §6.4.9).
    NetworkChangeNotify = 0x07 => NetworkChangeNotify<'a>,
    /// `0x08` — message (Table 148, §6.4.9).
    Message = 0x08 => Message<'a>,
    /// `0x09` — target_region (Table 156, §6.4.12).
    TargetRegion = 0x09 => TargetRegion<'a>,
    /// `0x0A` — target_region_name (Table 157, §6.4.13).
    TargetRegionName = 0x0A => TargetRegionName<'a>,
    /// `0x0B` — service_relocated (Table 152, §6.4.10).
    ServiceRelocated = 0x0B => ServiceRelocated,
    /// `0x0D` — C2_delivery_system (Table 115, §6.4.6.1).
    C2DeliverySystem = 0x0D => C2DeliverySystem,
    /// `0x11` — T2-MI (Table 158, §6.4.14).
    T2mi = 0x11 => T2miDescriptor<'a>,
    /// `0x10` — video_depth_range (Table 160, §6.4.16.1).
    VideoDepthRange = 0x10 => VideoDepthRangeDescriptor<'a>,
    /// `0x13` — URI_linkage (Table 159, §6.4.16.1).
    UriLinkage = 0x13 => UriLinkage<'a>,
    /// `0x15` — AC-4 (annex D).
    Ac4 = 0x15 => Ac4<'a>,
    /// `0x16` — C2_bundle_delivery_system (Table 139, §6.4.6.4).
    C2BundleDeliverySystem = 0x16 => C2BundleDeliverySystem,
    /// `0x17` — S2X_satellite_delivery_system (Table 140, §6.4.6.5.2).
    S2XSatelliteDeliverySystem = 0x17 => S2XSatelliteDeliverySystem<'a>,
    /// `0x19` — audio_preselection (Table 110, §6.4.1).
    AudioPreselection = 0x19 => AudioPreselection<'a>,
    /// `0x20` — TTML_subtitling (EN 303 560 Table 1, §5.2.1.1).
    TtmlSubtitling = 0x20 => TtmlSubtitling<'a>,
    /// `0x22` — service_prominence (Table 162c, §6.4.18).
    ServiceProminence = 0x22 => ServiceProminence<'a>,
    /// `0x23` — vvc_subpictures (Table 162a, §6.4.17).
    VvcSubpictures = 0x23 => VvcSubpicturesDescriptor<'a>,
}

/// Per-body metadata for the extension-descriptor sub-dispatch — the
/// `descriptor_tag_extension` value and a diagnostic name. Mirrors
/// [`crate::traits::DescriptorDef`] for the second dispatch level (ADR-0001).
pub trait ExtensionBodyDef {
    /// The `descriptor_tag_extension` value this body is selected by.
    const TAG_EXTENSION: u8;
    /// SCREAMING_SNAKE diagnostic name, suffix-free.
    const NAME: &'static str;
}

/// Extension descriptor (EN 300 468 Table 54, §6.2.18.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ExtensionDescriptor<'a> {
    /// `descriptor_tag_extension` (raw `u8`; see [`ExtensionTag`] for names).
    pub tag_extension: u8,
    /// Typed body, or [`ExtensionBody::Raw`] for not-yet-typed discriminants.
    pub body: ExtensionBody<'a>,
}

impl ExtensionDescriptor<'_> {
    /// Typed view of [`Self::tag_extension`], or `None` if not a known tag.
    #[must_use]
    pub fn kind(&self) -> Option<ExtensionTag> {
        Some(match self.tag_extension {
            0x00 => ExtensionTag::ImageIcon,
            0x04 => ExtensionTag::T2DeliverySystem,
            0x05 => ExtensionTag::ShDeliverySystem,
            0x06 => ExtensionTag::SupplementaryAudio,
            0x07 => ExtensionTag::NetworkChangeNotify,
            0x08 => ExtensionTag::Message,
            0x09 => ExtensionTag::TargetRegion,
            0x0A => ExtensionTag::TargetRegionName,
            0x0B => ExtensionTag::ServiceRelocated,
            0x0D => ExtensionTag::C2DeliverySystem,
            0x10 => ExtensionTag::VideoDepthRange,
            0x11 => ExtensionTag::T2mi,
            0x13 => ExtensionTag::UriLinkage,
            0x15 => ExtensionTag::Ac4,
            0x16 => ExtensionTag::C2BundleDeliverySystem,
            0x17 => ExtensionTag::S2XSatelliteDeliverySystem,
            0x19 => ExtensionTag::AudioPreselection,
            0x20 => ExtensionTag::TtmlSubtitling,
            0x22 => ExtensionTag::ServiceProminence,
            0x23 => ExtensionTag::VvcSubpictures,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Body parsers (each consumes the selector bytes after descriptor_tag_extension)
// ---------------------------------------------------------------------------

pub(crate) fn invalid(reason: &'static str) -> Error {
    Error::InvalidDescriptor { tag: TAG, reason }
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

impl<'a> crate::traits::DescriptorDef<'a> for ExtensionDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "EXTENSION";
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
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.text.raw(), b"Hi");
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
                assert_eq!(b.iso_639_language_code, Some(LangCode(*b"fre")));
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
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
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

    // -- service_prominence_descriptor (tag extension 0x22) ----------------

    #[test]
    fn parse_service_prominence_one_entry_service_only() {
        // One SOGI entry: service_flag=1, target_region_flag=0,
        // sogi_priority=0x123, service_id=0x4567, private_data [0xAB]
        let sel = [0x04, 0x21, 0x23, 0x45, 0x67, 0xAB];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ServiceProminence));
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 1);
                let e = &b.sogi_list[0];
                assert!(!e.sogi_flag);
                assert!(!e.target_region_flag);
                assert!(e.service_flag);
                assert_eq!(e.sogi_priority, 0x0123);
                assert_eq!(e.service_id, Some(0x4567));
                assert!(e.target_region_loop.is_none());
                assert_eq!(b.private_data, &[0xAB]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_one_entry_target_region() {
        // One SOGI entry: service_flag=0, target_region_flag=1,
        // sogi_priority=0x001, target_region_loop = [0xAA, 0xBB].
        let sel = [0x05, 0x40, 0x01, 0x02, 0xAA, 0xBB];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 1);
                let e = &b.sogi_list[0];
                assert!(!e.sogi_flag);
                assert!(e.target_region_flag);
                assert!(!e.service_flag);
                assert_eq!(e.sogi_priority, 0x0001);
                assert!(e.service_id.is_none());
                assert_eq!(e.target_region_loop, Some([0xAAu8, 0xBB].as_slice()));
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_two_entries_plus_private() {
        // Two SOGI entries + private_data tail.
        // Entry 0: service_flag=1, sogi_priority=0xABC, service_id=0x1111.
        // Entry 1: target_region_flag=1, sogi_priority=0x345, region=[0xCC].
        let sel = [0x08, 0x2A, 0xBC, 0x11, 0x11, 0x43, 0x45, 0x01, 0xCC, 0xDD];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 2);
                let e0 = &b.sogi_list[0];
                assert!(e0.service_flag);
                assert_eq!(e0.sogi_priority, 0x0ABC);
                assert_eq!(e0.service_id, Some(0x1111));
                assert!(e0.target_region_loop.is_none());
                let e1 = &b.sogi_list[1];
                assert!(!e1.sogi_flag);
                assert!(e1.target_region_flag);
                assert!(!e1.service_flag);
                assert_eq!(e1.sogi_priority, 0x0345);
                assert_eq!(e1.target_region_loop, Some([0xCCu8].as_slice()));
                assert_eq!(b.private_data, &[0xDD]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_empty_list_private_only() {
        // SOGI_list_length=0, private=[0x01, 0x02]
        let sel = [0x00, 0x01, 0x02];
        let bytes = wrap(0x22, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ServiceProminence(b) => {
                assert!(b.sogi_list.is_empty());
                assert_eq!(b.private_data, &[0x01, 0x02]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_service_prominence_rejects_overrun() {
        // SOGI_list_length=5 but only 3 bytes follow
        let sel = [0x05, 0xAA, 0xBB, 0xCC];
        let bytes = wrap(0x22, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_service_prominence_rejects_entry_overrun() {
        // SOGI_list_length=3, service_flag=1 but no service_id bytes follow
        let sel = [0x03, 0x20, 0x00, 0x00];
        let bytes = wrap(0x22, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_service_prominence() {
        let d = ExtensionDescriptor {
            tag_extension: 0x22,
            body: ExtensionBody::ServiceProminence(ServiceProminence {
                sogi_list: vec![SogiEntry {
                    sogi_flag: false,
                    target_region_flag: false,
                    service_flag: true,
                    sogi_priority: 0x123,
                    service_id: Some(0x4567),
                    target_region_loop: None,
                }],
                private_data: &[0xAB],
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":34"));
        assert!(json.contains("\"ServiceProminence\""));
    }

    #[test]
    fn parse_target_region_loop_raw() {
        let sel = [b'g', b'b', b'r', 0x01, 0x02, 0x03];
        let bytes = wrap(0x09, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TargetRegion(b) => {
                assert_eq!(b.country_code, LangCode(*b"gbr"));
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
                assert_eq!(b.country_code, LangCode(*b"gbr"));
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
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
    fn parse_t2mi_round_trip() {
        // t2mi_stream_id=5, num_t2mi_streams_minus_one=2,
        // pcr_iscr_common_clock_flag=true, 2-byte reserved tail.
        let sel = [0x05, 0x02, 0x01, 0x00, 0x00];
        let bytes = wrap(0x11, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::T2mi));
        match &d.body {
            ExtensionBody::T2mi(b) => {
                assert_eq!(b.t2mi_stream_id, 5);
                assert_eq!(b.num_t2mi_streams_minus_one, 2);
                assert!(b.pcr_iscr_common_clock_flag);
                assert_eq!(b.reserved_tail, &[0x00, 0x00]);
            }
            other => panic!("expected T2mi, got {other:?}"),
        }
        // parse → serialize → parse round-trip (byte-identical)
        round_trip(&d);
        // Also verify that serialize → parse round-trips the flag false variant
        let sel2 = [0x07, 0x00, 0x00, 0xFF];
        let bytes2 = wrap(0x11, &sel2);
        let d2 = ExtensionDescriptor::parse(&bytes2).unwrap();
        round_trip(&d2);
    }

    #[test]
    fn parse_t2mi_minimal() {
        // Only the 3 mandatory bytes, no reserved tail.
        let sel = [0x01, 0x03, 0x01];
        let bytes = wrap(0x11, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2mi(b) => {
                assert_eq!(b.t2mi_stream_id, 1);
                assert_eq!(b.num_t2mi_streams_minus_one, 3);
                assert!(b.pcr_iscr_common_clock_flag);
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected T2mi, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_t2mi_rejects_truncated() {
        let sel = [0xAA, 0xBB]; // only 2 bytes, need >= 3
        let bytes = wrap(0x11, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
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
                iso_639_language_code: LangCode(*b"eng"),
                text: DvbText::new(b"hello"),
            }),
        };
        // tag_ext(1) + message_id(1) + iso(3) + text(5) = 10
        assert_eq!(d.descriptor_length(), 10);
    }

    #[test]
    fn parse_video_depth_range_two_entries_round_trip() {
        // Two depth ranges in one selector:
        //   entry 1: range_type 0x00, range_length 3, disparity max=100 min=-50
        //   entry 2: range_type 0x05, range_length 2, raw [0xAA, 0xBB]
        // max=100 -> 0x0074 bits -> sext12=100; min=-50 -> 0x0032 bits,
        // two's complement of 50 is 0x0FCE, sext12 -> -50.
        let max_val: i16 = 100;
        let min_val: i16 = -50;
        let max_b = max_val as u16 & 0x0FFF; // 0x0064
        let min_b = min_val as u16 & 0x0FFF; // 0x0FCE
        let sel = [
            0x00,
            0x03,
            (max_b >> 4) as u8,
            (((max_b & 0x0F) << 4) | ((min_b >> 8) & 0x0F)) as u8,
            min_b as u8,
            0x05,
            0x02,
            0xAA,
            0xBB,
        ];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::VideoDepthRange));
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 2);
                assert_eq!(b.ranges[0].range_type, 0x00);
                match &b.ranges[0].body {
                    DepthRangeBody::ProductionDisparityHint { max, min } => {
                        assert_eq!(*max, 100);
                        assert_eq!(*min, -50);
                    }
                    _ => panic!("expected ProductionDisparityHint"),
                }
                assert_eq!(b.ranges[1].range_type, 0x05);
                match &b.ranges[1].body {
                    DepthRangeBody::Other(s) => assert_eq!(s, &[0xAA, 0xBB]),
                    _ => panic!("expected Other"),
                }
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        // parse → serialize → parse round-trip (byte-identical)
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_negative_edge_round_trip() {
        // ProductionDisparityHint with max=-1 (0x0FFF), min=0
        let max_val: i16 = -1;
        let min_val: i16 = 0;
        let max_b = max_val as u16 & 0x0FFF;
        let min_b = min_val as u16 & 0x0FFF;
        let sel = [
            0x00,
            0x03,
            (max_b >> 4) as u8,
            (((max_b & 0x0F) << 4) | ((min_b >> 8) & 0x0F)) as u8,
            min_b as u8,
        ];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 1);
                match &b.ranges[0].body {
                    DepthRangeBody::ProductionDisparityHint { max, min } => {
                        assert_eq!(*max, -1);
                        assert_eq!(*min, 0);
                    }
                    _ => panic!("expected ProductionDisparityHint"),
                }
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_multi_region_sei_round_trip() {
        // range_type 0x01 with empty body
        let sel = [0x01, 0x00, 0x01, 0x00];
        let bytes = wrap(0x10, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert_eq!(b.ranges.len(), 2);
                assert!(matches!(b.ranges[0].body, DepthRangeBody::MultiRegionSei));
                assert!(matches!(b.ranges[1].body, DepthRangeBody::MultiRegionSei));
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_empty_selector() {
        let bytes = wrap(0x10, &[]);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VideoDepthRange(b) => {
                assert!(b.ranges.is_empty());
            }
            other => panic!("expected VideoDepthRange, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_video_depth_range_rejects_truncated() {
        // Only range_type byte, no range_length
        let sel = [0x00];
        let bytes = wrap(0x10, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_video_depth_range_rejects_overrun() {
        // range_length=5 but only 2 bytes follow
        let sel = [0x00, 0x05, 0xAA, 0xBB];
        let bytes = wrap(0x10, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    /// Serialization is deterministic for an all-owned typed body (no borrowed
    /// slices). `ExtensionDescriptor` is serialize-only because
    /// `ExtensionBody::Message` contains `DvbText` which is serialize-only; we
    /// therefore assert serialize stability rather than a round-trip.
    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_is_stable_owned_body() {
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
        assert_eq!(json, serde_json::to_string(&typed.clone()).unwrap());
        assert!(json.contains("\"tag_extension\":13"));
        assert!(json.contains("\"C2DeliverySystem\""));
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
                iso_639_language_code: LangCode(*b"eng"),
                text: DvbText::new(b"hi"),
            }),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"message_id\":7"));
    }

    #[test]
    fn parse_vvc_subpictures_with_description_round_trip() {
        // Table 162a: default_service_mode=true, 2 subpictures,
        // processing_mode=5, service_description = "Hi" (2 bytes).
        // byte0: ds=1(0x80) | sdp=1(0x40) | n=2(0x02) = 0xC2
        // subpicture 0: component_tag=0x10, vvc_subpicture_id=0x01
        // subpicture 1: component_tag=0x11, vvc_subpicture_id=0x02
        // processing_mode byte: 0x05
        // service_description_length: 2, then b'H', b'i'
        let sel = [0xC2, 0x10, 0x01, 0x11, 0x02, 0x05, 0x02, b'H', b'i'];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::VvcSubpictures));
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(b.default_service_mode);
                assert_eq!(b.subpictures.len(), 2);
                assert_eq!(b.subpictures[0].component_tag, 0x10);
                assert_eq!(b.subpictures[0].vvc_subpicture_id, 0x01);
                assert_eq!(b.subpictures[1].component_tag, 0x11);
                assert_eq!(b.subpictures[1].vvc_subpicture_id, 0x02);
                assert_eq!(b.processing_mode, 5);
                assert!(b.service_description.is_some());
                let desc = b.service_description.as_ref().unwrap();
                assert_eq!(desc.raw(), b"Hi");
                assert_eq!(desc.decode(), "Hi");
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_vvc_subpictures_no_description_round_trip() {
        // default_service_mode=false, 1 subpicture, processing_mode=1,
        // no service_description.
        // byte0: ds=0 | sdp=0 | n=1 = 0x01
        let sel = [0x01, 0x20, 0x03, 0x01];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(!b.default_service_mode);
                assert_eq!(b.subpictures.len(), 1);
                assert_eq!(b.subpictures[0].component_tag, 0x20);
                assert_eq!(b.subpictures[0].vvc_subpicture_id, 0x03);
                assert_eq!(b.processing_mode, 1);
                assert!(b.service_description.is_none());
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_vvc_subpictures_rejects_truncated() {
        // Only 2 selector bytes, need at least 1 + 0*2 + 1 = 2 bytes.
        let sel = [0x00]; // n=0, but no processing_mode byte
        let bytes = wrap(0x23, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_vvc_subpictures_rejects_overrun() {
        // service_description_present but no length byte
        let sel = [0xC0]; // ds=1, sdp=1, n=0 — missing processing_mode + desc
        let bytes = wrap(0x23, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_vvc_subpictures_serialize_round_trip_no_subpictures() {
        // Zero subpictures, no description — minimal valid payload.
        // byte0: ds=1(0x80) | sdp=0 | n=0 = 0x80
        let sel = [0x80, 0x03];
        let bytes = wrap(0x23, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::VvcSubpictures(b) => {
                assert!(b.default_service_mode);
                assert!(b.subpictures.is_empty());
                assert_eq!(b.processing_mode, 3);
                assert!(b.service_description.is_none());
            }
            other => panic!("expected VvcSubpictures, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Serialization is deterministic for an all-owned typed body.
    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_vvc_subpictures() {
        let d = ExtensionDescriptor {
            tag_extension: 0x23,
            body: ExtensionBody::VvcSubpictures(VvcSubpicturesDescriptor {
                default_service_mode: true,
                subpictures: vec![VvcSubpicture {
                    component_tag: 0x10,
                    vvc_subpicture_id: 0x01,
                }],
                processing_mode: 5,
                service_description: Some(DvbText::new(b"Hi")),
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":35"));
        assert!(json.contains("\"VvcSubpictures\""));
        assert!(json.contains("\"service_description\":\"Hi\""));
    }

    // -- image_icon_descriptor (tag extension 0x00) --------------------------------

    #[test]
    fn parse_image_icon_first_position_mode0() {
        // descriptor_number=0, last=3, icon_id=5
        // position_flag=1, coordinate_system=1, h_origin=100, v_origin=200
        // icon_type=[0xDE, 0xAD], mode=0 icon_data=[0x01, 0x02, 0x03]
        // byte0: (0<<4)|3 = 0x03
        // byte1: icon_id=5, reserved=0 = 0x05
        // packed: (0<<6)|(1<<5)|(1<<2) = 0x24
        // origin: h=100=0x0064, v=200=0x00C8 → b0=0x06, b1=0x40, b2=0xC8
        let sel = [
            0x03, 0x05, 0x24, 0x06, 0x40, 0xC8, 0x02, 0xDE, 0xAD, 0x03, 0x01, 0x02, 0x03,
        ];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ImageIcon));
        match &d.body {
            ExtensionBody::ImageIcon(b) => {
                assert_eq!(b.descriptor_number, 0);
                assert_eq!(b.last_descriptor_number, 3);
                assert_eq!(b.icon_id, 5);
                match &b.body {
                    ImageIconBody::First(f) => {
                        assert_eq!(f.icon_transport_mode, 0);
                        assert!(f.position.is_some());
                        let pos = f.position.as_ref().unwrap();
                        assert_eq!(pos.coordinate_system, 1);
                        assert_eq!(pos.icon_horizontal_origin, 100);
                        assert_eq!(pos.icon_vertical_origin, 200);
                        assert_eq!(f.icon_type, &[0xDE, 0xAD]);
                        assert_eq!(f.payload, &[0x01, 0x02, 0x03]);
                    }
                    other => panic!("expected First, got {other:?}"),
                }
            }
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_first_no_position_mode1() {
        // descriptor_number=0, last=1, icon_id=7
        // position_flag=0, mode=1 (URL), icon_type=[0xAB], url=b"http"
        // byte0: (0<<4)|1 = 0x01; byte1: 0x07
        // packed: (1<<6)|(0<<5) = 0x40
        let sel = [0x01, 0x07, 0x40, 0x01, 0xAB, 0x04, b'h', b't', b't', b'p'];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ImageIcon(b) => match &b.body {
                ImageIconBody::First(f) => {
                    assert_eq!(f.icon_transport_mode, 1);
                    assert!(f.position.is_none());
                    assert_eq!(f.icon_type, &[0xAB]);
                    assert_eq!(f.payload, b"http");
                }
                other => panic!("expected First, got {other:?}"),
            },
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_first_mode2_empty_payload() {
        // descriptor_number=0, last=0, icon_id=0, mode=2 (reserved),
        // position_flag=0, icon_type=0 bytes, empty payload
        // byte0: 0x00; byte1: 0x00; packed: (2<<6) = 0x80
        let sel = [0x00, 0x00, 0x80, 0x00];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ImageIcon(b) => match &b.body {
                ImageIconBody::First(f) => {
                    assert_eq!(f.icon_transport_mode, 2);
                    assert!(f.position.is_none());
                    assert!(f.icon_type.is_empty());
                    assert!(f.payload.is_empty());
                }
                other => panic!("expected First, got {other:?}"),
            },
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_continuation() {
        // descriptor_number=2, last=3, icon_id=1
        // icon_data=[0xAA, 0xBB, 0xCC, 0xDD]
        // byte0: (2<<4)|3 = 0x23; byte1: 0x01; length=4
        let sel = [0x23, 0x01, 0x04, 0xAA, 0xBB, 0xCC, 0xDD];
        let bytes = wrap(0x00, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ImageIcon));
        match &d.body {
            ExtensionBody::ImageIcon(b) => {
                assert_eq!(b.descriptor_number, 2);
                assert_eq!(b.last_descriptor_number, 3);
                assert_eq!(b.icon_id, 1);
                match &b.body {
                    ImageIconBody::Continuation { icon_data } => {
                        assert_eq!(icon_data, &[0xAA, 0xBB, 0xCC, 0xDD]);
                    }
                    other => panic!("expected Continuation, got {other:?}"),
                }
            }
            other => panic!("expected ImageIcon, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_image_icon_rejects_trailing_bytes() {
        // First segment with an extra trailing byte.
        // mode=2, one extra byte 0xFF after the complete parse.
        let sel = [0x00, 0x00, 0x80, 0x00, 0xFF];
        let bytes = wrap(0x00, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_image_icon_rejects_truncated_continuation() {
        // Continuation with length=5 but only 3 data bytes.
        let sel = [0x23, 0x01, 0x05, 0xAA, 0xBB, 0xCC];
        let bytes = wrap(0x00, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_image_icon() {
        let d = ExtensionDescriptor {
            tag_extension: 0x00,
            body: ExtensionBody::ImageIcon(ImageIcon {
                descriptor_number: 2,
                last_descriptor_number: 3,
                icon_id: 1,
                body: ImageIconBody::Continuation {
                    icon_data: &[0xAA, 0xBB],
                },
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":0"));
        assert!(json.contains("\"ImageIcon\""));
    }

    // -- SH_delivery_system_descriptor (tag extension 0x05) -------------------

    #[test]
    fn parse_sh_tdm_no_interleaver() {
        // diversity_mode=0x0D (1101), one TDM entry, no interleaver.
        // TDM: polarization=2, roll_off=1, modulation_mode=3,
        //      code_rate=10 (1010), symbol_rate=21 (10101).
        // flags: mod_type=0, inter_pres=0, inter_type=0 -> 0x00
        // mb0 = (2<<6)|(1<<4)|(3<<2)|((10>>2)&3) = 0x80|0x10|0x0C|0x02 = 0x9E
        // mb1 = ((10&3)<<6)|(21<<1) = (2<<6)|42 = 0x80|0x2A = 0xAA
        let sel = [0xD0, 0x00, 0x9E, 0xAA];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::ShDeliverySystem));
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x0D);
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                assert!(m.interleaver.is_none());
                match &m.modulation {
                    ShModulationMode::Tdm {
                        polarization,
                        roll_off,
                        modulation_mode,
                        code_rate,
                        symbol_rate,
                    } => {
                        assert_eq!(*polarization, 2);
                        assert_eq!(*roll_off, 1);
                        assert_eq!(*modulation_mode, 3);
                        assert_eq!(*code_rate, 10);
                        assert_eq!(*symbol_rate, 21);
                    }
                    other => panic!("expected Tdm, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_ofdm_interleaver_type1() {
        // diversity_mode=0x05, one OFDM entry, interleaver Type1.
        // OFDM: bw=1, pri=true, cah=2, cr=11(0x0B), gi=3, tm=2, cf=true
        // Interleaver Type1: cm=21(0x15)
        // flags: mod_type=1, inter_pres=1, inter_type=1 -> 0xE0
        // mb0 = (1<<5)|(1<<4)|(2<<1)|((11>>3)&1) = 0x20|0x10|0x04|0x01 = 0x35
        // mb1 = ((11&7)<<5)|(3<<3)|(2<<1)|1 = 0x60|0x18|0x04|0x01 = 0x7D
        // Type1 byte: (21<<2) = 0x54
        let sel = [0x50, 0xE0, 0x35, 0x7D, 0x54];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x05);
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                match &m.modulation {
                    ShModulationMode::Ofdm {
                        bandwidth,
                        priority,
                        constellation_and_hierarchy,
                        code_rate,
                        guard_interval,
                        transmission_mode,
                        common_frequency,
                    } => {
                        assert_eq!(*bandwidth, 1);
                        assert!(*priority);
                        assert_eq!(*constellation_and_hierarchy, 2);
                        assert_eq!(*code_rate, 11);
                        assert_eq!(*guard_interval, 3);
                        assert_eq!(*transmission_mode, 2);
                        assert!(*common_frequency);
                    }
                    other => panic!("expected Ofdm, got {other:?}"),
                }
                match &m.interleaver {
                    Some(ShInterleaver::Type1 { common_multiplier }) => {
                        assert_eq!(*common_multiplier, 21);
                    }
                    other => panic!("expected Type1 interleaver, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_tdm_interleaver_type0() {
        // diversity_mode=0x08, one TDM entry, interleaver Type0.
        // TDM: pol=0, ro=3, mm=1, cr=5, sr=10
        // Type0: cm=10, lt=20, ns=30, sd=100, nli=40
        // flags: mod_type=0, inter_pres=1, inter_type=0 -> 0x40
        // mb0 = (0<<6)|(3<<4)|(1<<2)|((5>>2)&3) = 0x30|0x04|0x01 = 0x35
        // mb1 = ((5&3)<<6)|(10<<1) = (1<<6)|20 = 0x40|0x14 = 0x54
        // Type0 byte0: (10<<2)|(20>>4) = 40|1 = 0x29
        // Type0 byte1: ((20&15)<<4)|(30>>2) = (4<<4)|7 = 0x47
        // Type0 byte2: ((30&3)<<6)|(100>>2) = (2<<6)|25 = 0x99
        // Type0 byte3: ((100&3)<<6)|40 = 0x28
        let sel = [0x80, 0x40, 0x35, 0x54, 0x29, 0x47, 0x99, 0x28];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x08);
                assert_eq!(b.modulations.len(), 1);
                let m = &b.modulations[0];
                match &m.modulation {
                    ShModulationMode::Tdm {
                        polarization,
                        roll_off,
                        modulation_mode,
                        code_rate,
                        symbol_rate,
                    } => {
                        assert_eq!(*polarization, 0);
                        assert_eq!(*roll_off, 3);
                        assert_eq!(*modulation_mode, 1);
                        assert_eq!(*code_rate, 5);
                        assert_eq!(*symbol_rate, 10);
                    }
                    other => panic!("expected Tdm, got {other:?}"),
                }
                match &m.interleaver {
                    Some(ShInterleaver::Type0 {
                        common_multiplier,
                        nof_late_taps,
                        nof_slices,
                        slice_distance,
                        non_late_increments,
                    }) => {
                        assert_eq!(*common_multiplier, 10);
                        assert_eq!(*nof_late_taps, 20);
                        assert_eq!(*nof_slices, 30);
                        assert_eq!(*slice_distance, 100);
                        assert_eq!(*non_late_increments, 40);
                    }
                    other => panic!("expected Type0 interleaver, got {other:?}"),
                }
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_two_entries_mixed() {
        // diversity_mode=0x0D
        // Entry 1: TDM (same as test 1), no interleaver.
        // Entry 2: OFDM bw=4 pri=false cah=5 cr=9 gi=1 tm=1 cf=false,
        //          Type0 interleaver cm=15 lt=25 ns=35 sd=50 nli=55
        // Entry1: flags=0x00, mb0=0x9E, mb1=0xAA
        // Entry2 flags: 0xC0 (mod=1, pres=1, type=0)
        // OFDM mb0: (4<<5)|(0<<4)|(5<<1)|((9>>3)&1) = 0x80|0x0A|0x01 = 0x8B
        // OFDM mb1: ((9&7)<<5)|(1<<3)|(1<<1)|0 = 0x20|0x08|0x02 = 0x2A
        // Type0 byte0: (15<<2)|(25>>4) = 60|1 = 0x3D
        // Type0 byte1: ((25&15)<<4)|(35>>2) = (9<<4)|8 = 0x98
        // Type0 byte2: ((35&3)<<6)|(50>>2) = (3<<6)|12 = 0xCC
        // Type0 byte3: ((50&3)<<6)|55 = (2<<6)|55 = 0xB7
        let sel = [
            0xD0, 0x00, 0x9E, 0xAA, 0xC0, 0x8B, 0x2A, 0x3D, 0x98, 0xCC, 0xB7,
        ];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x0D);
                assert_eq!(b.modulations.len(), 2);
                // Entry 1
                let m0 = &b.modulations[0];
                assert!(matches!(m0.modulation, ShModulationMode::Tdm { .. }));
                assert!(m0.interleaver.is_none());
                // Entry 2
                let m1 = &b.modulations[1];
                assert!(matches!(m1.modulation, ShModulationMode::Ofdm { .. }));
                match &m1.modulation {
                    ShModulationMode::Ofdm {
                        bandwidth,
                        priority,
                        constellation_and_hierarchy,
                        code_rate,
                        ..
                    } => {
                        assert_eq!(*bandwidth, 4);
                        assert!(!priority);
                        assert_eq!(*constellation_and_hierarchy, 5);
                        assert_eq!(*code_rate, 9);
                    }
                    _ => unreachable!(),
                }
                assert!(matches!(m1.interleaver, Some(ShInterleaver::Type0 { .. })));
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_rejects_partial_entry() {
        // Complete entry followed by a lone flags byte with no modulation block
        let sel = [0xD0, 0x00, 0x9E, 0xAA, 0x00];
        let bytes = wrap(0x05, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_sh_single_diversity_byte() {
        // Only diversity_mode byte, no modulations.
        let sel = [0xD0];
        let bytes = wrap(0x05, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ShDeliverySystem(b) => {
                assert_eq!(b.diversity_mode, 0x0D);
                assert!(b.modulations.is_empty());
            }
            other => panic!("expected ShDeliverySystem, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_sh_rejects_empty_selector() {
        let bytes = wrap(0x05, &[]);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_sh_delivery_system() {
        let d = ExtensionDescriptor {
            tag_extension: 0x05,
            body: ExtensionBody::ShDeliverySystem(ShDeliverySystem {
                diversity_mode: 0x0D,
                modulations: vec![ShModulation {
                    modulation: ShModulationMode::Ofdm {
                        bandwidth: 1,
                        priority: true,
                        constellation_and_hierarchy: 2,
                        code_rate: 11,
                        guard_interval: 3,
                        transmission_mode: 2,
                        common_frequency: true,
                    },
                    interleaver: Some(ShInterleaver::Type1 {
                        common_multiplier: 21,
                    }),
                }],
            }),
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"tag_extension\":5"));
        assert!(json.contains("\"ShDeliverySystem\""));
    }

    fn from_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Cross-implementation conformance: exact extension-descriptor bytes
    /// compiled by TSDuck (github.com/tsduck/tsduck-test, reference tests 015
    /// and 115). Parsing then re-serializing must reproduce each descriptor
    /// verbatim — this validates our wire layout (including
    /// `reserved_future_use` bits = 1, per the DVB convention) against an
    /// independent encoder, which a self-round-trip cannot.
    #[test]
    fn tsduck_reference_round_trip_byte_exact() {
        let vectors = [
            // service_prominence (test-115)
            (
                "7f20221a300c04d2f000092911fc465241fd47425211fa0102fb0b0c000ddeadbeef",
                ExtensionTag::ServiceProminence,
            ),
            ("7f0a22000011223344556677", ExtensionTag::ServiceProminence),
            (
                "7f0d220b700e092906fe4742520102",
                ExtensionTag::ServiceProminence,
            ),
            // image_icon (test-015)
            (
                "7f1a000cfc2b3e71c809696d6167652f706e67080123456789abcdef",
                ExtensionTag::ImageIcon,
            ),
            (
                "7f220007fe5f0a696d6167652f6a70656712687474703a2f2f666f6f2f6261722e6a7067",
                ExtensionTag::ImageIcon,
            ),
            ("7f090033fe050123456789", ExtensionTag::ImageIcon),
            // SH_delivery_system (test-015)
            ("7f02055f", ExtensionTag::ShDeliverySystem),
            (
                "7f0d05afff94ac175f68831d8d99ad",
                ExtensionTag::ShDeliverySystem,
            ),
        ];
        for (hex, ext) in vectors {
            let bytes = from_hex(hex);
            let d =
                ExtensionDescriptor::parse(&bytes).unwrap_or_else(|e| panic!("parse {hex}: {e:?}"));
            assert_eq!(d.kind(), Some(ext), "kind for {hex}");
            let mut out = vec![0u8; d.serialized_len()];
            let n = d.serialize_into(&mut out).unwrap();
            assert_eq!(out[..n], bytes[..], "byte-exact re-serialize for {hex}");
        }

        // Decoded-field spot checks (prove we interpret TSDuck's known values,
        // not merely round-trip our own): the rich image_icon and the rich
        // service_prominence from the XML sources of tests 015 / 115.
        let icon = from_hex("7f1a000cfc2b3e71c809696d6167652f706e67080123456789abcdef");
        match &ExtensionDescriptor::parse(&icon).unwrap().body {
            ExtensionBody::ImageIcon(b) => {
                assert_eq!(b.descriptor_number, 0);
                assert_eq!(b.last_descriptor_number, 12);
                assert_eq!(b.icon_id, 4);
                match &b.body {
                    ImageIconBody::First(f) => {
                        assert_eq!(f.icon_transport_mode, 0);
                        let p = f.position.as_ref().unwrap();
                        assert_eq!(p.coordinate_system, 2);
                        assert_eq!(p.icon_horizontal_origin, 999);
                        assert_eq!(p.icon_vertical_origin, 456);
                        assert_eq!(f.icon_type, b"image/png");
                        assert_eq!(f.payload, &[0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]);
                    }
                    other => panic!("expected First, got {other:?}"),
                }
            }
            other => panic!("expected ImageIcon, got {other:?}"),
        }

        let sp = from_hex("7f20221a300c04d2f000092911fc465241fd47425211fa0102fb0b0c000ddeadbeef");
        match &ExtensionDescriptor::parse(&sp).unwrap().body {
            ExtensionBody::ServiceProminence(b) => {
                assert_eq!(b.sogi_list.len(), 2);
                assert_eq!(b.sogi_list[0].sogi_priority, 12);
                assert_eq!(b.sogi_list[0].service_id, Some(1234));
                assert!(b.sogi_list[1].sogi_flag);
                assert_eq!(b.sogi_list[1].service_id, Some(2345));
                assert!(b.sogi_list[1].target_region_loop.is_some());
                assert_eq!(b.private_data, &[0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected ServiceProminence, got {other:?}"),
        }
    }
}
