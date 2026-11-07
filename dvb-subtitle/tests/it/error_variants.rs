use broadcast_common::Parse;
use dvb_subtitle::{DisplayDefinitionSegment, Error};

#[test]
fn a_segment_of_another_type_is_an_unknown_segment_type() {
    let bytes = [0x0F, 0x13, 0x00, 0x01, 0x00, 0x05, 0, 0, 0, 0, 0];
    assert_eq!(
        DisplayDefinitionSegment::parse(&bytes).unwrap_err(),
        Error::UnknownSegmentType(0x13)
    );
}
