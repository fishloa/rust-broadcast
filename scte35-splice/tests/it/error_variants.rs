use broadcast_common::Parse;
use scte35_splice::{Error, SpliceInfoSection};

#[test]
fn a_section_with_another_table_id_is_an_unexpected_table_id() {
    let mut bytes = [0u8; 32];
    bytes[0] = 0xFD;
    assert_eq!(
        SpliceInfoSection::parse(&bytes).unwrap_err(),
        Error::UnexpectedTableId { table_id: 0xFD }
    );
}
