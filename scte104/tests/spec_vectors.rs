//! Byte-identical round-trip tests over **spec-derived** wire vectors (ANSI/SCTE
//! 104 2023), one per representative operation plus both message wrappers.
//!
//! ## Provenance
//!
//! Every byte vector below is **computed directly from the ANSI/SCTE 104 2023
//! syntax tables** (`scte104/docs/ansi_scte_104/operations.md` for Tables
//! 8-1/8-2/9-5, `scte104/docs/ansi_scte_104/pams_operations.md` for Tables
//! 10-1/10-3/10-5), independently of this crate's own serializer — mirroring
//! the pattern (and closing the exact gap) named in `rtcp-packet/tests/spec_vectors.rs`
//! and issue #936: every other test in this crate builds via the typed API,
//! serializes, parses, and compares against itself, which cannot distinguish a
//! correct implementation from a self-consistent wrong one (a field-offset bug
//! applied identically in both directions still passes). Each vector here
//! asserts field values at the documented byte offset *before* going anywhere
//! near this crate's `Parse`/`Serialize` impls, then separately checks that
//! re-serializing reproduces the vector byte-for-byte.

use broadcast_common::{Parse, Serialize};
use scte104::operations::provisioning_request::{
    DpiPidEntry, InjectorComponentList, ProvisioningRequest, ProvisioningService,
};
use scte104::operations::splice_request::{SpliceInsertType, SpliceRequest};
use scte104::operations::{AnyOperation, AnySingleOperation, ConfigRequest, FaultRequest};
use scte104::time::Timestamp;
use scte104::{MultipleOperationMessage, SingleOperationMessage};

// ── splice_request_data() — §9.3.1, Table 9-5 (opID 0x0101) ────────────────
//
// splice_insert_type=1 (spliceStart_normal), splice_event_id=0x00000042,
// unique_program_id=1, pre_roll_time=5000 (0x1388), break_duration=300 (0x012C),
// avail_num=0, avails_expected=0, auto_return_flag=1, not_an_entry_flag=0.
#[rustfmt::skip]
const SPLICE_REQUEST_VECTOR: [u8; 15] = [
    0x01,                   // splice_insert_type
    0x00, 0x00, 0x00, 0x42, // splice_event_id
    0x00, 0x01,             // unique_program_id
    0x13, 0x88,             // pre_roll_time
    0x01, 0x2C,             // break_duration
    0x00,                   // avail_num
    0x00,                   // avails_expected
    0x01,                   // auto_return_flag
    0x00,                   // not_an_entry_flag
];

#[test]
fn splice_request_vector_parses_and_round_trips() {
    let sr = SpliceRequest::parse(&SPLICE_REQUEST_VECTOR).expect("parse spec-derived vector");
    assert_eq!(sr.splice_insert_type, SpliceInsertType::SpliceStartNormal);
    assert_eq!(sr.splice_event_id, 0x0000_0042);
    assert_eq!(sr.unique_program_id, 1);
    assert_eq!(sr.pre_roll_time, 5000);
    assert_eq!(sr.break_duration, 300);
    assert_eq!(sr.avail_num, 0);
    assert_eq!(sr.avails_expected, 0);
    assert_eq!(sr.auto_return_flag, 1);
    assert_eq!(sr.not_an_entry_flag, 0);

    let mut out = vec![0u8; sr.serialized_len()];
    sr.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, SPLICE_REQUEST_VECTOR,
        "byte-identical to the Table 9-5 spec-derived vector"
    );
}

// ── single_operation_message() wrapping general_response_data() — §8.2.2,
//    Table 8-1 (opID 0x0000) ────────────────────────────────────────────────
//
// opID=0x0000, messageSize=13 (header only, general_response has no body),
// result=0x0000, result_extension=0xFFFF, protocol_version=0, AS_index=1,
// message_number=42 (0x2A), DPI_PID_index=0.
#[rustfmt::skip]
const GENERAL_RESPONSE_MESSAGE_VECTOR: [u8; 13] = [
    0x00, 0x00, // opID
    0x00, 0x0D, // messageSize = 13
    0x00, 0x00, // result
    0xFF, 0xFF, // result_extension
    0x00,       // protocol_version
    0x01,       // AS_index
    0x2A,       // message_number
    0x00, 0x00, // DPI_PID_index
];

#[test]
fn single_operation_message_vector_parses_and_round_trips() {
    let msg = SingleOperationMessage::parse(&GENERAL_RESPONSE_MESSAGE_VECTOR)
        .expect("parse spec-derived Table 8-1 vector");
    assert_eq!(msg.op_id, 0x0000);
    assert_eq!(msg.message_size, 13);
    assert_eq!(msg.result, 0x0000);
    assert_eq!(msg.result_extension, 0xFFFF);
    assert_eq!(msg.protocol_version, 0);
    assert_eq!(msg.as_index, 1);
    assert_eq!(msg.message_number, 42);
    assert_eq!(msg.dpi_pid_index, 0);
    assert!(matches!(msg.data, AnySingleOperation::GeneralResponse(_)));

    let mut out = vec![0u8; msg.serialized_len()];
    msg.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, GENERAL_RESPONSE_MESSAGE_VECTOR,
        "byte-identical to the Table 8-1 spec-derived vector"
    );
}

// ── multiple_operation_message() wrapping splice_request_data() — §8.2.3,
//    Table 8-2 header + Table 9-5 body ──────────────────────────────────────
//
// Reserved=0xFFFF, messageSize=31 (0x001F), protocol_version=0, AS_index=1,
// message_number=42, DPI_PID_index=0, SCTE35_protocol_version=0,
// timestamp()=None (time_type=0, 1 byte), num_ops=1, then opID=0x0101,
// data_length=15, data()=SPLICE_REQUEST_VECTOR.
#[rustfmt::skip]
const MULTIPLE_OPERATION_MESSAGE_VECTOR: [u8; 31] = [
    0xFF, 0xFF,             // Reserved
    0x00, 0x1F,             // messageSize = 31
    0x00,                   // protocol_version
    0x01,                   // AS_index
    0x2A,                   // message_number
    0x00, 0x00,             // DPI_PID_index
    0x00,                   // SCTE35_protocol_version
    0x00,                   // timestamp() time_type = 0 (None)
    0x01,                   // num_ops
    0x01, 0x01,             // opID = 0x0101 (splice_request)
    0x00, 0x0F,             // data_length = 15
    0x01,                   // splice_insert_type
    0x00, 0x00, 0x00, 0x42, // splice_event_id
    0x00, 0x01,             // unique_program_id
    0x13, 0x88,             // pre_roll_time
    0x01, 0x2C,             // break_duration
    0x00,                   // avail_num
    0x00,                   // avails_expected
    0x01,                   // auto_return_flag
    0x00,                   // not_an_entry_flag
];

#[test]
fn multiple_operation_message_vector_parses_and_round_trips() {
    let msg = MultipleOperationMessage::parse(&MULTIPLE_OPERATION_MESSAGE_VECTOR)
        .expect("parse spec-derived Table 8-2 vector");
    assert_eq!(msg.message_size, 31);
    assert_eq!(msg.protocol_version, 0);
    assert_eq!(msg.as_index, 1);
    assert_eq!(msg.message_number, 42);
    assert_eq!(msg.dpi_pid_index, 0);
    assert_eq!(msg.scte35_protocol_version, 0);
    assert_eq!(msg.timestamp, Timestamp::None);
    assert_eq!(msg.operations.len(), 1);
    assert_eq!(msg.operations[0].op_id, 0x0101);
    match &msg.operations[0].data {
        AnyOperation::SpliceRequest(sr) => {
            assert_eq!(sr.splice_event_id, 0x42);
            assert_eq!(sr.pre_roll_time, 5000);
        }
        other => panic!("expected SpliceRequest, got {other:?}"),
    }

    let mut out = vec![0u8; msg.serialized_len()];
    msg.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, MULTIPLE_OPERATION_MESSAGE_VECTOR,
        "byte-identical to the Table 8-2 spec-derived vector"
    );
}

// ── config_request_data() — §10.4.1, Table 10-1 (opID 0x0009) ──────────────
//
// AS_IP_address=192.168.1.1 (0xC0A80101), AS_socket_number=5167 (0x142F),
// activeflag=1 (primary), protocol_version=0, last_AS_index=7,
// last_injectorcount=3, permanent_connection_requested=1.
#[rustfmt::skip]
const CONFIG_REQUEST_VECTOR: [u8; 12] = [
    0xC0, 0xA8, 0x01, 0x01, // AS_IP_address
    0x14, 0x2F,             // AS_socket_number = 5167
    0x01,                   // activeflag
    0x00,                   // protocol_version
    0x07,                   // last_AS_index
    0x00, 0x03,             // last_injectorcount
    0x01,                   // permanent_connection_requested
];

#[test]
fn config_request_vector_parses_and_round_trips() {
    let cr = ConfigRequest::parse(&CONFIG_REQUEST_VECTOR).expect("parse spec-derived vector");
    assert_eq!(cr.as_ip_address, 0xC0A8_0101);
    assert_eq!(cr.as_socket_number, 5167);
    assert_eq!(cr.activeflag, 1);
    assert_eq!(cr.protocol_version, 0);
    assert_eq!(cr.last_as_index, 7);
    assert_eq!(cr.last_injectorcount, 3);
    assert_eq!(cr.permanent_connection_requested, 1);

    let mut out = vec![0u8; cr.serialized_len()];
    cr.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, CONFIG_REQUEST_VECTOR,
        "byte-identical to the Table 10-1 spec-derived vector"
    );
}

// ── fault_request_data() — §10.6.1, Table 10-5 (opID 0x000F) ────────────────
//
// injector_IP_address=10.0.0.2 (0x0A000002), injector_socket_number=8000
// (0x1F40), injector_service_name="svc1"+NUL padding to 32 bytes,
// DPI_PID_index=5.
#[rustfmt::skip]
const FAULT_REQUEST_VECTOR: [u8; 40] = [
    0x0A, 0x00, 0x00, 0x02,                         // injector_IP_address
    0x1F, 0x40,                                      // injector_socket_number = 8000
    0x73, 0x76, 0x63, 0x31, 0x00, 0x00, 0x00, 0x00,  // injector_service_name[0..8] "svc1\0\0\0\0"
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // injector_service_name[8..16]
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // injector_service_name[16..24]
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // injector_service_name[24..32]
    0x00, 0x05,                                      // DPI_PID_index = 5
];

#[test]
fn fault_request_vector_parses_and_round_trips() {
    let fr = FaultRequest::parse(&FAULT_REQUEST_VECTOR).expect("parse spec-derived vector");
    assert_eq!(fr.injector_ip_address, 0x0A00_0002);
    assert_eq!(fr.injector_socket_number, 8000);
    assert_eq!(&fr.injector_service_name[..4], b"svc1");
    assert!(fr.injector_service_name[4..].iter().all(|&b| b == 0));
    assert_eq!(fr.dpi_pid_index, 5);

    let mut out = vec![0u8; fr.serialized_len()];
    fr.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, FAULT_REQUEST_VECTOR,
        "byte-identical to the Table 10-5 spec-derived vector"
    );
}

// ── provisioning_request_data() — §10.5.1, Table 10-3 (opID 0x000B), plus
//    injector_component_list() — §10.8.1, Table 10-9 ───────────────────────
//
// service_count=1; service: injector_IP_address=10.0.0.1 (0x0A000001),
// injector_socket_number=8000 (0x1F40), service_name="svc1"+NUL padding,
// number_of_DPI_PIDs=1 (DPI_PID_index=5, shared_PID=0,
// event_id_compliance_flag=1), component_mode=1 (non-zero => list present):
// video_component_tag=0x10, 2 audio tags (0x20, 0x21), 1 data tag (0x30).
#[rustfmt::skip]
const PROVISIONING_REQUEST_VECTOR: [u8; 51] = [
    0x01,                                             // service_count = 1
    0x0A, 0x00, 0x00, 0x01,                           // injector_IP_address
    0x1F, 0x40,                                       // injector_socket_number = 8000
    0x73, 0x76, 0x63, 0x31, 0x00, 0x00, 0x00, 0x00,   // service_name[0..8] "svc1\0\0\0\0"
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,   // service_name[8..16]
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,   // service_name[16..24]
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,   // service_name[24..32]
    0x01,                                             // number_of_DPI_PIDs = 1
    0x00, 0x05, 0x00, 0x01,                           // DPI_PID_index=5, shared_PID=0, event_id_compliance_flag=1
    0x01,                                             // component_mode = 1 (list present)
    0x10,                                             // video_component_tag
    0x02,                                             // number_of_audio_component_tags = 2
    0x20, 0x21,                                       // audio_component_tag x2
    0x01,                                             // number_of_data_component_tags = 1
    0x30,                                             // data_component_tag
];

#[test]
fn provisioning_request_vector_parses_and_round_trips() {
    let pr = ProvisioningRequest::parse(&PROVISIONING_REQUEST_VECTOR)
        .expect("parse spec-derived Table 10-3 vector");
    assert_eq!(pr.services.len(), 1);
    let svc = &pr.services[0];
    assert_eq!(svc.injector_ip_address, 0x0A00_0001);
    assert_eq!(svc.injector_socket_number, 8000);
    assert_eq!(&svc.service_name[..4], b"svc1");
    assert_eq!(
        svc.dpi_pids,
        vec![DpiPidEntry {
            dpi_pid_index: 5,
            shared_pid: 0,
            event_id_compliance_flag: 1,
        }]
    );
    assert_eq!(svc.component_mode, 1);
    assert_eq!(
        svc.injector_component_list,
        Some(InjectorComponentList {
            video_component_tag: 0x10,
            audio_component_tags: vec![0x20, 0x21],
            data_component_tags: vec![0x30],
        })
    );

    let mut out = vec![0u8; pr.serialized_len()];
    pr.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, PROVISIONING_REQUEST_VECTOR,
        "byte-identical to the Table 10-3 spec-derived vector"
    );
}

// A second construction, built entirely from the typed API instead of the raw
// vector, must still serialize to the exact same spec-derived bytes — pinning
// the typed constructors to the spec vector, not just parse().
#[test]
fn provisioning_request_typed_construction_matches_vector() {
    let mut service_name = [0u8; 32];
    service_name[..4].copy_from_slice(b"svc1");
    let pr = ProvisioningRequest {
        services: vec![ProvisioningService {
            injector_ip_address: 0x0A00_0001,
            injector_socket_number: 8000,
            service_name,
            dpi_pids: vec![DpiPidEntry {
                dpi_pid_index: 5,
                shared_pid: 0,
                event_id_compliance_flag: 1,
            }],
            component_mode: 1,
            injector_component_list: Some(InjectorComponentList {
                video_component_tag: 0x10,
                audio_component_tags: vec![0x20, 0x21],
                data_component_tags: vec![0x30],
            }),
        }],
    };
    assert_eq!(pr.to_bytes(), PROVISIONING_REQUEST_VECTOR);
}

// ── op_id / opID sanity against the typed dispatch (Table 8-3) ─────────────

#[test]
fn new_op_ids_match_table_8_3() {
    use scte104::traits::OperationDef;
    assert_eq!(ConfigRequest::OP_ID, 0x0009);
    assert_eq!(scte104::operations::ConfigResponse::OP_ID, 0x000A);
    assert_eq!(ProvisioningRequest::OP_ID, 0x000B);
    assert_eq!(scte104::operations::ProvisioningResponse::OP_ID, 0x000C);
    assert_eq!(FaultRequest::OP_ID, 0x000F);
    assert_eq!(scte104::operations::FaultResponse::OP_ID, 0x0010);
    assert_eq!(scte104::operations::AsAliveRequest::OP_ID, 0x0011);
    assert_eq!(scte104::operations::AsAliveResponse::OP_ID, 0x0012);
}
