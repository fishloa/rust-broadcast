//! The data-field family through `broadcast_common::{Parse, Serialize}` on the real
//! EN 301 775 data-field fixture (one of every typed data unit).

use broadcast_common::{Parse, Serialize};
use dvb_vbi::{
    ClosedCaptioningDataField, DataField, DataUnit, DataUnitPayload, MonochromeDataField,
    TeletextDataField, VpsDataField, WssDataField,
};

fn fixture() -> Vec<u8> {
    test_fixtures::read("subtitles/vbi-data-field.bin")
}

fn round_trip<'a, T>(value: &T, wire: &'a mut Vec<u8>)
where
    T: Parse<'a, Error = dvb_vbi::Error>
        + Serialize<Error = dvb_vbi::Error>
        + PartialEq
        + core::fmt::Debug,
{
    *wire = Serialize::try_to_bytes(value).unwrap();
    let parsed = T::parse(wire).unwrap();
    assert_eq!(&parsed, value);
    assert_eq!(Serialize::try_to_bytes(&parsed).unwrap(), *wire);
}

#[test]
fn data_field_round_trips_byte_exact_through_the_traits() {
    let data = fixture();
    let field = <DataField as Parse>::parse(&data).unwrap();
    assert_eq!(Serialize::try_to_bytes(&field).unwrap(), data);
    let mut wire = Vec::new();
    round_trip(&field, &mut wire);
}

#[test]
fn every_payload_field_type_round_trips_through_the_traits() {
    let data = fixture();
    let field = <DataField as Parse>::parse(&data).unwrap();
    let mut seen = 0;
    for unit in &field.data_units {
        let unit_wire = Serialize::try_to_bytes(unit).unwrap();
        assert_eq!(unit_wire.len(), Serialize::serialized_len(unit));
        let mut wire = Vec::new();
        match &unit.payload {
            DataUnitPayload::Teletext(f) => {
                round_trip::<TeletextDataField>(f, &mut wire);
                seen += 1;
            }
            DataUnitPayload::Vps(f) => {
                round_trip::<VpsDataField>(f, &mut wire);
                seen += 1;
            }
            DataUnitPayload::Wss(f) => {
                round_trip::<WssDataField>(f, &mut wire);
                seen += 1;
            }
            DataUnitPayload::ClosedCaptioning(f) => {
                round_trip::<ClosedCaptioningDataField>(f, &mut wire);
                seen += 1;
            }
            DataUnitPayload::Monochrome(f) => {
                round_trip::<MonochromeDataField>(f, &mut wire);
                seen += 1;
            }
            _ => {}
        }
    }
    assert_eq!(seen, 5, "teletext, VPS, WSS, CC and monochrome units");
    let _: fn(&DataUnit) -> usize = |u| Serialize::serialized_len(u);
}
