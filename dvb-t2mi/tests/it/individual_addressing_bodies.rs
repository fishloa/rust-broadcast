//! Typed individual-addressing function bodies — ETSI TS 102 773 §5.2.8
//! (Tables 5–12b) and the transmitter/function loop framing (§5.2.8.1).
//!
//! Byte strings are hand-written from the spec layout with distinct values in
//! every sub-field, so a shifted or masked field cannot round-trip.

use broadcast_common::len::FieldOverflow;
use broadcast_common::{Parse, Serialize};
use dvb_t2mi::Error;
use dvb_t2mi::payload::{
    AddressingFunctionTag, BandwidthBody, CellIdBody, FrequencyBody, FunctionBody, FunctionEntry,
    IndividualAddressingPayload, PrivateDataBody, TransmitterEntry, TrPaprBody,
    TxSigAuxStreamTxIdBody, TxSigFefSeqNumBody,
};

/// Two transmitters: TX-SIG FEF seq nums, TX-SIG aux stream id and TR-PAPR on
/// the first; Frequency, Cell ID and Bandwidth on the second.
fn cell_id_body(cell_id: u16, wait_for_enable_flag: bool, rfu: u8) -> CellIdBody {
    let mut body = CellIdBody::new(cell_id, wait_for_enable_flag);
    body.rfu = rfu;
    body
}

fn bandwidth_body(ch_bandwidth: u8, wait_for_enable_flag: bool) -> BandwidthBody {
    BandwidthBody::new(ch_bandwidth, wait_for_enable_flag)
}

fn spec_payload_bytes() -> Vec<u8> {
    let mut bytes = vec![0x00, 0x29]; // rfu, individual_addressing_length = 41
    // TX 0x1234, function loop 20 bytes.
    bytes.extend_from_slice(&[
        0x12, 0x34, 0x14, //
        0x15, 0x07, 0xAD, 0x53, 0x12, 0x34, 0x56, // TX-SIG FEF seq nums
        0x16, 0x06, 0xAB, 0xC1, 0x23, 0x45, // TX-SIG aux stream TX ID
        0x12, 0x07, 0xAA, 0xBC, 0xAA, 0xA9, 0x55, // TR-PAPR
    ]);
    // TX 0xBEEF, function loop 15 bytes.
    bytes.extend_from_slice(&[
        0xBE, 0xEF, 0x0F, //
        0x17, 0x07, 0xA3, 0x88, 0x15, 0x50, 0x15, // Frequency (474 MHz)
        0x04, 0x05, 0x24, 0x68, 0xAA, // Cell ID
        0x06, 0x03, 0x55, // Bandwidth
    ]);
    bytes
}

fn spec_payload() -> IndividualAddressingPayload<'static> {
    IndividualAddressingPayload {
        rfu: 0x00,
        transmitters: vec![
            TransmitterEntry {
                transmitter_id: 0x1234,
                functions: vec![
                    FunctionEntry {
                        tag: 0x15,
                        body: FunctionBody::TxSigFefSeqNum(TxSigFefSeqNumBody {
                            rfu1: 0b1_0101,
                            seq_num_1: 5,
                            rfu2: 0b0_1010,
                            seq_num_2: 3,
                            rfu3: 0x12_3456,
                        }),
                    },
                    FunctionEntry {
                        tag: 0x16,
                        body: FunctionBody::TxSigAuxStreamTxId(TxSigAuxStreamTxIdBody {
                            tx_sig_aux_tx_id: 0xABC,
                            rfu: 0x1_2345,
                        }),
                    },
                    FunctionEntry {
                        tag: 0x12,
                        body: FunctionBody::TrPapr(TrPaprBody {
                            rfu1: 0x0A,
                            tr_clipping_threshold: 0xABC,
                            rfu2: 0b10_1010_1010_1010,
                            number_of_iterations: 0b01_0101_0101,
                        }),
                    },
                ],
            },
            TransmitterEntry {
                transmitter_id: 0xBEEF,
                functions: vec![
                    FunctionEntry {
                        tag: 0x17,
                        body: FunctionBody::Frequency(FrequencyBody {
                            rf_idx: 5,
                            frequency: 474_000_000,
                            rfu: 0b1_0101,
                        }),
                    },
                    FunctionEntry {
                        tag: 0x04,
                        body: FunctionBody::CellId(cell_id_body(0x2468, true, 0x2A)),
                    },
                    FunctionEntry {
                        tag: 0x06,
                        body: FunctionBody::Bandwidth(bandwidth_body(0x2A, true)),
                    },
                ],
            },
        ],
    }
}

#[test]
fn typed_bodies_serialize_to_the_spec_bytes() {
    let expected = spec_payload_bytes();
    let payload = spec_payload();
    assert_eq!(payload.serialized_len(), expected.len());
    let mut buf = vec![0u8; payload.serialized_len()];
    payload.serialize_into(&mut buf).unwrap();
    assert_eq!(buf, expected);

    let parsed = IndividualAddressingPayload::parse(&expected).unwrap();
    assert_eq!(parsed, payload);
}

#[test]
fn frequency_function_with_the_wrong_body_length_is_opaque() {
    // tag 0x17 with a 3-byte body (function_length 5) is not a Frequency
    // function (5-byte body): it must stay opaque, not be typed.
    let mut bytes = vec![0x00, 0x08];
    bytes.extend_from_slice(&[0x00, 0x01, 0x05, 0x17, 0x05, 0xAA, 0xBB, 0xCC]);
    let parsed = IndividualAddressingPayload::parse(&bytes).unwrap();
    match &parsed.transmitters[0].functions[0].body {
        FunctionBody::Raw(raw) => assert_eq!(*raw, &[0xAA, 0xBB, 0xCC]),
        other => panic!("expected opaque body, got {other:?}"),
    }

    let mut buf = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut buf).unwrap();
    assert_eq!(buf, bytes);
}

#[test]
fn transmitter_header_may_end_exactly_at_the_declared_length() {
    // One transmitter with an empty function loop: its 3-byte header is the
    // whole loop.
    let bytes = [0x00, 0x03, 0x00, 0x01, 0x00];
    let parsed = IndividualAddressingPayload::parse(&bytes).unwrap();
    assert_eq!(parsed.transmitters.len(), 1);
    assert_eq!(parsed.transmitters[0].transmitter_id, 0x0001);
    assert!(parsed.transmitters[0].functions.is_empty());
}

#[test]
fn truncated_transmitter_header_reports_the_exact_need() {
    // 4 declared bytes: a complete 3-byte transmitter header plus one stray
    // byte, which cannot hold a second header.
    let bytes = [0x00, 0x04, 0x00, 0x01, 0x00, 0xAA];
    assert_eq!(
        IndividualAddressingPayload::parse(&bytes).err(),
        Some(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 6,
                have: 4,
                what: "transmitter entry header",
            }
        ))
    );
}

#[test]
fn truncated_function_header_reports_the_exact_need() {
    // Function loop of 1 byte: not enough for tag + length.
    let bytes = [0x00, 0x04, 0x00, 0x01, 0x01, 0x15];
    assert_eq!(
        IndividualAddressingPayload::parse(&bytes).err(),
        Some(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 5,
                have: 4,
                what: "function entry header",
            }
        ))
    );
}

fn private_data_payload(body_len: usize) -> (IndividualAddressingPayload<'static>, Vec<u8>) {
    static PRIVATE: [u8; 251] = [0x5A; 251];
    let payload = IndividualAddressingPayload {
        rfu: 0x00,
        transmitters: vec![TransmitterEntry {
            transmitter_id: 0x0001,
            functions: vec![FunctionEntry {
                tag: 0x03,
                body: FunctionBody::PrivateData(PrivateDataBody::new(&PRIVATE[..body_len])),
            }],
        }],
    };
    (payload, vec![0x5A; body_len])
}

#[test]
fn individual_addressing_length_maximum_is_accepted() {
    // 2 (header) + 3 (transmitter) + 2 (function header) + 250 = 257 bytes
    // total; the loop length is 255, the largest a byte can carry.
    let (payload, _) = private_data_payload(250);
    assert_eq!(payload.serialized_len(), 257);
    let mut buf = vec![0u8; payload.serialized_len()];
    payload.serialize_into(&mut buf).unwrap();
    assert_eq!(buf[1], 255);
    assert_eq!(
        IndividualAddressingPayload::parse(&buf).unwrap(),
        payload
    );
}

#[test]
fn individual_addressing_length_above_a_byte_is_rejected() {
    let (payload, _) = private_data_payload(251);
    let mut buf = vec![0u8; payload.serialized_len()];
    assert_eq!(
        payload.serialize_into(&mut buf).err(),
        Some(Error::FieldOverflow(FieldOverflow {
            field: "individual_addressing_length",
            value: 256,
            max: 255,
        }))
    );
}

#[test]
fn function_body_names_follow_the_spec_tables() {
    let tags = [
        0x00u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x10, 0x11, 0x12, 0x13, 0x15, 0x16, 0x17,
    ];
    for tag in tags {
        assert_eq!(
            u8::from(AddressingFunctionTag::try_from(tag).unwrap()),
            tag,
            "tag {tag:#04x} encodes to its wire value"
        );
    }

    let payload = spec_payload();
    let names: Vec<&str> = payload
        .transmitters
        .iter()
        .flat_map(|tx| tx.functions.iter())
        .map(|f| f.body.name())
        .collect();
    assert_eq!(
        names,
        [
            "TX-SIG FEF sequence numbers",
            "TX-SIG auxiliary stream TX ID",
            "TR-PAPR",
            "Frequency",
            "Cell ID",
            "Bandwidth",
        ]
    );

    let raw = IndividualAddressingPayload::parse(&[0x00, 0x06, 0x00, 0x01, 0x03, 0x40, 0x03, 0xAA])
        .unwrap();
    assert_eq!(raw.transmitters[0].functions[0].body.name(), "raw");
}

#[test]
fn display_names_the_payload() {
    let payload = spec_payload();
    let text = payload.to_string();
    assert!(text.contains("IndividualAddressing"), "{text}");
    assert!(text.contains("tx_count: 2"), "{text}");
}
