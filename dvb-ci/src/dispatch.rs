//! `declare_resource_apdus!` — the resource-scoped APDU dispatch enum, declared
//! once (audit r10-O-4).
//!
//! Every CI-extension / CI Plus resource wraps its APDU structs in an enum that
//! parses by the leading 3-byte `apdu_tag` and serializes by delegating to the
//! variant. Eighteen of those were hand-written, each repeating the "length < 3
//! -> `BufferTooShort`; read the tag; `match`; else `UnexpectedApduTag`"
//! preamble plus two parallel `match self` arms that had to be kept in step.
//! One list line per variant now produces the enum, `parse`, both `Serialize`
//! methods and a `TAGS` list, following the workspace `declare_*!` pattern
//! (one list, no parallel arms to drift). A compile-time assertion rejects a
//! tag listed twice, which would silently shadow the later variant in `parse`.
//!
//! Two dispatch enums stay hand-written on purpose: `MultistreamHostControlApdu`
//! parses through `parse_mode(body, HostControlMode)` (the `tune_ip_req`
//! reserved-bit budget depends on the resource version, which the macro's
//! single `parse(body)` cannot carry), and `BroadcastServiceGatewayApdu`
//! delegates every tag it does not own to the inherited `ServiceGatewayApdu`
//! instead of rejecting it.

/// Declare a resource-scoped APDU dispatch enum from one `variant (Type) = tag`
/// list. The first variant's tag is what `UnexpectedApduTag::expected` names.
///
/// `$what` labels the errors (`"<what> apdu_tag"` for a short buffer, `<what>`
/// for an unexpected tag). Attributes (derives, serde, `non_exhaustive`) and
/// per-variant docs are passed through verbatim.
macro_rules! declare_resource_apdus {
    (
        $(#[$emeta:meta])*
        $vis:vis enum $name:ident $(<$lt:lifetime>)? ($what:literal) {
            $( $(#[$vmeta:meta])* $variant:ident ( $(#[$fmeta:meta])* $inner:ty ) = $tag:path ),+ $(,)?
        }
    ) => {
        $(#[$emeta])*
        $vis enum $name $(<$lt>)? {
            $( $(#[$vmeta])* $variant($(#[$fmeta])* $inner), )+
        }

        impl $(<$lt>)? $name $(<$lt>)? {
            /// Every `apdu_tag` this enum dispatches, in declaration order.
            pub const TAGS: &'static [$crate::tag::ApduTag] = &[ $( $tag ),+ ];

            /// The `apdu_tag` of the contained object (the list line it was
            /// declared with).
            #[must_use]
            pub fn tag(&self) -> $crate::tag::ApduTag {
                match self {
                    $( Self::$variant(_) => $tag, )+
                }
            }

            /// Parse an APDU, dispatching on the leading `apdu_tag`.
            pub fn parse(body: & $($lt)? [u8]) -> $crate::error::Result<Self> {
                if body.len() < 3 {
                    return Err($crate::error::Error::BufferTooShort {
                        need: 3,
                        have: body.len(),
                        what: concat!($what, " apdu_tag"),
                    });
                }
                let t = $crate::tag::ApduTag::from_bytes(body[0], body[1], body[2]);
                match t {
                    $( $tag => Ok(Self::$variant(<$inner>::parse(body)?)), )+
                    _ => Err($crate::error::Error::UnexpectedApduTag {
                        got: t.as_u24(),
                        expected: Self::TAGS[0].as_u24(),
                        what: $what,
                    }),
                }
            }
        }

        impl $(<$lt>)? ::broadcast_common::Serialize for $name $(<$lt>)? {
            type Error = $crate::error::Error;
            fn serialized_len(&self) -> usize {
                match self {
                    $( Self::$variant(o) => o.serialized_len(), )+
                }
            }
            fn serialize_into(&self, buf: &mut [u8]) -> $crate::error::Result<usize> {
                match self {
                    $( Self::$variant(o) => o.serialize_into(buf), )+
                }
            }
        }

        // A tag declared twice would make the later variant unreachable in
        // `parse`; reject it at compile time.
        const _: () = {
            let tags = [ $( $tag.as_u24() ),+ ];
            let mut i = 0;
            while i < tags.len() {
                let mut j = i + 1;
                while j < tags.len() {
                    assert!(
                        tags[i] != tags[j],
                        concat!("duplicate apdu_tag in declare_resource_apdus!(", stringify!($name), ")")
                    );
                    j += 1;
                }
                i += 1;
            }
        };
    };
}
pub(crate) use declare_resource_apdus;

/// Declare the `Parse`/`Serialize` pair of a header-only APDU (`length_field`
/// 0, unit struct) — previously copy-pasted as a local `empty_object!` in two
/// modules (audit r10-O-4).
macro_rules! declare_empty_apdu {
    ($ty:ty, $tag:expr, $what:literal) => {
        impl<'a> ::broadcast_common::Parse<'a> for $ty {
            type Error = $crate::error::Error;
            fn parse(bytes: &'a [u8]) -> $crate::error::Result<Self> {
                $crate::objects::parse_empty_apdu(bytes, $tag, $what)?;
                Ok(Self)
            }
        }
        impl ::broadcast_common::Serialize for $ty {
            type Error = $crate::error::Error;
            fn serialized_len(&self) -> usize {
                $crate::objects::empty_apdu_len()
            }
            fn serialize_into(&self, buf: &mut [u8]) -> $crate::error::Result<usize> {
                $crate::objects::serialize_empty_apdu($tag, buf)
            }
        }
    };
}
pub(crate) use declare_empty_apdu;

/// Declare the `Parse`/`Serialize` pair of an APDU whose whole body is one
/// opaque byte string held in `$field` — previously `opaque_object!` and
/// `opaque_dsmcc_object!`, the same macro under two names (audit r10-O-4).
macro_rules! declare_opaque_apdu {
    ($ty:ident, $field:ident, $tag:expr, $what:literal) => {
        impl<'a> ::broadcast_common::Parse<'a> for $ty<'a> {
            type Error = $crate::error::Error;
            fn parse(bytes: &'a [u8]) -> $crate::error::Result<Self> {
                let body = $crate::objects::parse_apdu_header(bytes, $tag, $what)?;
                Ok(Self { $field: body })
            }
        }
        impl ::broadcast_common::Serialize for $ty<'_> {
            type Error = $crate::error::Error;
            fn serialized_len(&self) -> usize {
                $crate::objects::apdu_len(self.$field.len())
            }
            fn serialize_into(&self, buf: &mut [u8]) -> $crate::error::Result<usize> {
                let body_len = self.$field.len();
                let pos = $crate::objects::write_apdu_header($tag, body_len, buf)?;
                buf[pos..pos + body_len].copy_from_slice(self.$field);
                Ok(pos + body_len)
            }
        }
    };
}
pub(crate) use declare_opaque_apdu;
