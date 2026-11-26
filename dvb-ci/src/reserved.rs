//! [`Reserved`] — the shared typed run of `reserved` / `reserved_future_use` /
//! unused bits inside a fixed-width field, re-exported from
//! [`broadcast_core::reserved`] so a CAM object and a DVB descriptor spell a
//! reserved field the same way.
//!
//! The bits never take part in serde output (the structs mark the field
//! `serde(skip)`): they are a wire artefact, not part of the decoded view.

pub use broadcast_core::reserved::{Reserved, ReservedOnes, ReservedZero};
