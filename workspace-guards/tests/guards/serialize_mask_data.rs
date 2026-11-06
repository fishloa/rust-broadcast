//! Per-crate data for the `serialize_mask` guard. The scope is exactly these
//! crates (the serializers that pack struct fields into bit-fields); add a
//! line for a new crate of that kind. Every `allow` entry is pinned to the
//! exact source text and carries its reason.

use crate::serialize_mask::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "dvb-si",
        min_files: 100,
        allow: &[
            (
                "tables/rct.rs",
                "let sd_lo = (loc.start_date & 0x3F) as u8;",
                "start_date is range-checked to 9 bits (fit_mask) at the top of serialize_locator; this only splits the low 6 bits into byte 1",
            ),
            (
                "descriptors/extension/dts_hd.rs",
                "(a.bit_rate_or_scaled as u8 & 0x3F) << 2",
                "bit_rate_or_scaled is rejected above MAX_BIT_RATE (14 bits) earlier in serialize_into; this splits its low 6 bits",
            ),
            (
                "descriptors/dts.rs",
                "u64::from(self.",
                "every packed field is rejected above its *_MAX with a typed InvalidDescriptor at the top of serialize_into",
            ),
            (
                "descriptors/cable_delivery_system.rs",
                "(self.symbol_rate_bcd & 0x0F) as u8) << 4",
                "symbol_rate_bcd is the full 32-bit u32 (28-bit value + low nibble); nothing can overflow, this extracts the low nibble",
            ),
            (
                "descriptors/video_window.rs",
                "(self.vertical_offset & 0x0F) << 4",
                "vertical_offset is range-checked to 14 bits (fit_bits) earlier in serialize_into; this splits its low 4 bits",
            ),
            (
                "descriptors/target_background_grid.rs",
                "(self.vertical_size & 0x0F) << 4",
                "vertical_size is range-checked to 14 bits (fit_bits) earlier in serialize_into; this splits its low 4 bits",
            ),
        ],
    },
    Crate {
        name: "dvb-subtitle",
        min_files: 5,
        allow: &[
            (
                "segments/clut_definition.rs",
                "buf[2] = (self.y_value << 2) | (self.cr_value >> 2);",
                "reduced-range entry: Y<=63, Cr<=15, Cb<=15, T<=3 are rejected with InvalidValue immediately above (REDUCED_*_MAX); this only packs the already-checked values",
            ),
            (
                "segments/clut_definition.rs",
                "buf[3] = (self.cr_value << 6) | (self.cb_value << 2) | self.t_value;",
                "same reduced-range check as the previous line",
            ),
        ],
    },
    Crate {
        name: "scte35-splice",
        min_files: 5,
        allow: &[
            (
                "section.rs",
                "(self.tier & 0x0F) as u8) << 4",
                "tier is rejected above TIER_IGNORE (12 bits) at the top of serialize_into; these lines only split its low nibble into byte 11 (all three sites)",
            ),
            (
                "dvb_ta/das_descriptor.rs",
                "let (t, r) = (equivalent_segmentation_type & 0x0F, reserved & 0x0F);",
                "DasNibbleOrder::join masks by design; both callers (das_descriptor serialize_into, compact serialize_into_with_order) reject an over-range nibble with fit_mask before calling it",
            ),
        ],
    },
];
