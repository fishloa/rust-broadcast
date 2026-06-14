//! BIOP (Broadcast Inter-ORB Protocol) object-carousel layer.
//!
//! Implements the DVB-profiled subset of ISO/IEC 13818-6 §11 as documented
//! in `docs/iso_13818_6_biop.md` (transcribed from ETSI TR 101 202 §4.7).
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

pub mod fs;
pub mod ior;
pub mod message;

pub use fs::{CarouselFs, CarouselObject};
pub use ior::{
    BiopProfileBody, ConnBinder, Ior, LiteComponent, LiteOptionsProfileBody, NameComponent,
    NsapAddress, ObjectKind, ObjectLocation, ServiceLocation, TaggedProfile, Tap,
};
pub use message::{
    Binding, BiopMessage, CompressedModuleDescriptor, DirectoryMessage, FileMessage, ModuleInfo,
    ServiceGatewayInfo,
};
