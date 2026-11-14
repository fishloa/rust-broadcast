//! Per-crate data for the `deadlines` guard. The scope is the crates that
//! carry a host clock as `broadcast_common::stage::Timestamp` and arm timers
//! from it; a crate that gains a `Timestamp` input gets a line here in the same
//! change. Every `ALLOW` entry is line-pinned with its reason.

use crate::deadlines::Crate;

pub const TABLE: &[Crate] = &[
    Crate {
        name: "broadcast-client",
        allow: &[],
    },
    Crate {
        name: "media-plane",
        allow: &[],
    },
];
