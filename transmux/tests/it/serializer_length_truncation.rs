//! Serializer length/count/value truncation guard for `transmux` (#1129).
//!
//! Version-0 FullBoxes carry 32-bit `creation_time` / `modification_time` /
//! `duration` / `media_time`. A 64-bit value above `u32::MAX` used to be
//! written as `value as u32` (silently wrapped); it must be a `FieldOverflow`
//! error, while `u32::MAX` itself and every version-1 value still serialize.
//! The lexical half (length narrowing) is the `truncation` guard in
//! workspace-guards.

use broadcast_common::Serialize;
use broadcast_common::timescale::Timescale;
use transmux::{MediaHeaderBox, MovieHeaderBox, ProducerReferenceTimeBox, TrackHeaderBox, TrackId};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

const OVER_U32: u64 = 0x1_0000_0000;

fn serialize<T: Serialize>(t: &T) -> Result<Vec<u8>, T::Error> {
    let mut buf = vec![0u8; t.serialized_len()];
    t.serialize_into(&mut buf)?;
    Ok(buf)
}

fn mvhd(version: u8, ct: u64, mt: u64, dur: u64) -> MovieHeaderBox {
    MovieHeaderBox {
        version,
        flags: 0,
        creation_time: ct,
        modification_time: mt,
        timescale: timescale(1000),
        duration: dur,
        rate: 0x0001_0000,
        volume: 0x0100,
        matrix: [0; 9],
        next_track_id: TrackId::new(2),
    }
}

fn tkhd(version: u8, ct: u64, mt: u64, dur: u64) -> TrackHeaderBox {
    TrackHeaderBox {
        version,
        flags: 3,
        creation_time: ct,
        modification_time: mt,
        track_id: TrackId::new(1),
        duration: dur,
        layer: 0,
        alternate_group: 0,
        volume: 0,
        matrix: [0; 9],
        width: 0,
        height: 0,
    }
}

fn mdhd(version: u8, ct: u64, mt: u64, dur: u64) -> MediaHeaderBox {
    MediaHeaderBox {
        version,
        flags: 0,
        creation_time: ct,
        modification_time: mt,
        timescale: timescale(90_000),
        duration: dur,
        language: 0x55C4,
    }
}

fn prft(version: u8, media_time: u64) -> ProducerReferenceTimeBox {
    ProducerReferenceTimeBox {
        version,
        flags: 0,
        reference_track_id: TrackId::new(1),
        ntp_timestamp: 1,
        media_time,
    }
}

#[test]
fn v0_header_boxes_reject_values_above_u32() {
    // (creation, modification, duration): one over-range field at a time.
    let cases = [
        (OVER_U32, 0, 0),
        (0, OVER_U32, 0),
        (0, 0, OVER_U32),
        (u64::MAX, 0, 0),
    ];
    for (ct, mt, dur) in cases {
        assert!(
            matches!(
                serialize(&mvhd(0, ct, mt, dur)),
                Err(transmux::error::Error::FieldOverflow(..))
            ),
            "mvhd {ct} {mt} {dur}"
        );
        assert!(
            matches!(
                serialize(&tkhd(0, ct, mt, dur)),
                Err(transmux::error::Error::FieldOverflow(..))
            ),
            "tkhd {ct} {mt} {dur}"
        );
        assert!(
            matches!(
                serialize(&mdhd(0, ct, mt, dur)),
                Err(transmux::error::Error::FieldOverflow(..))
            ),
            "mdhd {ct} {mt} {dur}"
        );
        // Version 1 holds the same values in 64 bits.
        assert!(serialize(&mvhd(1, ct, mt, dur)).is_ok());
        assert!(serialize(&tkhd(1, ct, mt, dur)).is_ok());
        assert!(serialize(&mdhd(1, ct, mt, dur)).is_ok());
    }
    assert!(matches!(
        serialize(&prft(0, OVER_U32)),
        Err(transmux::error::Error::FieldOverflow(..))
    ));
    assert!(serialize(&prft(1, OVER_U32)).is_ok());
}

#[test]
fn v0_header_boxes_accept_u32_max() {
    let m = u64::from(u32::MAX);
    let v = serialize(&mvhd(0, m, m, m)).expect("mvhd v0 u32::MAX");
    assert_eq!(&v[12..16], &u32::MAX.to_be_bytes());
    assert!(serialize(&tkhd(0, m, m, m)).is_ok());
    assert!(serialize(&mdhd(0, m, m, m)).is_ok());
    assert!(serialize(&prft(0, m)).is_ok());
}
