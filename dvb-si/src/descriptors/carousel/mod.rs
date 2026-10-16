//! Data-carousel descriptor namespace — [`AnyCarouselDescriptor`] +
//! [`parse_carousel_loop`].
//!
//! The descriptors that ETSI EN 301 192 §10.2 (Table 48) allows in a DSM-CC
//! data carousel — in a DownloadInfoIndication `moduleInfo` / BIOP
//! `ModuleInfo.userInfo` and in a DownloadServerInitiate `GroupInfo` — live in
//! their OWN private tag space (Table 49: `0x00..=0x0B` allocated by DVB,
//! `0x0C..=0x6F` reserved for DVB, `0x70..=0x7F` reserved for DVB MHP,
//! `0x80..=0xFF` private). Tag `0x01` is `type_descriptor` here but
//! `video_stream` in the SI/MPEG space, so loops must not be walked with
//! [`crate::descriptors::parse_loop`].
//!
//! [`AnyCarouselDescriptor`] is generated from a single declarative list
//! (`declare_carousel_descriptors!`), one line per tag, exactly like the AIT namespace
//! in [`crate::descriptors::ait`]; the generated drift test pins each
//! tag literal to the type's [`crate::traits::DescriptorDef::TAG`].
//!
//! ```
//! use dvb_si::descriptors::carousel::{parse_carousel_loop, AnyCarouselDescriptor};
//!
//! // type_descriptor "text/css" then an MHP caching_priority_descriptor (0x71,
//! // not allocated in EN 301 192 so it stays `Unknown`).
//! let loop_bytes = [
//!     0x01, 0x08, b't', b'e', b'x', b't', b'/', b'c', b's', b's',
//!     0x71, 0x02, 0x02, 0xFF,
//! ];
//! let items: Vec<_> = parse_carousel_loop(&loop_bytes).collect();
//! assert_eq!(items.len(), 2);
//! match items[0].as_ref().unwrap() {
//!     AnyCarouselDescriptor::Type(t) => assert_eq!(t.media_type.raw(), b"text/css"),
//!     other => panic!("expected Type, got {other:?}"),
//! }
//! assert!(matches!(
//!     items[1].as_ref().unwrap(),
//!     AnyCarouselDescriptor::Unknown { tag: 0x71, .. }
//! ));
//! ```

pub mod compressed_module;
pub mod crc32;
pub mod est_download_time;
pub mod group_link;
pub mod info;
pub mod location;
pub mod module_link;
pub mod name;
pub mod ssu_module_type;
pub mod subgroup_association;
pub mod type_descriptor;

pub use compressed_module::CompressedModuleDescriptor;
pub use crc32::Crc32Descriptor;
pub use est_download_time::EstDownloadTimeDescriptor;
pub use group_link::GroupLinkDescriptor;
pub use info::InfoDescriptor;
pub use location::LocationDescriptor;
pub use module_link::{LinkPosition, ModuleLinkDescriptor};
pub use name::NameDescriptor;
pub use ssu_module_type::{SsuModuleType, SsuModuleTypeDescriptor};
pub use subgroup_association::SubgroupAssociationDescriptor;
pub use type_descriptor::TypeDescriptor;

/// Declares [`AnyCarouselDescriptor`] + its dispatcher from one tag list.
///
/// Mirrors `declare_ait_descriptors!` in `crate::descriptors::ait`.
/// Each line is `Variant = 0xTAG => crate::descriptors::carousel::module::Type[<'a>]`.
macro_rules! declare_carousel_descriptors {
    (
        $lt:lifetime;
        $( $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented data-carousel descriptor, plus an `Unknown` fallthrough.
        ///
        /// Data-carousel descriptor tags belong to their own private tag space
        /// (EN 301 192 §10.2.1 Table 48/49), separate from both the EN 300 468 SI
        /// tags and the AIT tags. The same numeric tag value may map to a
        /// different type there.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnyCarouselDescriptor<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// Tag with no typed implementation; `body` is the payload sans
            /// the 2-byte (tag, length) header.
            Unknown {
                /// The raw descriptor_tag byte.
                tag: u8,
                /// The raw payload bytes (descriptor_length bytes).
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyCarouselDescriptor<$lt> {
                fn from(d: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(d)
                }
            }
        )+

        impl<$lt> AnyCarouselDescriptor<$lt> {
            /// Every tag the generated dispatcher routes (excludes
            /// [`AnyCarouselDescriptor::Unknown`]).
            pub const DISPATCHED_TAGS: &'static [u8] = &[$($tag),+];

            /// Diagnostic name of the contained carousel descriptor — the type's
            /// [`DescriptorDef::NAME`](crate::traits::DescriptorDef::NAME)
            /// (`"TYPE"`, `"MODULE_LINK"`, …); `"UNKNOWN"`
            /// for [`AnyCarouselDescriptor::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::DescriptorDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one full carousel descriptor (2-byte header included) by its tag.
            ///
            /// `None` means no typed implementation exists for `tag` (the
            /// caller turns that into [`AnyCarouselDescriptor::Unknown`]). `Some(Err)`
            /// is a typed parse failure for a recognised tag.
            pub(crate) fn dispatch(tag: u8, full: &$lt [u8]) -> Option<crate::error::Result<Self>> {
                use broadcast_common::Parse;
                match tag {
                    $(
                        $tag => Some(<$($path)::+>::parse(full).map(Self::$variant)),
                    )+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn tag_literals_match_descriptor_def() {
                use crate::traits::DescriptorDef;
                $(
                    assert_eq!(
                        $tag,
                        <$($path)::+ as DescriptorDef>::TAG,
                        concat!("tag literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as DescriptorDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_carousel_descriptors! {'a;
    Type                = 0x01 => crate::descriptors::carousel::type_descriptor::TypeDescriptor<'a>,
    Name                = 0x02 => crate::descriptors::carousel::name::NameDescriptor<'a>,
    Info                = 0x03 => crate::descriptors::carousel::info::InfoDescriptor<'a>,
    ModuleLink          = 0x04 => crate::descriptors::carousel::module_link::ModuleLinkDescriptor,
    Crc32               = 0x05 => crate::descriptors::carousel::crc32::Crc32Descriptor,
    Location            = 0x06 => crate::descriptors::carousel::location::LocationDescriptor,
    EstDownloadTime     = 0x07 => crate::descriptors::carousel::est_download_time::EstDownloadTimeDescriptor,
    GroupLink           = 0x08 => crate::descriptors::carousel::group_link::GroupLinkDescriptor,
    CompressedModule    = 0x09 => crate::descriptors::carousel::compressed_module::CompressedModuleDescriptor,
    SsuModuleType       = 0x0A => crate::descriptors::carousel::ssu_module_type::SsuModuleTypeDescriptor,
    SubgroupAssociation = 0x0B => crate::descriptors::carousel::subgroup_association::SubgroupAssociationDescriptor,
}

/// Lazily walk a raw data-carousel descriptor loop. Never panics.
///
/// Per-descriptor parse errors yield `Err` and iteration continues (the
/// `descriptor_length` field bounds each entry, so the walker can always
/// advance past a malformed body). A truncated final header or body yields
/// one `Err` and then the iterator fuses.
#[must_use]
pub fn parse_carousel_loop(bytes: &[u8]) -> CarouselDescriptorIter<'_> {
    CarouselDescriptorIter {
        bytes,
        pos: 0,
        fused: false,
    }
}

/// Iterator over a raw data-carousel descriptor loop; see [`parse_carousel_loop`].
#[derive(Debug, Clone)]
pub struct CarouselDescriptorIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for CarouselDescriptorIter<'a> {
    type Item = crate::error::Result<AnyCarouselDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        Some(match AnyCarouselDescriptor::dispatch(tag, full) {
            Some(res) => res,
            None => Ok(AnyCarouselDescriptor::Unknown {
                tag,
                body: &full[2..],
            }),
        })
    }
}

impl core::iter::FusedIterator for CarouselDescriptorIter<'_> {}

/// A raw data-carousel descriptor loop, borrowed from the section. Zero-copy: walk it
/// typed via [`CarouselDescriptorLoop::iter`]; serde serializes the typed walk.
///
/// This mirrors [`crate::descriptors::DescriptorLoop`] for the data-carousel namespace.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CarouselDescriptorLoop<'a>(&'a [u8]);

impl core::fmt::Debug for CarouselDescriptorLoop<'_> {
    /// Cheap: prints the byte length, not the decoded entries.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CarouselDescriptorLoop(<{} bytes>)", self.0.len())
    }
}

impl<'a> IntoIterator for &CarouselDescriptorLoop<'a> {
    type Item = crate::error::Result<AnyCarouselDescriptor<'a>>;
    type IntoIter = CarouselDescriptorIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> CarouselDescriptorLoop<'a> {
    /// Wrap a raw carousel descriptor-loop slice (the `descriptor()` bytes only —
    /// no enclosing length field).
    #[must_use]
    pub const fn new(raw: &'a [u8]) -> Self {
        Self(raw)
    }

    /// The raw wire bytes of the loop, verbatim.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.0
    }

    /// Length in bytes of the raw loop.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Is the loop empty?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Walk the loop as raw `(tag, body)` pairs without typed parsing,
    /// stopping at the first truncated entry.
    pub fn raw_tags(&self) -> impl Iterator<Item = (u8, &'a [u8])> {
        let bytes = self.0;
        let mut pos = 0usize;
        core::iter::from_fn(move || {
            let rem = bytes.get(pos..)?;
            let (&tag, rem) = rem.split_first()?;
            let (&len, rem) = rem.split_first()?;
            let body = rem.get(..usize::from(len))?;
            pos += 2 + usize::from(len);
            Some((tag, body))
        })
    }

    /// Lazily walk the loop, yielding one typed [`AnyCarouselDescriptor`] per entry
    /// (or [`AnyCarouselDescriptor::Unknown`] for tags with no implementation).
    /// Delegates to [`parse_carousel_loop`]; never panics.
    #[must_use]
    pub fn iter(&self) -> CarouselDescriptorIter<'a> {
        parse_carousel_loop(self.0)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for CarouselDescriptorLoop<'_> {
    /// Serializes as a sequence of the typed walk: each `Ok(d)` becomes the
    /// [`AnyCarouselDescriptor`] (camelCase external tagging), and each `Err(e)`
    /// becomes a `{"parseError": "<Display>"}` map — parse errors are
    /// surfaced, never silently dropped (r03-W13, matching
    /// [`DescriptorLoop`](crate::descriptors::DescriptorLoop)'s serde impl).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        struct Entry<'a>(crate::error::Result<AnyCarouselDescriptor<'a>>);
        impl serde::Serialize for Entry<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                match &self.0 {
                    Ok(d) => d.serialize(s),
                    Err(e) => {
                        use serde::ser::SerializeMap;
                        let mut m = s.serialize_map(Some(1))?;
                        m.serialize_entry("parseError", &alloc::string::ToString::to_string(e))?;
                        m.end()
                    }
                }
            }
        }
        s.collect_seq(self.iter().map(Entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_typed_and_unknown() {
        let buf = [
            0x09, 0x05, 0x08, 0x00, 0x00, 0x02, 0x34, // compressed_module
            0x05, 0x04, 0xDE, 0xAD, 0xBE, 0xEF, // CRC32
            0x71, 0x02, 0x02, 0xFF, // unknown (MHP caching_priority)
        ];
        let items: Vec<_> = parse_carousel_loop(&buf).collect();
        assert_eq!(items.len(), 3);
        assert!(matches!(
            items[0].as_ref().unwrap(),
            AnyCarouselDescriptor::CompressedModule(c) if c.original_size == 0x234
        ));
        assert!(matches!(
            items[1].as_ref().unwrap(),
            AnyCarouselDescriptor::Crc32(c) if c.crc_32 == 0xDEAD_BEEF
        ));
        assert!(matches!(
            items[2].as_ref().unwrap(),
            AnyCarouselDescriptor::Unknown { tag: 0x71, body } if *body == [0x02, 0xFF]
        ));
        assert_eq!(items[0].as_ref().unwrap().name(), "COMPRESSED_MODULE");
    }

    #[test]
    fn loop_helpers_and_truncation() {
        let buf = [0x06, 0x01, 0x42, 0x07, 0x04, 0x00];
        let l = CarouselDescriptorLoop::new(&buf);
        assert_eq!(l.len(), 6);
        assert!(!l.is_empty());
        assert_eq!(l.raw(), &buf);
        // raw_tags stops at the truncated second entry.
        assert_eq!(l.raw_tags().count(), 1);
        let items: Vec<_> = l.iter().collect();
        assert_eq!(items.len(), 2);
        assert!(items[0].is_ok() && items[1].is_err());
        assert!(CarouselDescriptorLoop::default().is_empty());
    }

    #[test]
    fn typed_parse_error_surfaces_and_walk_continues() {
        // location_descriptor with a 2-byte body (must be 1), then a valid one.
        let buf = [0x06, 0x02, 0x01, 0x02, 0x06, 0x01, 0x09];
        let items: Vec<_> = parse_carousel_loop(&buf).collect();
        assert_eq!(items.len(), 2);
        assert!(items[0].is_err());
        assert!(matches!(
            items[1].as_ref().unwrap(),
            AnyCarouselDescriptor::Location(l) if l.location_tag == 9
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_surfaces_parse_errors_instead_of_dropping_them() {
        let buf = [0x06, 0xFF];
        let json = serde_json::to_string(&CarouselDescriptorLoop::new(&buf)).unwrap();
        assert!(json.contains("parseError"), "{json}");
    }

    #[test]
    fn dispatched_tags_covers_all_known() {
        assert_eq!(
            AnyCarouselDescriptor::DISPATCHED_TAGS,
            &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B
            ]
        );
    }
}
