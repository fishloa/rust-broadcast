//! Splice descriptors — ANSI/SCTE 35 2023r1 §10, Tables 16-28.
//!
//! One module per `splice_descriptor_tag` (§10.1, Table 16), each mirroring the
//! dvb-si descriptor shape: spec-cited doc → `TAG` const → struct → symmetric
//! [`Parse`](dvb_common::Parse)/[`Serialize`](dvb_common::Serialize) →
//! in-module round-trip tests. All splice descriptors share the 6-byte
//! `splice_descriptor()` header (tag + length + 4-byte `identifier`, §10.2)
//! handled by [`header`]. [`AnySpliceDescriptor`] unifies them with a raw
//! fall-through for unknown tags.

pub mod any;
pub mod audio;
pub mod avail;
pub mod dtmf;
pub mod header;
pub mod segmentation;
pub mod segmentation_enums;
pub mod time;

pub use any::{parse_loop, AnySpliceDescriptor, SpliceDescriptorIter};
pub use audio::{AudioComponent, AudioDescriptor};
pub use avail::AvailDescriptor;
pub use dtmf::DtmfDescriptor;
pub use header::CUEI;
pub use segmentation::{
    DeliveryRestrictions, MidUpid, Mpu, SegmentationComponent, SegmentationDescriptor,
};
pub use segmentation_enums::{DeviceRestrictions, SegmentationTypeId, SegmentationUpidType};
pub use time::TimeDescriptor;
