//! NORM serialization contracts and truncation guards (RFC 5740 §4): the
//! number `serialize_into` returns equals `serialized_len()`, every short
//! prefix is rejected without panicking, and an `hdr_len` smaller than the
//! fixed header is an `InconsistentLength`.

use broadcast_core::Serialize;
use rmt_flute::{
    FecPayloadId, HeaderExtension, NORM_VERSION, NormAckType, NormCmd, NormCmdBody, NormCmdType,
    NormCommonHeader, NormData, NormFeedback, NormInfo, NormMessageType, SenderWord,
};

static EXT_CONTENT: [u8; 3] = [0xAA, 0xBB, 0xCC];

fn ext() -> HeaderExtension<'static> {
    HeaderExtension::new(200, &EXT_CONTENT)
}

fn common(ty: NormMessageType) -> NormCommonHeader {
    NormCommonHeader {
        version: NORM_VERSION,
        message_type: ty,
        sequence: 0x1234,
        source_id: 0xCAFE_BABE,
    }
}

fn sender() -> SenderWord {
    SenderWord {
        instance_id: 0x00FF,
        grtt: 0x40,
        backoff: 0x0A,
        gsize: 0x05,
    }
}

fn info(payload: &'static [u8]) -> NormInfo<'static> {
    NormInfo {
        common: common(NormMessageType::Info),
        sender: sender(),
        flags: 0x04,
        fec_id: 129,
        object_transport_id: 0x1234,
        extensions: vec![ext()],
        payload,
    }
}

fn data(payload: &'static [u8]) -> NormData<'static> {
    NormData {
        common: common(NormMessageType::Data),
        sender: sender(),
        flags: 0x05,
        fec_id: 129,
        object_transport_id: 0x1234,
        fec_payload_id: FecPayloadId::new(&[0x01, 0x02, 0x03, 0x04]),
        extensions: vec![ext()],
        payload,
    }
}

fn cmd(body: NormCmdBody, sub_type: NormCmdType, content: &'static [u8]) -> NormCmd<'static> {
    NormCmd {
        common: common(NormMessageType::Cmd),
        sender: sender(),
        sub_type,
        reserved: [0; 3],
        body,
        extensions: vec![ext()],
        content,
    }
}

fn all_cmds(content: &'static [u8]) -> Vec<NormCmd<'static>> {
    let fec = || FecPayloadId::new(&[0x01, 0x02, 0x03, 0x04]);
    vec![
        cmd(
            NormCmdBody::Flush {
                fec_id: 129,
                object_transport_id: 0x0102,
                fec_payload_id: fec(),
            },
            NormCmdType::Flush,
            content,
        ),
        cmd(NormCmdBody::Eot, NormCmdType::Eot, content),
        cmd(
            NormCmdBody::Squelch {
                fec_id: 129,
                object_transport_id: 0x0304,
                fec_payload_id: fec(),
            },
            NormCmdType::Squelch,
            content,
        ),
        cmd(
            NormCmdBody::Cc {
                cc_sequence: 0x0102,
                send_time_sec: 0x0304_0506,
                send_time_usec: 0x0708_090A,
            },
            NormCmdType::Cc,
            content,
        ),
        cmd(
            NormCmdBody::RepairAdv { flags: 0x01 },
            NormCmdType::RepairAdv,
            content,
        ),
        cmd(
            NormCmdBody::AckReq {
                ack_type: NormAckType::Flush,
                ack_id: 0x07,
            },
            NormCmdType::AckReq,
            content,
        ),
        cmd(NormCmdBody::Application, NormCmdType::Application, content),
        cmd(
            NormCmdBody::Other([0x01, 0x02, 0x03]),
            NormCmdType::Other(9),
            content,
        ),
    ]
}

fn feedback(payload: &'static [u8]) -> NormFeedback<'static> {
    NormFeedback {
        common: common(NormMessageType::Nack),
        server_id: 0x0102_0304,
        instance_id: 0x0506,
        ack_or_reserved: 0x0708,
        grtt_response_sec: 0x090A_0B0C,
        grtt_response_usec: 0x0D0E_0F10,
        extensions: vec![ext()],
        payload,
    }
}

fn assert_return_matches_len<T>(message: &T)
where
    T: Serialize,
    T::Error: core::fmt::Debug,
{
    let mut buf = vec![0u8; message.serialized_len()];
    let written = message.serialize_into(&mut buf).unwrap();
    assert_eq!(written, message.serialized_len());
    let bytes = message.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), written);
    assert_eq!(&bytes[..], &buf[..written]);
}

#[test]
fn serialize_into_returns_the_serialized_length_for_every_message_type() {
    let info = info(b"info payload");
    assert_return_matches_len(&info);
    let bytes = info.try_to_bytes().unwrap();
    assert_eq!(NormInfo::parse(&bytes).unwrap(), info);

    let data = data(b"data payload");
    assert_return_matches_len(&data);
    let bytes = data.try_to_bytes().unwrap();
    assert_eq!(NormData::parse(&bytes, 4).unwrap(), data);

    for message in all_cmds(b"command content") {
        assert_return_matches_len(&message);
        let bytes = message.try_to_bytes().unwrap();
        let parsed = NormCmd::parse(&bytes, 4).unwrap();
        assert_eq!(parsed, message);
    }

    let feedback = feedback(b"feedback payload");
    assert_return_matches_len(&feedback);
    let bytes = feedback.try_to_bytes().unwrap();
    assert_eq!(NormFeedback::parse(&bytes).unwrap(), feedback);
}

#[test]
fn every_short_prefix_is_rejected_without_panicking() {
    let info = info(&[]);
    let bytes = info.try_to_bytes().unwrap();
    for k in 0..bytes.len() {
        assert!(
            NormInfo::parse(&bytes[..k]).is_err(),
            "NORM_INFO prefix {k} must be rejected"
        );
    }

    let data = data(&[]);
    let bytes = data.try_to_bytes().unwrap();
    for k in 0..bytes.len() {
        assert!(
            NormData::parse(&bytes[..k], 4).is_err(),
            "NORM_DATA prefix {k} must be rejected"
        );
    }

    for message in all_cmds(&[]) {
        let bytes = message.try_to_bytes().unwrap();
        for k in 0..bytes.len() {
            assert!(
                NormCmd::parse(&bytes[..k], 4).is_err(),
                "NORM_CMD({}) prefix {k} must be rejected",
                message.sub_type.name()
            );
        }
    }

    let feedback = feedback(&[]);
    let bytes = feedback.try_to_bytes().unwrap();
    for k in 0..bytes.len() {
        assert!(
            NormFeedback::parse(&bytes[..k]).is_err(),
            "NORM feedback prefix {k} must be rejected"
        );
    }
}

#[test]
fn a_norm_cmd_hdr_len_below_the_fixed_header_is_inconsistent() {
    let message = cmd(NormCmdBody::Eot, NormCmdType::Eot, &[]);
    let mut bytes = message.try_to_bytes().unwrap();
    bytes[1] = 1;
    assert!(matches!(
        NormCmd::parse(&bytes, 0),
        Err(rmt_flute::Error::InconsistentLength { .. })
    ));
}

#[test]
fn a_norm_feedback_hdr_len_below_the_fixed_header_is_inconsistent() {
    let message = feedback(&[]);
    let mut bytes = message.try_to_bytes().unwrap();
    bytes[1] = 1;
    assert!(matches!(
        NormFeedback::parse(&bytes),
        Err(rmt_flute::Error::InconsistentLength { .. })
    ));
}
