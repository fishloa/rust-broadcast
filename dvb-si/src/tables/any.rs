//! Unified table dispatch: [`AnyTable`].
//!
//! [`AnyTable`] is generated from a single declarative list
//! (`declare_tables!`) — one line per crate-implemented table type.
//! The list is the single source of truth: it produces the enum, the
//! `From<T>` conversions, the table_id range dispatcher, and a drift test
//! that pins each range literal to the type's
//! [`crate::traits::TableDef::TABLE_ID_RANGES`].
//!
//! [`AnyTable::parse`] dispatches on the first byte (table_id) using range
//! patterns. An unrecognised table_id yields
//! `AnyTable::Unknown { table_id, raw }` — the full section bytes are
//! retained.
//!
//! [`AnyTable::parse_as`] is a type-keyed thin alias to `T::parse` — it
//! bypasses dispatch entirely and lets callers obtain, for example, a
//! [`crate::tables::mpe::MpeDatagramSection`] for a `0x3E` section that the
//! default dispatcher would route to `DsmccSection`.
//!
//! ```
//! use dvb_common::Serialize;
//! use dvb_si::tables::AnyTable;
//! use dvb_si::tables::pat::{Pat, PatEntry};
//!
//! // Serialize a small PAT, then dispatch the bytes back through AnyTable::parse.
//! let pat = Pat {
//!     transport_stream_id: 1, version_number: 0, current_next_indicator: true,
//!     section_number: 0, last_section_number: 0,
//!     entries: vec![PatEntry { program_number: 1, pid: 0x0100 }],
//! };
//! let mut section = vec![0u8; pat.serialized_len()];
//! pat.serialize_into(&mut section).unwrap();
//!
//! match AnyTable::parse(&section).unwrap() {
//!     AnyTable::Pat(parsed) => assert_eq!(parsed.entries[0].pid, 0x0100),
//!     other => panic!("expected Pat, got {other:?}"),
//! }
//! ```
//!
//! # Adding a table
//!
//! 1. Create the module with the wire layout, a `pub const TABLE_ID: u8` (or
//!    `TABLE_ID_FIRST`/`TABLE_ID_LAST` for a range), and the symmetric
//!    [`dvb_common::Parse`]/[`dvb_common::Serialize`] impls + round-trip tests
//!    (copy an existing module).
//! 2. `impl TableDef` for the type (`TABLE_ID_RANGES` covering the full
//!    table_id range, `NAME` in SCREAMING_SNAKE without the `_section` or
//!    `_table` suffix).
//! 3. Add one line to the `declare_tables!` invocation below — the enum
//!    variant, dispatcher arm, and drift test entry are generated from it.
//!    If the type should NOT be auto-dispatched (e.g. `MpeDatagramSection`,
//!    whose `0x3E` id is claimed by the `DsmccSection` range), add it to the
//!    `@no_dispatch` section instead.
//! 4. The disjointness test in `declare_tables_tests` catches any overlapping
//!    range entries automatically — no manual test edits needed.

/// Declares [`AnyTable`] + its dispatcher from one range list.
///
/// Each dispatch line is `Variant = [lo..=hi, …] => module::Type[<'a>]`.
/// The optional trailing `@no_dispatch …` section adds variants that are NOT
/// reachable from the generated dispatcher — the variant exists for callers
/// that obtain the type via `AnyTable::parse_as` or direct `T::parse`.
macro_rules! declare_tables {
    (
        $lt:lifetime;
        $( $variant:ident = [ $( $lo:literal ..= $hi:literal ),+ ] => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
        $( ; @no_dispatch $( $nd_variant:ident => $($nd_path:ident)::+ $(<$nd_plt:lifetime>)? ),+ $(,)? )?
    ) => {
        /// Every crate-implemented table, plus an `Unknown` fallthrough.
        ///
        /// serde uses external tagging with camelCase variant keys — a parsed
        /// PAT serializes as `{"pat": {…}}`.
        /// Variant names map 1:1 to the table modules; see each module for the
        /// wire layout.
        ///
        /// `0x3E` (`datagram_section`) is routed to `DsmccSection` by the
        /// default dispatcher. The typed MPE view is reachable via
        /// `AnyTable::parse_as::<MpeDatagramSection>` or
        /// `MpeDatagramSection::parse` directly; the `MpeDatagram` variant
        /// exists in this enum for API completeness but is never produced by
        /// `AnyTable::parse`.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        // Covariant in `$lt`: every variant holds only lifetime-parametrised
        // table views or `&$lt [u8]` (`Unknown`), so the derived impl is sound.
        #[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
        #[non_exhaustive]
        pub enum AnyTable<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            $($(
                #[allow(missing_docs)]
                $nd_variant($($nd_path)::+ $(<$nd_plt>)?),
            )+)?
            /// table_id with no typed implementation; `raw` is the full
            /// section bytes including the table_id header.
            Unknown {
                /// The raw table_id byte.
                table_id: u8,
                /// The raw section bytes (full, header included).
                raw: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnyTable<$lt> {
                fn from(t: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(t)
                }
            }
        )+
        $($(
            impl<$lt> From<$($nd_path)::+ $(<$nd_plt>)?> for AnyTable<$lt> {
                fn from(t: $($nd_path)::+ $(<$nd_plt>)?) -> Self {
                    Self::$nd_variant(t)
                }
            }
        )+)?

        impl<$lt> AnyTable<$lt> {
            /// All table_id ranges covered by the auto-dispatcher (excludes
            /// `@no_dispatch` variants). Each entry is `(lo, hi)` inclusive.
            pub const DISPATCHED_RANGES: &'static [(u8, u8)] =
                &[$( $( ($lo, $hi) ),+ ),+];

            /// Diagnostic name of the contained table — the type's
            /// [`TableDef::NAME`](crate::traits::TableDef::NAME)
            /// (`"EVENT_INFORMATION"`, `"PROGRAM_ASSOCIATION"`, …);
            /// `"UNKNOWN"` for [`AnyTable::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::TableDef>::NAME,
                    )+
                    $($(
                        Self::$nd_variant(_) =>
                            <$($nd_path)::+ as crate::traits::TableDef>::NAME,
                    )+)?
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Dispatch one complete section by its table_id (byte 0).
            ///
            /// Returns `Err(BufferTooShort)` when `bytes` is empty.
            /// Unknown table_ids produce `Ok(AnyTable::Unknown { … })`.
            ///
            /// # Errors
            /// - [`crate::Error::BufferTooShort`] — `bytes` is empty.
            /// - Any parse error from the dispatched type.
            pub fn parse(bytes: &$lt [u8]) -> crate::Result<Self> {
                let table_id = *bytes.first().ok_or(crate::Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "section table_id",
                })?;
                match table_id {
                    $(
                        $( $lo..=$hi )|+ => {
                            <$($path)::+ as dvb_common::Parse>::parse(bytes).map(Self::$variant)
                        }
                    )+
                    _ => Ok(Self::Unknown { table_id, raw: bytes }),
                }
            }

            /// Type-keyed parse: bypass the dispatcher and parse `bytes`
            /// directly as `T`. Useful for types excluded from the default
            /// dispatch, e.g.:
            ///
            /// ```rust
            /// use dvb_si::tables::AnyTable;
            /// use dvb_si::tables::mpe::MpeDatagramSection;
            ///
            /// // A deliberately-too-short slice: parse_as propagates the
            /// // BufferTooShort error from MpeDatagramSection::parse.
            /// let err = AnyTable::parse_as::<MpeDatagramSection>(&[0x3E, 0x00]);
            /// assert!(err.is_err());
            /// ```
            ///
            /// # Errors
            /// Propagates `T::parse` errors.
            pub fn parse_as<T>(bytes: &$lt [u8]) -> crate::Result<T>
            where
                T: crate::traits::TableDef<$lt>,
            {
                <T as dvb_common::Parse>::parse(bytes)
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn ranges_match_tabledef() {
                use crate::traits::TableDef;
                $(
                    assert_eq!(
                        &[ $( ($lo, $hi) ),+ ][..],
                        <$($path)::+ as TableDef>::TABLE_ID_RANGES,
                        concat!("TABLE_ID_RANGES drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as TableDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
                $($(
                    assert!(
                        !<$($nd_path)::+ as TableDef>::NAME.is_empty(),
                        concat!("empty NAME for no-dispatch ", stringify!($nd_variant)),
                    );
                )+)?
            }

            #[test]
            fn dispatched_ranges_are_disjoint() {
                // Collect all (lo, hi) pairs, sort by lo, then check no
                // two adjacent entries overlap.
                let mut ranges: Vec<(u8, u8)> = vec![
                    $( $( ($lo, $hi), )+ )+
                ];
                ranges.sort_by_key(|r| r.0);
                for w in ranges.windows(2) {
                    let (_, prev_hi) = w[0];
                    let (next_lo, _) = w[1];
                    assert!(
                        next_lo > prev_hi,
                        "overlapping dispatch ranges: {w:?}",
                    );
                }
            }
        }
    };
}

declare_tables! {'a;
    // MPEG-2 systems tables (ISO/IEC 13818-1).
    Pat       = [0x00..=0x00] => crate::tables::pat::Pat,
    Cat       = [0x01..=0x01] => crate::tables::cat::Cat<'a>,
    Pmt       = [0x02..=0x02] => crate::tables::pmt::Pmt<'a>,
    Tsdt      = [0x03..=0x03] => crate::tables::tsdt::Tsdt<'a>,
    // DSM-CC sections (ISO/IEC 13818-6) — 0x3E is included; the MPE typed
    // view (`MpeDatagramSection`) is reachable via `AnyTable::parse_as` or
    // `MpeDatagramSection::parse`.
    DsmccSection = [0x3A..=0x3F] => crate::tables::dsmcc::DsmccSection<'a>,
    // DVB tables (ETSI EN 300 468).
    Nit       = [0x40..=0x41] => crate::tables::nit::Nit<'a>,
    Sdt       = [0x42..=0x42, 0x46..=0x46] => crate::tables::sdt::Sdt<'a>,
    Bat       = [0x4A..=0x4A] => crate::tables::bat::Bat<'a>,
    Unt       = [0x4B..=0x4B] => crate::tables::unt::Unt<'a>,
    Int       = [0x4C..=0x4C] => crate::tables::int::Int<'a>,
    Sat       = [0x4D..=0x4D] => crate::tables::sat::Sat<'a>,
    Eit       = [0x4E..=0x6F] => crate::tables::eit::Eit<'a>,
    Tdt       = [0x70..=0x70] => crate::tables::tdt::Tdt,
    Rst       = [0x71..=0x71] => crate::tables::rst::Rst,
    St        = [0x72..=0x72] => crate::tables::st::St,
    Tot       = [0x73..=0x73] => crate::tables::tot::Tot<'a>,
    Ait       = [0x74..=0x74] => crate::tables::ait::Ait<'a>,
    Container = [0x75..=0x75] => crate::tables::container::Container<'a>,
    Rct       = [0x76..=0x76] => crate::tables::rct::Rct<'a>,
    Cit       = [0x77..=0x77] => crate::tables::cit::Cit<'a>,
    MpeFec    = [0x78..=0x78] => crate::tables::mpe_fec::MpeFec<'a>,
    Rnt       = [0x79..=0x79] => crate::tables::rnt::Rnt<'a>,
    MpeIfec   = [0x7A..=0x7A] => crate::tables::mpe_ifec::MpeIfec<'a>,
    ProtectionMessage    = [0x7B..=0x7B] => crate::tables::protection_message::ProtectionMessageSection<'a>,
    DownloadableFontInfo = [0x7C..=0x7C] => crate::tables::downloadable_font_info::DownloadableFontInfoSection<'a>,
    Dit       = [0x7E..=0x7E] => crate::tables::dit::Dit,
    Sit       = [0x7F..=0x7F] => crate::tables::sit::Sit<'a>;
    // MPE datagram_section (ETSI EN 301 192 §7.1): table_id 0x3E overlaps
    // the DsmccSection range above, so it is NOT auto-dispatched. Use
    // `AnyTable::parse_as::<MpeDatagramSection>(bytes)` for the typed view.
    @no_dispatch
    MpeDatagram => crate::tables::mpe::MpeDatagramSection<'a>,
}
