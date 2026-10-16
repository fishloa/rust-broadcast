//! UNT descriptor namespace — [`AnyUntDescriptor`] + [`parse_unt_loop`].
//!
//! The Update Notification Table (ETSI TS 102 006 §9.4) has its OWN descriptor
//! tag space for `0x00..=0x3F` (Table 19, clause 9.5.1): "MPEG descriptors in
//! the range 0x00 to 0x3F cannot be used in the UNT". So UNT tag 0x02 is
//! `update_descriptor`, never MPEG `video_stream_descriptor`; 0x03 is
//! `SSU_location_descriptor` (not `audio_stream`), 0x04 `message_descriptor`
//! (not `hierarchy`), … (issue #1285). The tags 0x06..=0x0A are the
//! target descriptors shared with the INT (one Rust type each, see
//! [`crate::descriptors::target`]). Tags `0x40..=0xFF` are "in the scope of
//! DVB-SI" (telephone 0x57, private_data_specifier 0x5F, user private) and are
//! dispatched through [`AnyUntDescriptor::Si`].
//!
//! ```
//! use dvb_si::descriptors::unt::{parse_unt_loop, AnyUntDescriptor};
//!
//! // update_descriptor (UNT tag 0x02: automatic, immediate, priority 3).
//! let raw = [0x02, 0x01, 0x43];
//! let item = parse_unt_loop(&raw).next().unwrap().unwrap();
//! assert!(matches!(item, AnyUntDescriptor::Update(_)));
//! ```

pub mod enhanced_message;
pub mod message;
pub mod scheduling;
pub mod ssu_event_name;
pub mod ssu_location;
pub mod ssu_uri;
pub mod subgroup_association;
pub mod update;

pub use enhanced_message::EnhancedMessageDescriptor;
pub use message::MessageDescriptor;
pub use scheduling::{SchedulingDescriptor, TimeUnit};
pub use ssu_event_name::SsuEventNameDescriptor;
pub use ssu_location::SsuLocationDescriptor;
pub use ssu_uri::SsuUriDescriptor;
pub use subgroup_association::SsuSubgroupAssociationDescriptor;
pub use update::{UpdateDescriptor, UpdateFlag, UpdateMethod};

use crate::descriptors::scoped::declare_scoped_namespace;

declare_scoped_namespace! {'a;
    doc "Every crate-implemented UNT descriptor (ETSI TS 102 006 Table 19), the SI fall-through for tags `0x40..=0xFF`, and `Unknown` for an unimplemented tag in the UNT-owned `0x00..=0x3F` range.";
    enum AnyUntDescriptor, iter UntDescriptorIter, loop UntDescriptorLoop, parse parse_unt_loop;
    @owned 0x00..=0x3F,
    Scheduling = 0x01 => crate::descriptors::unt::scheduling::SchedulingDescriptor<'a>,
    Update = 0x02 => crate::descriptors::unt::update::UpdateDescriptor<'a>,
    SsuLocation = 0x03 => crate::descriptors::unt::ssu_location::SsuLocationDescriptor<'a>,
    Message = 0x04 => crate::descriptors::unt::message::MessageDescriptor<'a>,
    SsuEventName = 0x05 => crate::descriptors::unt::ssu_event_name::SsuEventNameDescriptor<'a>,
    TargetSmartcard = 0x06 => crate::descriptors::target::smartcard::TargetSmartcardDescriptor<'a>,
    TargetMacAddress = 0x07 => crate::descriptors::target::mac_address::TargetMacAddressDescriptor<'a>,
    TargetSerialNumber = 0x08 => crate::descriptors::target::serial_number::TargetSerialNumberDescriptor<'a>,
    TargetIpAddress = 0x09 => crate::descriptors::target::ip_address::TargetIpAddressDescriptor<'a>,
    TargetIpv6Address = 0x0A => crate::descriptors::target::ipv6_address::TargetIpv6AddressDescriptor<'a>,
    SsuSubgroupAssociation = 0x0B => crate::descriptors::unt::subgroup_association::SsuSubgroupAssociationDescriptor<'a>,
    EnhancedMessage = 0x0C => crate::descriptors::unt::enhanced_message::EnhancedMessageDescriptor<'a>,
    SsuUri = 0x0D => crate::descriptors::unt::ssu_uri::SsuUriDescriptor<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_tags_are_the_table_19_set() {
        assert_eq!(
            AnyUntDescriptor::DISPATCHED_TAGS,
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D]
        );
    }

    /// The #1285 bug: SI/MPEG 0x02 (video_stream), 0x03 (audio_stream), 0x04
    /// (hierarchy) must decode as UNT descriptors; an MPEG tag with no UNT
    /// meaning is `Unknown`, never SI.
    #[test]
    fn unt_tags_do_not_decode_as_mpeg() {
        let raw = [
            0x02, 0x01, 0x01, // update_descriptor (MPEG: video_stream)
            0x03, 0x02, 0x00, 0x05, // SSU_location (MPEG: audio_stream)
            0x04, 0x04, 0x00, b'e', b'n', b'g', // message_descriptor (MPEG: hierarchy)
            0x0E, 0x00, // UNT-unassigned MPEG-range tag
        ];
        let items: Vec<_> = parse_unt_loop(&raw).collect();
        assert!(matches!(items[0].as_ref().unwrap(), AnyUntDescriptor::Update(_)));
        assert!(matches!(items[1].as_ref().unwrap(), AnyUntDescriptor::SsuLocation(_)));
        assert!(matches!(items[2].as_ref().unwrap(), AnyUntDescriptor::Message(_)));
        assert!(matches!(
            items[3].as_ref().unwrap(),
            AnyUntDescriptor::Unknown { tag: 0x0E, .. }
        ));
    }

    #[test]
    fn si_range_falls_back_to_si_dispatch() {
        let raw = [0x5F, 0x04, 0x00, 0x00, 0x00, 0x28];
        match parse_unt_loop(&raw).next().unwrap().unwrap() {
            AnyUntDescriptor::Si(d) => assert_eq!(d.name(), "PRIVATE_DATA_SPECIFIER"),
            other => panic!("expected Si, got {other:?}"),
        }
    }

    #[test]
    fn loop_wrapper_and_truncation() {
        let raw = [0x0D, 0x02, 0x00, 0x00];
        let l = UntDescriptorLoop::new(&raw);
        assert_eq!(l.raw(), &raw[..]);
        assert!(matches!(l.iter().next().unwrap().unwrap(), AnyUntDescriptor::SsuUri(_)));
        let mut it = parse_unt_loop(&[0x01, 0x20, 0x00]);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_surfaces_typed_and_parse_errors() {
        let ok = UntDescriptorLoop::new(&[0x0D, 0x02, 0x00, 0x00]);
        assert!(serde_json::to_string(&ok).unwrap().contains("ssuUri"));
        let bad = UntDescriptorLoop::new(&[0x01, 0xFF]);
        assert!(serde_json::to_string(&bad).unwrap().contains("parseError"));
    }
}
