//! RNT descriptor namespace — [`AnyRntDescriptor`] + [`parse_rnt_loop`].
//!
//! The Resolution provider Notification Table (ETSI TS 102 323 V1.4.1 §5.2)
//! reuses the SI tag space except for three tags: Table 4 (clause 5.3.3.1)
//! NOTE 2 — "The descriptor tag values 0x40-0x42, inclusive, in the above
//! table, override the definition of the meaning of those descriptor tags in
//! EN 300 468, when they are used in the RNT table only". So RNT tag 0x40
//! is `RAR_over_DVB_stream_descriptor` (not `network_name`), 0x41
//! `RAR_over_IP_descriptor` (not `service_list`) and 0x42 `RNT_scan_descriptor`
//! (not `stuffing`) (issue #1285). Every other tag — including the
//! MPEG-range `metadata_pointer_descriptor` 0x25 Table 4 lists for the RNT, and
//! `default_authority` 0x73 — keeps its SI meaning and is dispatched through
//! [`AnyRntDescriptor::Si`].
//!
//! ```
//! use dvb_si::descriptors::rnt::{parse_rnt_loop, AnyRntDescriptor};
//!
//! // RNT_scan_descriptor (tag 0x42, not SI stuffing): one entry.
//! let raw = [0x42, 0x05, 0x12, 0x34, 0x56, 0x78, 0x09];
//! let item = parse_rnt_loop(&raw).next().unwrap().unwrap();
//! assert!(matches!(item, AnyRntDescriptor::RntScan(_)));
//! ```

pub mod rar_over_dvb_stream;
pub mod rar_over_ip;
pub mod scan;

pub use rar_over_dvb_stream::{RarDownloadSchedule, RarOverDvbStreamDescriptor};
pub use rar_over_ip::RarOverIpDescriptor;
pub use scan::{RntScanDescriptor, RntScanEntry};

use crate::descriptors::scoped::declare_scoped_namespace;

declare_scoped_namespace! {'a;
    doc "Every crate-implemented RNT-specific descriptor (ETSI TS 102 323 Table 4: tags 0x40-0x42, which override their EN 300 468 meaning inside the RNT) plus the SI fall-through for every other tag.";
    enum AnyRntDescriptor, iter RntDescriptorIter, loop RntDescriptorLoop, parse parse_rnt_loop;
    RarOverDvbStream = 0x40 => crate::descriptors::rnt::rar_over_dvb_stream::RarOverDvbStreamDescriptor<'a>,
    RarOverIp = 0x41 => crate::descriptors::rnt::rar_over_ip::RarOverIpDescriptor<'a>,
    RntScan = 0x42 => crate::descriptors::rnt::scan::RntScanDescriptor<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_tags_are_the_table_4_overrides() {
        assert_eq!(AnyRntDescriptor::DISPATCHED_TAGS, &[0x40, 0x41, 0x42]);
    }

    /// The #1285 bug: 0x40/0x41/0x42 are NOT network_name/service_list/
    /// stuffing inside the RNT; every other tag stays SI (incl. 0x25 MPEG
    /// metadata_pointer and 0x73 default_authority).
    #[test]
    fn overrides_win_and_everything_else_is_si() {
        let raw = [
            0x42, 0x00, // RNT_scan (SI: stuffing)
            0x48, 0x03, 0x01, 0x00, 0x00, // SI service_descriptor tag stays SI
            0x73, 0x03, b'a', b'b', b'c', // SI default_authority
        ];
        let items: Vec<_> = parse_rnt_loop(&raw).collect();
        assert!(matches!(
            items[0].as_ref().unwrap(),
            AnyRntDescriptor::RntScan(_)
        ));
        assert!(matches!(
            items[1].as_ref().unwrap(),
            AnyRntDescriptor::Si(_)
        ));
        match items[2].as_ref().unwrap() {
            AnyRntDescriptor::Si(d) => assert_eq!(d.name(), "DEFAULT_AUTHORITY"),
            other => panic!("expected Si default_authority, got {other:?}"),
        }
    }

    #[test]
    fn mpeg_range_tags_are_si_not_unknown_in_rnt() {
        // 0x25 metadata_pointer is in the MPEG range but Table 4 allows it in
        // the RNT with its SI/MPEG meaning: it must reach SI dispatch, never
        // the INT/UNT-style "Unknown".
        // A well-formed metadata_pointer_descriptor body (application_format
        // 0x0010, metadata_format 0x3F, service_id 0, flags: no locator,
        // MPEG_carriage_flags 3 (so no program_number), reserved 11111 -> 0x7F)
        // reaches SI dispatch and is typed.
        let ok = [0x25, 0x05, 0x00, 0x10, 0x3F, 0x00, 0x7F];
        match parse_rnt_loop(&ok).next().unwrap() {
            Ok(AnyRntDescriptor::Si(d)) => assert_eq!(d.name(), "METADATA_POINTER"),
            other => panic!("expected the typed SI path, got {other:?}"),
        }
        // An empty body is still routed to the SI parser (not `Unknown`), which
        // rejects it as a too-short metadata_pointer_descriptor.
        let empty = [0x25, 0x00];
        match parse_rnt_loop(&empty).next().unwrap() {
            Err(crate::Error::InvalidDescriptor { tag: 0x25, reason })
                if reason.contains("metadata_pointer_descriptor too short") => {}
            other => panic!("expected the SI parser's too-short error, got {other:?}"),
        }
    }

    #[test]
    fn loop_wrapper_and_truncation() {
        let raw = [0x42, 0x00];
        let l = RntDescriptorLoop::new(&raw);
        assert_eq!(l.raw(), &raw[..]);
        assert!(matches!(
            l.iter().next().unwrap().unwrap(),
            AnyRntDescriptor::RntScan(_)
        ));
        let mut it = parse_rnt_loop(&[0x40, 0x20, 0x00]);
        assert!(it.next().unwrap().is_err());
        assert!(it.next().is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_surfaces_typed_and_parse_errors() {
        let ok = RntDescriptorLoop::new(&[0x42, 0x00]);
        assert!(serde_json::to_string(&ok).unwrap().contains("rntScan"));
        let bad = RntDescriptorLoop::new(&[0x40, 0xFF]);
        assert!(serde_json::to_string(&bad).unwrap().contains("parseError"));
    }
}
