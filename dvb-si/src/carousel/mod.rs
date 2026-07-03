//! DSM-CC data-carousel download protocol — ISO/IEC 13818-6 §7.2/§7.3 as
//! profiled by DVB (TR 101 202 §4.6/§4.7.5, TS 102 006 SSU, TS 102 809).
//!
//! Layer cake: [`crate::tables::dsmcc::DsmccSection`] frames the sections
//! (table_id 0x3B control / 0x3C data); this module types their payloads —
//! [`UnMessage`] (DSI/DII) and [`DownloadDataBlock`] — and
//! [`ModuleReassembler`] collects DDB blocks into complete modules.
//!
//! Wire layouts are documented in `docs/iso_13818_6_carousel.md` (with
//! provenance notes — ISO/IEC 13818-6 itself cannot be vendored) and pinned
//! against a live capture by the `carousel_fixture` integration test.

pub mod biop;
pub mod messages;
pub mod reassembler;

pub use biop::{
    Binding, BiopMessage, BiopProfileBody, CarouselFs, CarouselObject, CompressedModuleDescriptor,
    ConnBinder, DirectoryMessage, FileMessage, Ior, LiteComponent, LiteOptionsProfileBody,
    ModuleInfo, NameComponent, NsapAddress, ObjectKind, ObjectLocation, ServiceGatewayInfo,
    ServiceLocation, TaggedProfile, Tap,
};
pub use messages::{
    Dii, DiiModule, DownloadDataBlock, Dsi, GroupInfo, GroupInfoIndication, UnMessage,
};
pub use reassembler::{DEFAULT_MAX_MODULE_SIZE, Module, ModuleKey, ModuleReassembler};
