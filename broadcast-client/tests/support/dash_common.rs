//! The DASH test scaffolding of `dash-runtime` (the DESIGN 3.6 `World`, the synthesised live origin,
//! the real captures and the static loopback origin), shared so the session suites drive the same
//! origins as the core suites. Cross-crate tests live in the highest crate.

#![allow(dead_code)]

#[path = "../../../dash-runtime/tests/common/world.rs"]
pub mod world;

#[path = "../../../dash-runtime/tests/common/synth.rs"]
pub mod synth;

#[path = "../../../dash-runtime/tests/common/capture.rs"]
pub mod capture;

#[path = "../../../dash-runtime/tests/common/loopback.rs"]
pub mod loopback;
