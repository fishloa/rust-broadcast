//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the
//! canonical SCTE 35 `splice_insert()` section (CRC-valid).

use broadcast_common::base64::base64_decode_strict;
use broadcast_core::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::error::Error;

const SECTION: &str = "/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=";

#[test]
fn splice_info_section_rejects_every_short_buffer() {
    let bytes = base64_decode_strict(SECTION).unwrap();
    let section = SpliceInfoSection::parse(&bytes).expect("section");
    test_fixtures::assert_short_buffers_rejected(
        section.serialized_len(),
        |buf| section.serialize_into(buf),
        |e| match e {
            Error::OutputBufferTooSmall(broadcast_core::error::OutputBufferTooSmall {
                need,
                have,
            }) => Some((*need, *have)),
            _ => None,
        },
    );
}
