//! BER-encoded KLV Length — SMPTE ST 377-1:2019 §6.3.4
//! (`private/specs-verified/smpte/st377-1/st377-1.md`), the ISO/IEC 8825-1
//! Basic Encoding Rules constrained for MXF.
//!
//! The codec itself is shared with every other KLV user in the workspace and
//! lives in [`broadcast_common::klv`]; this module re-exports it so the
//! `st377_1::…` and `st377_1::ber::…` paths an MXF consumer already uses keep
//! resolving. [`ber_length_size`] and [`encode_ber_length`] (the canonical
//! minimal form) are not re-exported here: every MXF length token goes through
//! the width-preserving pair [`ber_length_size_for`] / [`encode_ber_length_as`].

pub use broadcast_common::klv::{
    BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as,
};
