//! BIOP (Broadcast Inter-ORB Protocol) object-carousel layer.
//!
//! Implements the DVB-profiled subset of ISO/IEC 13818-6 §11 as documented
//! in `dvb-si/docs/text/iso_13818_6/` (transcribed from ETSI TR 101 202 §4.7).
//!
//! # Layer overview
//!
//! BIOP messages live inside the **complete modules** assembled by
//! [`crate::carousel::ModuleReassembler`].  The top-level entry points are:
//!
//! - [`ior`] — `IOP::IOR`, tagged profiles, `ObjectLocation`, `ConnBinder`,
//!   `ServiceLocation`, `NsapAddress`.
//! - [`message`] — BIOP message header + `DirectoryMessage`, `FileMessage`,
//!   `ModuleInfo`, `ServiceGatewayInfo` and the `BiopMessage` dispatch enum.
//! - [`fs`] — `CarouselFs` — walk a set of reassembled modules as a virtual
//!   filesystem; resolves paths to `&[u8]` file content.
//!
//! # Constants
//!
//! Tagged-profile and component tags (32-bit) from TR 101 202 §4.7.3:

/// `profileId_tag` for the BIOP Profile Body — TR 101 202 §4.7.3.2.
pub const TAG_BIOP: u32 = 0x49534F06;
/// `profileId_tag` for the Lite Options Profile Body — TR 101 202 §4.7.3.3.
pub const TAG_LITE_OPTIONS: u32 = 0x49534F05;
/// `componentId_tag` for BIOP::ObjectLocation — TR 101 202 Table 4.5.
pub const TAG_OBJECT_LOCATION: u32 = 0x49534F50;
/// `componentId_tag` for DSM::ConnBinder — TR 101 202 Table 4.5.
pub const TAG_CONN_BINDER: u32 = 0x49534F40;
/// `componentId_tag` for DSM::ServiceLocation — TR 101 202 Table 4.7.
pub const TAG_SERVICE_LOCATION: u32 = 0x49534F46;

/// Tap `use` value — module delivery parameters (BIOP_DELIVERY_PARA_USE).
/// TR 101 202 §4.7.3.2, Table 4.6.
pub const BIOP_DELIVERY_PARA_USE: u16 = 0x0016;
/// Tap `use` value — BIOP objects in Modules (BIOP_OBJECT_USE).
/// TR 101 202 §4.7.3.2, Table 4.6.
pub const BIOP_OBJECT_USE: u16 = 0x0017;

/// `bindingType` value — name bound to a non-Directory/ServiceGateway object.
/// TR 101 202 §4.7.4.1, Table 4.9.
pub const BINDING_NOBJECT: u8 = 0x01;
/// `bindingType` value — name bound to a Directory or ServiceGateway.
/// TR 101 202 §4.7.4.1, Table 4.9.
pub const BINDING_NCONTEXT: u8 = 0x02;

/// BIOP message header magic bytes — `"BIOP"` as a 32-bit big-endian integer.
/// TR 101 202 §4.7.4, Table 4.9.
pub const BIOP_MAGIC: u32 = 0x42494F50;
/// BIOP version major — 1. TR 101 202 §4.7.4.
pub const BIOP_VERSION_MAJOR: u8 = 0x01;
/// BIOP version minor — 0. TR 101 202 §4.7.4.
pub const BIOP_VERSION_MINOR: u8 = 0x00;
/// CDR byte-order flag for big-endian (DVB mandatory). TR 101 202 §4.7.3.
pub const BYTE_ORDER_BIG_ENDIAN: u8 = 0x00;

/// `compressed_module_descriptor` tag in the ModuleInfo `userInfo` loop.
/// TR 101 202 §4.6.6.10.
pub const COMPRESSED_MODULE_DESCRIPTOR_TAG: u8 = 0x09;

/// Computes the wire-derived byte range `[pos, pos+len)`, bounded by `end`.
///
/// Every length field this guards (`messageBody_length`, `objectInfo_length`,
/// `content_length`, `type_id_length`, `profile_data_length`, CosNaming
/// `id_length`/`kind_length`, `initialContext_length`, …) is a 32-bit wire
/// value per TR 101 202 §4.7.3/4.7.4/4.7.5; a plain `pos + len` can wrap
/// `usize` on a 32-bit target when `len` is near `u32::MAX`, after which the
/// `> end` guard would wrongly pass. `checked_add` makes that impossible.
pub(super) fn span(
    pos: usize,
    len: u32,
    end: usize,
) -> crate::error::Result<core::ops::Range<usize>> {
    let len = len as usize;
    pos.checked_add(len)
        .filter(|&stop| stop <= end)
        .map(|stop| pos..stop)
        .ok_or(crate::error::Error::SectionLengthOverflow {
            declared: len,
            available: end.saturating_sub(pos),
        })
}

pub mod fs;
pub mod ior;
pub mod message;

pub use fs::{CarouselFs, CarouselObject};
pub use ior::{
    BiopProfileBody, ConnBinder, Ior, LiteComponent, LiteOptionsProfileBody, NameComponent,
    NsapAddress, ObjectKind, ObjectLocation, ServiceLocation, TaggedProfile, Tap,
};
pub use message::{
    Binding, BiopMessage, CompressedModuleDescriptor, DirectoryMessage, DsmStreamInfo, FileMessage,
    ModuleInfo, ServiceContext, ServiceGatewayInfo, StreamEventMessage, StreamMessage,
};

#[cfg(test)]
mod tests {
    use super::span;

    #[test]
    fn span_ok_within_bounds() {
        assert_eq!(span(10, 5, 20).unwrap(), 10..15);
        assert_eq!(span(0, 0, 0).unwrap(), 0..0);
    }

    #[test]
    fn span_rejects_len_past_end() {
        assert!(span(10, 11, 20).is_err());
    }

    /// `pos` alone (not just `pos + len`) can sit near `usize::MAX` — this is
    /// the case a plain `pos + len` addition can overflow/panic on a 64-bit
    /// host too (unlike a 32-bit-wire-length site, where `pos` is a small
    /// cursor and only `len` is large): `checked_add` must still catch it.
    #[test]
    fn span_rejects_pos_plus_len_overflowing_usize() {
        assert!(span(usize::MAX - 1, 4, usize::MAX).is_err());
    }
}
