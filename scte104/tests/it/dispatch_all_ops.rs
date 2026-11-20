//! Every `AnySingleOperation` opID arm must dispatch to its own typed
//! variant through the public entry point (ANSI/SCTE 104 2023 Table 8-3 /
//! Table 8-4, `private/specs-verified/scte/scte104/ansi_scte_104/`).

use broadcast_common::Serialize;
use scte104::operations::provisioning_request::ProvisioningRequest;
use scte104::operations::{AnySingleOperation, FaultRequest};
use scte104::time::Time;
use scte104::traits::OperationDef;

fn variant_name(op: &AnySingleOperation<'_>) -> &'static str {
    match op {
        AnySingleOperation::GeneralResponse(_) => "GENERAL_RESPONSE",
        AnySingleOperation::InitRequest(_) => "INIT_REQUEST",
        AnySingleOperation::InitResponse(_) => "INIT_RESPONSE",
        AnySingleOperation::AliveRequest(_) => "ALIVE_REQUEST",
        AnySingleOperation::AliveResponse(_) => "ALIVE_RESPONSE",
        AnySingleOperation::InjectResponse(_) => "INJECT_RESPONSE",
        AnySingleOperation::InjectCompleteResponse(_) => "INJECT_COMPLETE_RESPONSE",
        AnySingleOperation::ConfigRequest(_) => "CONFIG_REQUEST",
        AnySingleOperation::ConfigResponse(_) => "CONFIG_RESPONSE",
        AnySingleOperation::ProvisioningRequest(_) => "PROVISIONING_REQUEST",
        AnySingleOperation::ProvisioningResponse(_) => "PROVISIONING_RESPONSE",
        AnySingleOperation::FaultRequest(_) => "FAULT_REQUEST",
        AnySingleOperation::FaultResponse(_) => "FAULT_RESPONSE",
        AnySingleOperation::AsAliveRequest(_) => "AS_ALIVE_REQUEST",
        AnySingleOperation::AsAliveResponse(_) => "AS_ALIVE_RESPONSE",
        AnySingleOperation::Unknown { .. } => "UNKNOWN",
        _ => "OTHER",
    }
}

fn time_body() -> Vec<u8> {
    let mut out = [0u8; 8];
    Time::zero().serialize_into(&mut out).unwrap();
    out.to_vec()
}

fn fault_body() -> Vec<u8> {
    FaultRequest {
        injector_ip_address: 0,
        injector_socket_number: 0,
        injector_service_name: [0; 32],
        dpi_pid_index: 0,
    }
    .serialized_body()
}

fn provisioning_body() -> Vec<u8> {
    ProvisioningRequest {
        services: Vec::new(),
    }
    .serialized_body()
}

trait SerializedBody {
    fn serialized_body(&self) -> Vec<u8>;
}

impl<T: Serialize<Error = scte104::Error>> SerializedBody for T {
    fn serialized_body(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.serialized_len()];
        self.serialize_into(&mut out).unwrap();
        out
    }
}

#[test]
fn every_single_operation_id_dispatches_to_its_own_variant() {
    let cases: &[(u16, &str, Vec<u8>)] = &[
        (0x0000, "GENERAL_RESPONSE", Vec::new()),
        (0x0001, "INIT_REQUEST", Vec::new()),
        (0x0002, "INIT_RESPONSE", Vec::new()),
        (0x0003, "ALIVE_REQUEST", time_body()),
        (0x0004, "ALIVE_RESPONSE", time_body()),
        (0x0007, "INJECT_RESPONSE", vec![0x2A]),
        (0x0008, "INJECT_COMPLETE_RESPONSE", vec![0x01, 0x02]),
        (0x0009, "CONFIG_REQUEST", vec![0u8; 12]),
        (0x000A, "CONFIG_RESPONSE", vec![0x03, 0x01]),
        (0x000B, "PROVISIONING_REQUEST", provisioning_body()),
        (0x000C, "PROVISIONING_RESPONSE", Vec::new()),
        (0x000F, "FAULT_REQUEST", fault_body()),
        (0x0010, "FAULT_RESPONSE", Vec::new()),
        (0x0011, "AS_ALIVE_REQUEST", Vec::new()),
        (0x0012, "AS_ALIVE_RESPONSE", Vec::new()),
    ];

    for (op_id, want, body) in cases {
        let op = AnySingleOperation::dispatch(*op_id, body)
            .unwrap_or_else(|e| panic!("opID {op_id:#06X} must parse: {e}"));
        assert_eq!(
            variant_name(&op),
            *want,
            "opID {op_id:#06X} routed to the wrong variant"
        );
        let mut out = vec![0u8; op.body_len()];
        let n = op.serialize_body_into(&mut out).unwrap();
        assert_eq!(n, body.len(), "opID {op_id:#06X} body length");
        assert_eq!(&out, body, "opID {op_id:#06X} body bytes");
    }
}

#[test]
fn operation_ids_match_their_declared_constants() {
    assert_eq!(
        <scte104::operations::GeneralResponse as OperationDef>::OP_ID,
        0x0000
    );
    assert_eq!(
        <scte104::operations::InitRequest as OperationDef>::OP_ID,
        0x0001
    );
    assert_eq!(
        <scte104::operations::InitResponse as OperationDef>::OP_ID,
        0x0002
    );
    assert_eq!(
        <scte104::operations::AliveRequest as OperationDef>::OP_ID,
        0x0003
    );
    assert_eq!(
        <scte104::operations::AliveResponse as OperationDef>::OP_ID,
        0x0004
    );
    assert_eq!(
        <scte104::operations::InjectResponse as OperationDef>::OP_ID,
        0x0007
    );
    assert_eq!(
        <scte104::operations::InjectCompleteResponse as OperationDef>::OP_ID,
        0x0008
    );
    assert_eq!(
        <scte104::operations::ConfigRequest as OperationDef>::OP_ID,
        0x0009
    );
    assert_eq!(
        <scte104::operations::ConfigResponse as OperationDef>::OP_ID,
        0x000A
    );
    assert_eq!(ProvisioningRequest::OP_ID, 0x000B);
    assert_eq!(
        <scte104::operations::ProvisioningResponse as OperationDef>::OP_ID,
        0x000C
    );
    assert_eq!(FaultRequest::OP_ID, 0x000F);
    assert_eq!(
        <scte104::operations::FaultResponse as OperationDef>::OP_ID,
        0x0010
    );
    assert_eq!(
        <scte104::operations::AsAliveRequest as OperationDef>::OP_ID,
        0x0011
    );
    assert_eq!(
        <scte104::operations::AsAliveResponse as OperationDef>::OP_ID,
        0x0012
    );
}
