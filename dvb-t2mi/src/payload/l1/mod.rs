//! EN 302 755 §7.2 L1-pre / L1-post signalling parser.
//!
//! As carried in ETSI TS 102 773 T2-MI packets (types `0x10` / `0x11`).
//!
//! - [`L1Pre`] — 168-bit L1-pre information block (Figure 25).
//! - [`L1PostConfigurable`] — configurable L1-post fields (Figure 27).
//! - [`L1PostDynamic`] — dynamic L1-post fields (Figure 28).
//! - [`L1Post`] — assembled decoded L1-post.
//! - Enum types for every spec-defined field in `enums`.

pub mod enums;
pub mod post;
pub mod pre;

pub use enums::{
    AuxStreamType, GuardInterval, L1CodeRate, L1FecType, L1Modulation, PaprReduction,
    PaprReductionV0, PaprReductionVn, PilotPattern, PlpCodeRate, PlpFecType, PlpMode,
    PlpModulation, PlpPayloadType, PlpType, T2Version, TxInputStreamType,
};
pub use post::{
    AuxConfig, AuxDynamic, FefInfo, L1ExtBlock, L1Post, L1PostConfigurable, L1PostDynamic,
    PlpConfig, PlpDynamic, RfFrequency,
};
pub use pre::L1Pre;
