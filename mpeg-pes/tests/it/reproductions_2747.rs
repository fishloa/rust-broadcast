//! #2747: several serialisers silently masked an out-of-range public field
//! into a different, valid-looking value (ESCR base/extension, `TrickMode`
//! sub-fields, `program_packet_sequence_counter`, `scrambling_control`,
//! `ES_rate`, `additional_copy_info`) and `Pts`/`Dts` carried an unchecked
//! `u64`. Each now rejects the over-range value on construction/serialization.

use mpeg_pes::{Escr, PesHeader, PesPacket, StreamId, TrickMode};

fn serialize_header<'a>(header: PesHeader<'a>) -> Result<(), mpeg_pes::Error> {
    let mut pes = PesPacket::new(StreamId::new(0xE0), &[]);
    pes.header = Some(header);
    let mut out = vec![0u8; pes.serialized_len()];
    pes.serialize_into(&mut out)?;
    Ok(())
}

#[test]
fn escr_base_and_extension_are_not_masked() {
    // A base wider than 33 bits used to be written with `& ESCR_BASE_MASK`.
    let mut h = PesHeader::new();
    h.escr = Some(Escr {
        base: 1 << 33,
        extension: 0,
        reserved: 0b11,
    });
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "ESCR_base",
                ..
            }
        ))
    ));

    // An extension wider than 9 bits used to be written with `& ESCR_EXT_MASK`.
    let mut h = PesHeader::new();
    h.escr = Some(Escr {
        base: 0,
        extension: 1 << 9,
        reserved: 0b11,
    });
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "ESCR_ext",
                ..
            }
        ))
    ));
}

#[test]
fn trick_mode_sub_fields_are_not_masked() {
    // `field_id` is 2 bits and used to be written with `& 0x03`.
    let mut h = PesHeader::new();
    h.dsm_trick_mode = Some(TrickMode::FastForward {
        field_id: 0b100,
        intra_slice_refresh: false,
        frequency_truncation: 0,
    });
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "trick_mode.field_id",
                ..
            }
        ))
    ));

    // `rep_cntrl` is 5 bits and used to be written with `& 0x1F`.
    let mut h = PesHeader::new();
    h.dsm_trick_mode = Some(TrickMode::SlowMotion {
        rep_cntrl: 0b100000,
    });
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "trick_mode.rep_cntrl",
                ..
            }
        ))
    ));
}

#[test]
fn program_packet_sequence_counter_is_not_masked() {
    let mut ext = mpeg_pes::PesExtension::new();
    ext.program_packet_sequence_counter = Some(mpeg_pes::ProgramPacketSequenceCounter {
        counter: 0x80,
        mpeg1_mpeg2_identifier: false,
        original_stuff_length: 0,
    });
    let mut h = PesHeader::new();
    h.pes_extension = Some(ext);
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "program_packet_sequence_counter.counter",
                ..
            }
        ))
    ));
}

#[test]
fn additional_copy_info_is_not_masked() {
    let mut h = PesHeader::new();
    h.additional_copy_info = Some(mpeg_pes::AdditionalCopyInfo::new(0x80));
    assert!(matches!(
        serialize_header(h),
        Err(mpeg_pes::Error::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "additional_copy_info",
                ..
            }
        ))
    ));
}

#[test]
fn pts_and_dts_reject_out_of_range() {
    // A 33-bit timestamp past the wire width used to wrap modulo 2^33.
    assert!(mpeg_pes::Pts::try_new(1 << 33).is_err());
    assert!(mpeg_pes::Dts::try_new(1 << 33).is_err());
    assert!(mpeg_pes::Pts::try_new((1 << 33) - 1).is_ok());
    assert!(mpeg_pes::Dts::try_new((1 << 33) - 1).is_ok());
}

#[test]
fn es_rate_rejects_out_of_range() {
    assert!(mpeg_pes::EsRate::try_new(1 << 22).is_err());
    assert!(mpeg_pes::EsRate::try_new((1 << 22) - 1).is_ok());
}
