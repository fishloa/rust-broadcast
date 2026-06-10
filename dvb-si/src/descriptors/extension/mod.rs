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
//! - `0x09` target_region (Table 156, §6.4.12) — region loop unfolded.
//! - `0x0A` target_region_name (Table 157, §6.4.13) — region loop unfolded.
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

#[cfg(test)]
mod test_support;

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
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
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

        /// Map a `descriptor_tag_extension` byte to its known [`ExtensionTag`].
        fn kind_from_tag(tag_extension: u8) -> Option<ExtensionTag> {
            Some(match tag_extension {
                $(
                    $tag => ExtensionTag::$variant,
                )+
                _ => return None,
            })
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
    T2DeliverySystem = 0x04 => T2DeliverySystem,
    /// `0x05` — SH_delivery_system (Table 119, §6.4.6.2).
    ShDeliverySystem = 0x05 => ShDeliverySystem,
    /// `0x06` — supplementary_audio (Table 153, §6.4.11).
    SupplementaryAudio = 0x06 => SupplementaryAudio<'a>,
    /// `0x07` — network_change_notify (Table 149, §6.4.9).
    NetworkChangeNotify = 0x07 => NetworkChangeNotify,
    /// `0x08` — message (Table 148, §6.4.9).
    Message = 0x08 => Message<'a>,
    /// `0x09` — target_region (Table 156, §6.4.12).
    TargetRegion = 0x09 => TargetRegion,
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

/// Sealed trait — no external implementors. Keeps `ExtensionBodyDef` extensible
/// without breaking downstream implementors.
pub(crate) mod sealed {
    pub trait Sealed {}
}

/// Per-body metadata for the extension-descriptor sub-dispatch — the
/// `descriptor_tag_extension` value and a diagnostic name. Mirrors
/// [`crate::traits::DescriptorDef`] for the second dispatch level (ADR-0001).
pub trait ExtensionBodyDef: sealed::Sealed {
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
        kind_from_tag(self.tag_extension)
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
    use super::test_support::*;
    use super::*;

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
        assert!(json.contains("\"c2DeliverySystem\""));
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
        assert!(json.contains("\"raw\""));

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
