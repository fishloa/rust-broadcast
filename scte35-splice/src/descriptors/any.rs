//! Unified splice-descriptor dispatch: [`AnySpliceDescriptor`] + [`parse_loop`].
//!
//! [`AnySpliceDescriptor`] is generated from a single declarative list
//! (`declare_splice_descriptors!`) — one line per crate-implemented
//! `splice_descriptor_tag` (§10.1, Table 16). The list is the single source of
//! truth: it produces the enum, the `From<T>` conversions, the tag → type
//! dispatcher, and a drift test that pins each tag literal to the type's
//! [`SpliceDescriptorDef::TAG`](crate::traits::SpliceDescriptorDef::TAG).
//!
//! [`parse_loop`] lazily walks the splice descriptor loop in a
//! splice_info_section, yielding one [`AnySpliceDescriptor`] per entry. Unknown
//! tags fall through to [`AnySpliceDescriptor::Unknown`] with the raw body, so a
//! loop round-trips byte-for-byte.

use crate::error::Result;

/// Declares [`AnySpliceDescriptor`] + its dispatcher from one tag list.
macro_rules! declare_splice_descriptors {
    (
        $lt:lifetime;
        $( $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented splice descriptor, plus an `Unknown`
        /// fallthrough that preserves the raw body for lossless round-trips.
        ///
        /// serde uses external tagging with camelCase variant keys.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnySpliceDescriptor<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// A `splice_descriptor_tag` with no typed implementation; `body`
            /// is the payload after the 2-byte (tag, length) header.
            Unknown {
                /// The raw `splice_descriptor_tag` byte.
                tag: u8,
                /// The raw body bytes (`descriptor_length` bytes).
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnySpliceDescriptor<$lt> {
                fn from(d: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(d)
                }
            }
        )+

        impl<$lt> AnySpliceDescriptor<$lt> {
            /// Every tag the generated dispatcher routes (excludes
            /// [`AnySpliceDescriptor::Unknown`]).
            pub const DISPATCHED_TAGS: &'static [u8] = &[$($tag),+];

            /// Diagnostic name of the contained descriptor — the type's
            /// [`SpliceDescriptorDef::NAME`](crate::traits::SpliceDescriptorDef::NAME)
            /// (`"AVAIL"`, `"SEGMENTATION"`, …); `"UNKNOWN"` for
            /// [`AnySpliceDescriptor::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::SpliceDescriptorDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one full descriptor (2-byte header included) by its tag.
            /// Unknown tags yield [`AnySpliceDescriptor::Unknown`].
            pub(crate) fn dispatch(tag: u8, full: &$lt [u8]) -> Result<Self> {
                use dvb_common::Parse;
                match tag {
                    $(
                        $tag => <$($path)::+>::parse(full).map(Self::$variant),
                    )+
                    _ => Ok(Self::Unknown { tag, body: &full[2..] }),
                }
            }

            /// Bytes this descriptor serializes to (header included).
            #[must_use]
            pub fn serialized_len(&self) -> usize {
                use dvb_common::Serialize;
                match self {
                    $(
                        Self::$variant(d) => d.serialized_len(),
                    )+
                    Self::Unknown { body, .. } => 2 + body.len(),
                }
            }

            /// Serialize this descriptor (header included) into `buf`.
            pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                use dvb_common::Serialize;
                match self {
                    $(
                        Self::$variant(d) => d.serialize_into(buf),
                    )+
                    Self::Unknown { tag, body } => {
                        let need = 2 + body.len();
                        if buf.len() < need {
                            return Err(crate::error::Error::OutputBufferTooSmall {
                                need,
                                have: buf.len(),
                            });
                        }
                        buf[0] = *tag;
                        buf[1] = body.len() as u8;
                        buf[2..need].copy_from_slice(body);
                        Ok(need)
                    }
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn tag_literals_match_descriptor_def() {
                use crate::traits::SpliceDescriptorDef;
                $(
                    assert_eq!(
                        $tag,
                        <$($path)::+ as SpliceDescriptorDef>::TAG,
                        concat!("tag literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as SpliceDescriptorDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_splice_descriptors! {'a;
    Avail        = 0x00 => crate::descriptors::avail::AvailDescriptor,
    Dtmf         = 0x01 => crate::descriptors::dtmf::DtmfDescriptor,
    Segmentation = 0x02 => crate::descriptors::segmentation::SegmentationDescriptor<'a>,
    Time         = 0x03 => crate::descriptors::time::TimeDescriptor,
    Audio        = 0x04 => crate::descriptors::audio::AudioDescriptor,
    DvbDas       = 0xF0 => crate::dvb_ta::das_descriptor::DvbDasDescriptor<'a>,
}

/// Lazily walk a raw splice descriptor loop, yielding one
/// [`AnySpliceDescriptor`] per entry. Never panics: a truncated final header or
/// body yields one `Err` and then the iterator fuses.
#[must_use]
pub fn parse_loop(bytes: &[u8]) -> SpliceDescriptorIter<'_> {
    SpliceDescriptorIter {
        bytes,
        pos: 0,
        fused: false,
    }
}

/// Iterator over a raw splice descriptor loop; see [`parse_loop`].
#[derive(Debug, Clone)]
pub struct SpliceDescriptorIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for SpliceDescriptorIter<'a> {
    type Item = Result<AnySpliceDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fused || self.pos >= self.bytes.len() {
            return None;
        }
        let rem = &self.bytes[self.pos..];
        if rem.len() < 2 {
            self.fused = true;
            return Some(Err(crate::error::Error::BufferTooShort {
                need: 2,
                have: rem.len(),
                what: "splice_descriptor header in loop",
            }));
        }
        let tag = rem[0];
        let len = rem[1] as usize;
        let total = 2 + len;
        if rem.len() < total {
            self.fused = true;
            return Some(Err(crate::error::Error::BufferTooShort {
                need: total,
                have: rem.len(),
                what: "splice_descriptor body in loop",
            }));
        }
        let full = &rem[..total];
        self.pos += total;
        Some(AnySpliceDescriptor::dispatch(tag, full))
    }
}

impl core::iter::FusedIterator for SpliceDescriptorIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::avail::AvailDescriptor;
    use crate::descriptors::header::CUEI;
    use dvb_common::Serialize;

    #[test]
    fn loop_dispatches_typed_and_unknown() {
        let avail = AvailDescriptor {
            identifier: CUEI,
            provider_avail_id: 0x1111_2222,
        }
        .to_bytes();
        let unknown = [0xEE, 0x02, 0xAB, 0xCD];
        let mut loop_bytes = avail.clone();
        loop_bytes.extend_from_slice(&unknown);

        let items: Vec<_> = parse_loop(&loop_bytes).collect::<Result<_>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnySpliceDescriptor::Avail(_)));
        assert_eq!(items[0].name(), "AVAIL");
        match &items[1] {
            AnySpliceDescriptor::Unknown { tag: 0xEE, body } => assert_eq!(*body, &[0xAB, 0xCD]),
            other => panic!("expected Unknown, got {other:?}"),
        }

        // Round-trip the whole loop through serialize_into.
        let mut out = Vec::new();
        for d in &items {
            let mut b = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut b).unwrap();
            out.extend_from_slice(&b);
        }
        assert_eq!(out, loop_bytes);
    }

    #[test]
    fn truncated_final_entry_yields_err_then_fuses() {
        let bytes = [0x00, 0x08, 0x43, 0x55]; // claims 8 body bytes, only 2 present
        let mut it = parse_loop(&bytes);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }
}
