//! Per-crate data for the `clock33` guard. The scope is exactly these crates
//! (the ones that handle 33-bit PTS/DTS/`pts_time` or 27 MHz PCR arithmetic);
//! every `ALLOW` entry is line-pinned with its reason. Add a line for a new
//! crate that does timestamp arithmetic.

use crate::clock33::{BANNED_FULL, BANNED_NO_PCR, Crate};

pub const TABLE: &[Crate] = &[
    Crate {
        name: "buffering-runtime",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "compliance-probe",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "dvb-conformance",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "dvb-si",
        banned: BANNED_NO_PCR,
        allow: &[],
    },
    Crate {
        name: "dvb-tools",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "media-doctor",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "playout-runtime",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "scte35-splice",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "ssai-runtime",
        banned: BANNED_FULL,
        allow: &[(
            "src/splice.rs",
            98,
            "abs_diff",
            "condition_splice_point is the documented LINEAR (non-wrapping) variant for a clock that does not wrap; \
         the wrapping clock uses condition_splice_point_wrapping (circular distance, issue #1125)",
        )],
    },
    Crate {
        name: "timed-metadata",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "transmux",
        banned: BANNED_FULL,
        allow: &[],
    },
    Crate {
        name: "ts-fix",
        banned: BANNED_FULL,
        allow: &[],
    },
];
