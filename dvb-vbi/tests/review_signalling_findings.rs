//! Adversarial-review findings (review/signalling-captions): FAILING
//! regressions for confirmed defects; production code is untouched.

use dvb_vbi::{DataUnit, DataUnitId, DataUnitPayload};

/// #1106 added `DataUnit::stuffing`, documented "always 0 for `Stuffing` /
/// `Opaque` payloads", but nothing enforces it: serialize appends the 0xFF
/// bytes after the payload, and parse folds them back into the payload with
/// `stuffing: 0`. serialize -> parse is therefore not equal (the struct-level
/// half of the round-trip invariant); serialize should reject the pair.
#[test]
fn stuffing_count_on_opaque_or_stuffing_payload_round_trips_or_is_rejected() {
    let body = [0xDEu8, 0xAD];
    let units = [
        DataUnit {
            id: DataUnitId::Reserved(0x55),
            payload: DataUnitPayload::Opaque(&body),
            stuffing: 3,
        },
        DataUnit {
            id: DataUnitId::Stuffing,
            payload: DataUnitPayload::Stuffing { length: 2 },
            stuffing: 3,
        },
    ];
    for unit in units {
        let mut out = vec![0u8; unit.serialized_len()];
        if unit.serialize_into(&mut out).is_err() {
            continue;
        }
        let (back, _) = DataUnit::parse(&out).unwrap();
        assert_eq!(back, unit, "wire {out:02X?}");
    }
}
