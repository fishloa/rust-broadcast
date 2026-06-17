//! AIT descriptor namespace — [`AnyAitDescriptor`] + [`parse_ait_loop`].
//!
//! The AIT (Application Information Table, ETSI TS 102 809 §5.3) uses a
//! SEPARATE descriptor namespace from the EN 300 468 SI descriptors. Low tag
//! bytes mean different things (e.g. AIT tag 0x02 = transport_protocol, but
//! SI tag 0x02 = video_stream). This module mirrors the structure of
//! [`crate::descriptors::any`] for the AIT namespace.
//!
//! [`AnyAitDescriptor`] is generated from a single declarative list
//! (`declare_ait_descriptors!`) — one line per AIT descriptor tag. The list
//! produces the enum, `From` impls, `name()`, `dispatch()`,
//! `DISPATCHED_TAGS`, and a drift test that pins each tag literal to the
//! type's [`crate::traits::DescriptorDef::TAG`].
//!
//! [`parse_ait_loop`] lazily walks a raw AIT descriptor loop (the
//! `descriptor()` sequence inside AIT common or per-application loops),
//! yielding one [`AnyAitDescriptor`] per entry.
//!
//! ```
//! use dvb_si::descriptors::ait::{parse_ait_loop, AnyAitDescriptor};
//!
//! // An AIT descriptor loop: application_usage (tag 0x16, usage_type=0x01)
//! // then an unknown tag 0xFE.
//! let loop_bytes = [
//!     0x16, 0x01, 0x01,             // application_usage: Digital Text
//!     0xFE, 0x02, 0xCA, 0xFE,       // unknown 0xFE
//! ];
//! let items: Vec<_> = parse_ait_loop(&loop_bytes).collect();
//! assert_eq!(items.len(), 2);
//! match items[0].as_ref().unwrap() {
//!     AnyAitDescriptor::ApplicationUsage(au) => {
//!         assert_eq!(au.usage_type, 0x01);
//!     }
//!     other => panic!("expected ApplicationUsage, got {other:?}"),
//! }
//! assert!(matches!(
//!     items[1].as_ref().unwrap(),
//!     AnyAitDescriptor::Unknown { tag: 0xFE, .. }
//! ));
//! ```

pub mod application;
pub mod application_name;
pub mod application_usage;
pub mod dvb_j_application;
pub mod dvb_j_application_location;
pub mod external_application_authorisation;
pub mod simple_application_boundary;
pub mod simple_application_location;
pub mod transport_protocol;

pub use application::ApplicationDescriptor;
pub use application::Visibility;
pub use application_name::ApplicationNameDescriptor;
pub use application_usage::ApplicationUsageDescriptor;
pub use dvb_j_application::DvbJApplicationDescriptor;
pub use dvb_j_application_location::DvbJApplicationLocationDescriptor;
pub use external_application_authorisation::ExternalApplicationAuthorisationDescriptor;
pub use simple_application_boundary::SimpleApplicationBoundaryDescriptor;
pub use simple_application_location::SimpleApplicationLocationDescriptor;
pub use transport_protocol::TransportProtocolDescriptor;

/// Declares [`AnyAitDescriptor`] + its dispatcher from one tag list.
///
/// Mirrors the `declare_descriptors!` macro in `crate::descriptors::any`.
/// Each line is `Variant = 0xTAG => crate::descriptors::ait::module::Type[<'a>]`.
macro_rules! declare_ait_descriptors {
    (
        $lt:lifetime;
        $( $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented AIT descriptor, plus an `Unknown` fallthrough.
        ///
        /// AIT tags belong to a separate namespace from EN 300 468 SI tags
        /// (ETSI TS 102 809 §5.3). The same numeric tag value may map to
        /// a different type here.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnyAitDescriptor<$lt> {
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
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyAitDescriptor<$lt> {
                fn from(d: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(d)
                }
            }
        )+

        impl<$lt> AnyAitDescriptor<$lt> {
            /// Every tag the generated dispatcher routes (excludes
            /// [`AnyAitDescriptor::Unknown`]).
            pub const DISPATCHED_TAGS: &'static [u8] = &[$($tag),+];

            /// Diagnostic name of the contained AIT descriptor — the type's
            /// [`DescriptorDef::NAME`](crate::traits::DescriptorDef::NAME)
            /// (`"APPLICATION"`, `"TRANSPORT_PROTOCOL"`, …); `"UNKNOWN"`
            /// for [`AnyAitDescriptor::Unknown`].
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

            /// Parse one full AIT descriptor (2-byte header included) by its tag.
            ///
            /// `None` means no typed implementation exists for `tag` (the
            /// caller turns that into [`AnyAitDescriptor::Unknown`]). `Some(Err)`
            /// is a typed parse failure for a recognised tag.
            pub(crate) fn dispatch(tag: u8, full: &$lt [u8]) -> Option<crate::error::Result<Self>> {
                use dvb_common::Parse;
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

declare_ait_descriptors! {'a;
    Application              = 0x00 => crate::descriptors::ait::application::ApplicationDescriptor,
    ApplicationName          = 0x01 => crate::descriptors::ait::application_name::ApplicationNameDescriptor<'a>,
    TransportProtocol        = 0x02 => crate::descriptors::ait::transport_protocol::TransportProtocolDescriptor<'a>,
    DvbJApplication          = 0x03 => crate::descriptors::ait::dvb_j_application::DvbJApplicationDescriptor<'a>,
    DvbJApplicationLocation  = 0x04 => crate::descriptors::ait::dvb_j_application_location::DvbJApplicationLocationDescriptor<'a>,
    ExternalAppAuthorisation = 0x05 => crate::descriptors::ait::external_application_authorisation::ExternalApplicationAuthorisationDescriptor,
    SimpleAppLocation        = 0x15 => crate::descriptors::ait::simple_application_location::SimpleApplicationLocationDescriptor<'a>,
    ApplicationUsage         = 0x16 => crate::descriptors::ait::application_usage::ApplicationUsageDescriptor,
    SimpleAppBoundary        = 0x17 => crate::descriptors::ait::simple_application_boundary::SimpleApplicationBoundaryDescriptor<'a>,
}

/// Lazily walk a raw AIT descriptor loop. Never panics.
///
/// Per-descriptor parse errors yield `Err` and iteration continues (the
/// `descriptor_length` field bounds each entry, so the walker can always
/// advance past a malformed body). A truncated final header or body yields
/// one `Err` and then the iterator fuses.
#[must_use]
pub fn parse_ait_loop(bytes: &[u8]) -> AitDescriptorIter<'_> {
    AitDescriptorIter {
        bytes,
        pos: 0,
        fused: false,
    }
}

/// Iterator over a raw AIT descriptor loop; see [`parse_ait_loop`].
#[derive(Debug, Clone)]
pub struct AitDescriptorIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for AitDescriptorIter<'a> {
    type Item = crate::error::Result<AnyAitDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };
        Some(match AnyAitDescriptor::dispatch(tag, full) {
            Some(res) => res,
            None => Ok(AnyAitDescriptor::Unknown {
                tag,
                body: &full[2..],
            }),
        })
    }
}

impl core::iter::FusedIterator for AitDescriptorIter<'_> {}

/// A raw AIT descriptor loop, borrowed from the section. Zero-copy: walk it
/// typed via [`AitDescriptorLoop::iter`]; serde serializes the typed walk.
///
/// This mirrors [`crate::descriptors::DescriptorLoop`] for the AIT namespace.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AitDescriptorLoop<'a>(&'a [u8]);

impl<'a> AitDescriptorLoop<'a> {
    /// Wrap a raw AIT descriptor-loop slice (the `descriptor()` bytes only —
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

    /// Lazily walk the loop, yielding one typed [`AnyAitDescriptor`] per entry
    /// (or [`AnyAitDescriptor::Unknown`] for tags with no implementation).
    /// Delegates to [`parse_ait_loop`]; never panics.
    #[must_use]
    pub fn iter(&self) -> AitDescriptorIter<'a> {
        parse_ait_loop(self.0)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for AitDescriptorLoop<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use alloc::vec::Vec;
        let items: Vec<crate::error::Result<AnyAitDescriptor<'_>>> = self.iter().collect();
        s.collect_seq(items.into_iter().filter_map(|r| r.ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_ait_loop_yields_typed_variants() {
        // application_usage (0x16) + simple_app_location (0x15) + unknown (0xFE).
        let buf = alloc::vec![
            0x16, 0x01, 0x01, 0x15, 0x05, b'/', b'a', b'p', b'p', b'/', 0xFE, 0x02, 0xCA, 0xFE,
        ];
        let items: Vec<_> = parse_ait_loop(&buf).collect();
        assert_eq!(items.len(), 3);

        match items[0].as_ref().unwrap() {
            AnyAitDescriptor::ApplicationUsage(au) => {
                assert_eq!(au.usage_type, 0x01);
            }
            other => panic!("expected ApplicationUsage, got {other:?}"),
        }

        match items[1].as_ref().unwrap() {
            AnyAitDescriptor::SimpleAppLocation(loc) => {
                assert_eq!(loc.initial_path_bytes.raw(), b"/app/");
            }
            other => panic!("expected SimpleAppLocation, got {other:?}"),
        }

        match items[2].as_ref().unwrap() {
            AnyAitDescriptor::Unknown { tag, body } => {
                assert_eq!(*tag, 0xFE);
                assert_eq!(*body, &[0xCA, 0xFE]);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn ait_descriptor_loop_iter() {
        let buf = [0x16, 0x01, 0x01];
        let loop_ = AitDescriptorLoop::new(&buf);
        let items: Vec<_> = loop_.iter().collect();
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0].as_ref().unwrap(),
            AnyAitDescriptor::ApplicationUsage(_)
        ));
        assert_eq!(loop_.raw(), &buf[..]);
    }

    #[test]
    fn dispatched_tags_covers_all_known() {
        assert_eq!(
            AnyAitDescriptor::DISPATCHED_TAGS,
            &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x15, 0x16, 0x17]
        );
    }
}
