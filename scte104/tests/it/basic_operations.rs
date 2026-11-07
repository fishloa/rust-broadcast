//! Wire-byte, round-trip, truncation and short-buffer tests for the basic
//! (single-operation) bodies hand-written in `scte104/src/operations/mod.rs`:
//! `alive_request_data()` / `alive_response_data()` (Tables 9-3/9-4, `time()` per
//! §12.4), `inject_response_data()` (Table 9-14), `injectcomplete_response_data()`
//! (Table 9-16) and `config_response_data()` (Table 10-2). Spec transcriptions:
//! `private/specs-verified/scte/scte104/ansi_scte_104/operations.md` and
//! `pams_operations.md`.

use broadcast_common::{Parse, Serialize};
use scte104::Error;
use scte104::operations::{
    AliveRequest, AliveResponse, ConfigRequest, ConfigResponse, FaultRequest,
    InjectCompleteResponse, InjectResponse,
};
use scte104::time::Time;

fn too_small(e: &Error) -> Option<(usize, usize)> {
    match e {
        Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
            need,
            have,
        }) => Some((*need, *have)),
        _ => None,
    }
}

fn assert_short_buffers<T: Serialize<Error = Error>>(value: &T) {
    test_fixtures::assert_short_buffers_rejected(
        value.serialized_len(),
        |buf| value.serialize_into(buf),
        too_small,
    );
}

fn assert_truncations_rejected<'a, T: Parse<'a, Error = Error> + core::fmt::Debug>(
    wire: &'a [u8],
    what: &'static str,
) {
    for have in 0..wire.len() {
        match T::parse(&wire[..have]) {
            Err(Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need,
                have: got,
                what: w,
            })) => {
                assert_eq!(
                    (need, got, w),
                    (wire.len(), have, what),
                    "{have}-byte prefix"
                );
            }
            other => panic!("{have}-byte prefix of {what}: {other:?}"),
        }
    }
}

#[rustfmt::skip]
const TIME_WIRE: [u8; 8] = [
    0x2A, 0x4C, 0x9E, 0x10,
    0x00, 0x07, 0xA1, 0x20,
];

fn time() -> Time {
    Time {
        seconds: 0x2A4C_9E10,
        microseconds: 500_000,
    }
}

#[test]
fn alive_request_wire_round_trip_and_truncation() {
    let op = AliveRequest { time: time() };
    assert_eq!(op.try_to_bytes().unwrap(), TIME_WIRE);
    assert_eq!(AliveRequest::parse(&TIME_WIRE).unwrap(), op);
    assert_truncations_rejected::<AliveRequest>(&TIME_WIRE, "time()");
    assert_short_buffers(&op);
}

#[test]
fn alive_response_wire_round_trip_and_truncation() {
    let op = AliveResponse { time: time() };
    assert_eq!(op.try_to_bytes().unwrap(), TIME_WIRE);
    assert_eq!(AliveResponse::parse(&TIME_WIRE).unwrap(), op);
    assert_truncations_rejected::<AliveResponse>(&TIME_WIRE, "time()");
    assert_short_buffers(&op);
}

#[test]
fn alive_time_zero_signals_unused() {
    let zero = AliveRequest { time: Time::zero() };
    assert_eq!(zero.try_to_bytes().unwrap(), [0u8; 8]);
    assert_eq!(AliveRequest::parse(&[0u8; 8]).unwrap(), zero);
}

#[test]
fn inject_response_wire_round_trip_and_truncation() {
    let op = InjectResponse { message_number: 42 };
    assert_eq!(op.try_to_bytes().unwrap(), [0x2A]);
    assert_eq!(InjectResponse::parse(&[0x2A]).unwrap(), op);
    assert_eq!(
        InjectResponse::parse(&[]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 1,
                have: 0,
                what: "inject_response message_number",
            }
        ))
    );
    assert_short_buffers(&op);
}

#[test]
fn inject_complete_response_wire_round_trip_and_truncation() {
    let op = InjectCompleteResponse {
        message_number: 42,
        cue_message_count: 3,
    };
    assert_eq!(op.try_to_bytes().unwrap(), [0x2A, 0x03]);
    assert_eq!(InjectCompleteResponse::parse(&[0x2A, 0x03]).unwrap(), op);
    assert_truncations_rejected::<InjectCompleteResponse>(
        &[0x2A, 0x03],
        "inject_complete_response",
    );
    assert_short_buffers(&op);
}

#[test]
fn config_response_wire_round_trip_and_truncation() {
    let op = ConfigResponse {
        as_index: 7,
        permanent_connection_requested: 1,
    };
    assert_eq!(op.try_to_bytes().unwrap(), [0x07, 0x01]);
    assert_eq!(ConfigResponse::parse(&[0x07, 0x01]).unwrap(), op);
    assert_truncations_rejected::<ConfigResponse>(&[0x07, 0x01], "config_response_data");
    assert_short_buffers(&op);
}

#[test]
fn config_and_fault_request_reject_truncation_and_short_buffers() {
    let config = ConfigRequest {
        as_ip_address: 0xC0A8_0101,
        as_socket_number: 5167,
        activeflag: 1,
        protocol_version: 0,
        last_as_index: 7,
        last_injectorcount: 3,
        permanent_connection_requested: 1,
    };
    let wire = config.try_to_bytes().unwrap();
    assert_eq!(wire.len(), 12);
    assert_truncations_rejected::<ConfigRequest>(&wire, "config_request_data");
    assert_short_buffers(&config);

    let mut name = [0u8; 32];
    name[..4].copy_from_slice(b"svc1");
    let fault = FaultRequest {
        injector_ip_address: 0x0A00_0002,
        injector_socket_number: 8000,
        injector_service_name: name,
        dpi_pid_index: 5,
    };
    let wire = fault.try_to_bytes().unwrap();
    assert_eq!(wire.len(), 40);
    assert_truncations_rejected::<FaultRequest>(&wire, "fault_request_data");
    assert_short_buffers(&fault);
}
