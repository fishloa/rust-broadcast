//! Unified payload dispatch: [`AnyPayload`].
//!
//! [`AnyPayload`] is generated from a single declarative list
//! (`declare_payloads!`) — one line per T2-MI payload type.
//! The list is the single source of truth: it produces the enum, the
//! `From<T>` conversions, the `packet_type` → parser dispatcher, and a drift
//! test that pins each literal to the type's
//! [`crate::traits::PayloadDef::PACKET_TYPE`].
//!
//! # Dispatch contract
//!
//! [`AnyPayload::dispatch`] takes the **payload bytes only** (the bytes after
//! the 6-byte T2-MI header, up to but not including the 4-byte CRC trailer).
//! Each payload parser expects exactly those bytes — the header and CRC are NOT
//! passed in.  To recover the payload slice from a raw packet buffer use
//! [`crate::packet::Header::payload_bytes`].
//!
//! # Adding a payload
//!
//! 1. Create the module with the wire layout and the symmetric
//!    [`broadcast_common::Parse`] / [`broadcast_common::Serialize`] impls + round-trip
//!    tests (copy an existing module).
//! 2. `impl PayloadDef` for the type (`PACKET_TYPE` from the spec / the
//!    [`crate::packet::PacketType`] enum value, `NAME` in SCREAMING_SNAKE
//!    without the `_payload` suffix).
//! 3. Add one line to the `declare_payloads!` invocation below — the enum
//!    variant, dispatcher arm, and drift test are generated from it.
//! 4. The integration completeness test walks the generated
//!    [`AnyPayload::DISPATCHED_TYPES`] automatically — no test edits needed.

use alloc::boxed::Box;

/// Declares [`AnyPayload`] + its dispatcher from one packet-type list.
///
/// Each line is `Variant = 0xTYPE => module::Type[<'a>]`.
macro_rules! declare_payloads {
    (
        $lt:lifetime;
        $( $variant:ident = $ptype:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented T2-MI payload, plus an `Unknown` fallthrough.
        ///
        /// serde uses external tagging with camelCase variant keys.
        /// Variant names map 1:1 to the payload modules; see each module
        /// for the wire layout.
        ///
        /// # Dispatch contract
        ///
        /// Use [`AnyPayload::dispatch`] with the payload bytes (post-header,
        /// pre-CRC). See the module-level documentation for details.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        // Covariant in `$lt`: every variant holds only lifetime-parametrised
        // payload views or `&$lt [u8]` (`Unknown`), so the derive is sound.
        #[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
        #[non_exhaustive]
        pub enum AnyPayload<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// Runtime-registered custom payload (see [`PayloadRegistry`]).
            ///
            /// [`PayloadRegistry`]: crate::payload::registry::PayloadRegistry
            Other {
                /// The raw `packet_type` byte.
                packet_type: u8,
                /// The parsed, type-erased payload value. Call `downcast_ref` on
                /// it (see [`PayloadObject`][crate::payload::registry::PayloadObject]) to recover the concrete type.
                #[cfg_attr(
                    feature = "serde",
                    serde(serialize_with = "crate::payload::registry::serialize_erased")
                )]
                value: Box<dyn crate::payload::registry::PayloadObject>,
            },
            /// Packet type with no typed implementation; `body` contains the
            /// raw payload bytes (post-header, pre-CRC).
            Unknown {
                /// The raw `packet_type` byte.
                packet_type: u8,
                /// The raw payload bytes.
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyPayload<$lt> {
                fn from(p: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(p)
                }
            }
        )+

        impl<$lt> AnyPayload<$lt> {
            /// Every `packet_type` the generated dispatcher routes (excludes
            /// [`AnyPayload::Unknown`]).
            pub const DISPATCHED_TYPES: &'static [u8] = &[$($ptype),+];

            /// Diagnostic name of the contained payload — the type's
            /// [`PayloadDef::NAME`](crate::traits::PayloadDef::NAME)
            /// (`"BBFRAME"`, `"L1_CURRENT"`, …); `"CUSTOM"` for
            /// [`AnyPayload::Other`] (runtime-registered) and `"UNKNOWN"`
            /// for [`AnyPayload::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::PayloadDef>::NAME,
                    )+
                    Self::Other { .. } => "CUSTOM",
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one payload by its `packet_type`.
            ///
            /// `payload_bytes` must be the **payload-only slice** (bytes after
            /// the 6-byte T2-MI header, before the 4-byte CRC trailer).
            ///
            /// Returns `None` when `packet_type` has no typed implementation
            /// (the caller turns that into [`AnyPayload::Unknown`]).
            /// Returns `Some(Err)` on a typed parse failure for a recognised type.
            ///
            /// See the [module-level documentation][self] for the dispatch
            /// contract (payload-only bytes, header and CRC excluded).
            pub fn dispatch(
                packet_type: u8,
                payload_bytes: &$lt [u8],
            ) -> Option<crate::Result<Self>> {
                use broadcast_common::Parse;
                match packet_type {
                    $(
                        $ptype => Some(
                            <$($path)::+>::parse(payload_bytes).map(Self::$variant),
                        ),
                    )+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn packet_type_literals_match_payload_def() {
                use crate::traits::PayloadDef;
                $(
                    assert_eq!(
                        $ptype,
                        <$($path)::+ as PayloadDef>::PACKET_TYPE,
                        concat!("PACKET_TYPE literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as PayloadDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_payloads! {'a;
    // TS 102 773 Table 1 — all 12 defined packet types in numerical order.
    Bbframe              = 0x00 => crate::payload::bbframe::BbframePayload<'a>,
    AuxIq                = 0x01 => crate::payload::aux_iq::AuxIqPayload<'a>,
    ArbitraryCells       = 0x02 => crate::payload::arbitrary_cells::ArbitraryCellsPayload<'a>,
    L1Current            = 0x10 => crate::payload::l1_current::L1CurrentPayload<'a>,
    L1Future             = 0x11 => crate::payload::l1_future::L1FuturePayload<'a>,
    P2Bias               = 0x12 => crate::payload::p2_bias::P2BiasPayload,
    Timestamp            = 0x20 => crate::payload::timestamp::T2TimestampPayload,
    IndividualAddressing = 0x21 => crate::payload::individual_addressing::IndividualAddressingPayload<'a>,
    FefNull              = 0x30 => crate::payload::fef_null::FefNullPayload,
    FefIq                = 0x31 => crate::payload::fef_iq::FefIqPayload<'a>,
    FefComposite         = 0x32 => crate::payload::fef_composite::FefCompositePayload,
    FefSubpart           = 0x33 => crate::payload::fef_subpart::FefSubPartPayload<'a>,
}

impl<'a> AnyPayload<'a> {
    /// Parse one payload by its `packet_type`, preferring the registry's custom
    /// parsers over the built-in dispatch.
    ///
    /// `payload_bytes` must be the **payload-only slice** (bytes after the
    /// 6-byte T2-MI header, before the 4-byte CRC trailer).
    ///
    /// # Precedence
    ///
    /// 1. If `registry` holds a custom parser for `packet_type`, it is called;
    ///    the result becomes [`AnyPayload::Other`] (or an error on parse failure).
    /// 2. Otherwise, falls back to the built-in [`AnyPayload::dispatch`].
    /// 3. If neither route handles `packet_type`, returns `None` — the caller
    ///    turns that into [`AnyPayload::Unknown`].
    ///
    /// See the [module-level documentation][self] for the dispatch contract
    /// (payload-only bytes, header and CRC excluded).
    pub fn dispatch_with(
        registry: &crate::payload::registry::PayloadRegistry,
        packet_type: u8,
        payload_bytes: &'a [u8],
    ) -> Option<crate::Result<Self>> {
        if let Some(parse_fn) = registry.lookup(packet_type) {
            return Some(match parse_fn(payload_bytes) {
                Ok(value) => Ok(Self::Other { packet_type, value }),
                Err(e) => Err(e),
            });
        }
        // Fall back to built-in dispatch
        Self::dispatch(packet_type, payload_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Completeness ─────────────────────────────────────────────────────────

    /// `AnyPayload::name()` reflects `PayloadDef::NAME`; `UNKNOWN` for unknowns.
    #[test]
    fn name_maps_variant_to_payloaddef_name() {
        let bb = AnyPayload::dispatch(0x00, &[0x00, 0x00, 0x00])
            .expect("dispatched")
            .expect("valid bbframe payload");
        assert_eq!(bb.name(), "BBFRAME");
        let unknown = AnyPayload::Unknown {
            packet_type: 0x7F,
            body: &[],
        };
        assert_eq!(unknown.name(), "UNKNOWN");

        // A runtime-registered custom payload reports "CUSTOM".
        use crate::payload::registry::PayloadRegistry;

        // An unregistered packet_type yields no custom dispatch.
        let empty = PayloadRegistry::new();
        assert!(AnyPayload::dispatch_with(&empty, 0x40, &[]).is_none());

        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct NameTestPayload {
            _x: u8,
        }

        impl<'a> broadcast_common::Parse<'a> for NameTestPayload {
            type Error = crate::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.is_empty() {
                    return Err(crate::Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "NameTest",
                    });
                }
                Ok(Self { _x: bytes[0] })
            }
        }

        impl crate::traits::PayloadDef<'_> for NameTestPayload {
            const PACKET_TYPE: u8 = 0x41;
            const NAME: &'static str = "NAME_TEST";
        }

        let mut reg = PayloadRegistry::new();
        reg.register::<NameTestPayload>();
        let parsed = AnyPayload::dispatch_with(&reg, 0x41, &[0xAA])
            .unwrap()
            .unwrap();
        assert_eq!(parsed.name(), "CUSTOM");
    }

    /// Every entry in DISPATCHED_TYPES must dispatch to a non-Unknown variant.
    #[test]
    fn every_dispatched_type_routes_non_unknown() {
        // Minimal valid payload bytes for each packet type (all RFU = 0 — the
        // parsers reject non-zero reserved bits). See each payload module's
        // own tests for full boundary coverage.

        // 0x00 BBFrame: frame_idx(1) + plp_id(1) + intl_frame_start+rfu(1) = 3 bytes.
        let bbframe_bytes: &[u8] = &[0x00, 0x00, 0x00];
        // 0x01 AuxIq: frame_idx(1) + aux_id(4bits, must be 1..=15)+rfu(4bits)(1) + rfu(1) = 3 bytes.
        // aux_id=1: byte1 = (1<<4) = 0x10.
        let aux_iq_bytes: &[u8] = &[0x00, 0x10, 0x00];
        // 0x02 ArbitraryCells: 8-byte header (rfu bytes 3,4 = 0, byte5 top 2 = 0).
        let arb_cells_bytes: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        // 0x10 L1Current: frame_idx(1) + freq_source(2bits)+rfu(6bits)(1) = 2 bytes.
        let l1_current_bytes: &[u8] = &[0x00, 0x00];
        // 0x11 L1Future: frame_idx(1) + rfu(1) = 2 bytes.
        let l1_future_bytes: &[u8] = &[0x00, 0x00];
        // 0x12 P2Bias: 5 bytes, all rfu = 0.
        let p2_bias_bytes: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00];
        // 0x20 Timestamp: 11 bytes, rfu top 4 bits of byte0 = 0, bw=0 (1.7 MHz).
        let timestamp_bytes: &[u8] = &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        // 0x21 IndividualAddressing: rfu(1) + length(1, value 0) = 2 bytes.
        let indiv_addr_bytes: &[u8] = &[0x00, 0x00];
        // 0x30 FefNull: fef_idx(1) + rfu(1, must be 0) + s1_field+s2_field(1) = 3 bytes.
        let fef_null_bytes: &[u8] = &[0x00, 0x00, 0x00];
        // 0x31 FefIq: fef_idx(1) + rfu(1, must be 0) + s1+s2(1) = 3 bytes.
        let fef_iq_bytes: &[u8] = &[0x00, 0x00, 0x00];
        // 0x32 FefComposite: 8 bytes. byte1 [7]=rfu1=0, bytes2-5=rfu2=0.
        let fef_composite_bytes: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        // 0x33 FefSubpart: 15 bytes.
        // bytes 3-6 = rfu1 = 0, byte 11 = rfu2 = 0, byte 12 top 2 = 0.
        // subpart_variety bytes 9-10 = 0x0000 = Null.
        let fef_subpart_bytes: &[u8] = &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00,
        ];

        let fixtures: &[(u8, &[u8])] = &[
            (0x00, bbframe_bytes),
            (0x01, aux_iq_bytes),
            (0x02, arb_cells_bytes),
            (0x10, l1_current_bytes),
            (0x11, l1_future_bytes),
            (0x12, p2_bias_bytes),
            (0x20, timestamp_bytes),
            (0x21, indiv_addr_bytes),
            (0x30, fef_null_bytes),
            (0x31, fef_iq_bytes),
            (0x32, fef_composite_bytes),
            (0x33, fef_subpart_bytes),
        ];

        for &(pt, bytes) in fixtures {
            let result = AnyPayload::dispatch(pt, bytes);
            assert!(result.is_some(), "0x{pt:02x} returned None from dispatch");
            let parsed = result.unwrap();
            assert!(
                parsed.is_ok(),
                "0x{pt:02x} dispatch parse failed: {:?}",
                parsed.unwrap_err()
            );
            assert!(
                !matches!(parsed.unwrap(), AnyPayload::Unknown { .. }),
                "0x{pt:02x} was dispatched to Unknown"
            );
        }
    }

    /// DISPATCHED_TYPES has exactly 12 entries (one per TS 102 773 Table 1 type).
    #[test]
    fn dispatched_types_count_is_twelve() {
        assert_eq!(AnyPayload::DISPATCHED_TYPES.len(), 12);
    }

    /// DISPATCHED_TYPES contains all 12 defined packet_type values.
    #[test]
    fn dispatched_types_contains_all_defined_packet_types() {
        let expected = [
            0x00u8, 0x01, 0x02, 0x10, 0x11, 0x12, 0x20, 0x21, 0x30, 0x31, 0x32, 0x33,
        ];
        for pt in expected {
            assert!(
                AnyPayload::DISPATCHED_TYPES.contains(&pt),
                "0x{pt:02x} missing from DISPATCHED_TYPES"
            );
        }
    }

    // ── Unknown fallthrough ───────────────────────────────────────────────────

    /// An undispatched packet_type returns None from dispatch (caller makes Unknown).
    #[test]
    fn undispatched_packet_type_returns_none() {
        // 0x22..=0x2F are RFU, never defined.
        assert!(AnyPayload::dispatch(0x22, &[]).is_none());
        assert!(AnyPayload::dispatch(0xFF, &[]).is_none());
    }

    // ── From impls ────────────────────────────────────────────────────────────

    #[test]
    fn from_bbframe_payload_into_any_payload() {
        use crate::payload::bbframe::BbframePayload;
        let p = BbframePayload {
            frame_idx: 1,
            plp_id: 2,
            intl_frame_start: false,
            bbframe: &[],
        };
        let any = AnyPayload::from(p);
        assert!(matches!(any, AnyPayload::Bbframe(_)));
    }

    #[test]
    fn from_fef_null_payload_into_any_payload() {
        use crate::payload::fef_null::{FefNullPayload, S1Field};
        let p = FefNullPayload {
            fef_idx: 0,
            s1_field: S1Field::V0,
            s2_field: 0,
        };
        let any = AnyPayload::from(p);
        assert!(matches!(any, AnyPayload::FefNull(_)));
    }

    // ── serde ─────────────────────────────────────────────────────────────────

    #[cfg(feature = "serde")]
    #[test]
    fn bbframe_serializes_as_camel_case_external_tag() {
        use crate::payload::bbframe::BbframePayload;
        let p = BbframePayload {
            frame_idx: 0x42,
            plp_id: 0x05,
            intl_frame_start: true,
            bbframe: &[],
        };
        let any = AnyPayload::Bbframe(p);
        let json = serde_json::to_value(&any).unwrap();
        assert!(
            json.get("bbframe").is_some(),
            "expected camelCase 'bbframe' key, got: {json}"
        );
        assert_eq!(json["bbframe"]["frame_idx"], 0x42);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn unknown_serializes_with_packet_type_and_body() {
        let any = AnyPayload::Unknown {
            packet_type: 0x22,
            body: &[0xDE, 0xAD],
        };
        let json = serde_json::to_value(&any).unwrap();
        assert!(
            json.get("unknown").is_some(),
            "expected 'unknown' key, got: {json}"
        );
        assert_eq!(json["unknown"]["packet_type"], 0x22);
    }
}
