//! #2944: the numeric value of the Command Message length cap is the security
//! property, so it is pinned literally, together with both edges of the
//! accept/reject boundary.

use broadcast_core::Serialize;
use rtmp_runtime::RtmpError;
use rtmp_runtime::amf0::{Amf0Value, MAX_AMF0_VALUES};
use rtmp_runtime::chunk::{ChunkWriter, Message};
use rtmp_runtime::handshake::{HANDSHAKE_PACKET_LEN, RTMP_VERSION};
use rtmp_runtime::ids::{ChunkStreamId, MessageStreamId, Timestamp};
use rtmp_runtime::message::MessageTypeId;
use rtmp_runtime::server::{MAX_COMMAND_LEN, ServerEvent, ServerSession};

const CLIENT_CSID: u32 = 3;

fn c0_c1() -> Vec<u8> {
    let mut bytes = vec![0u8; 1 + HANDSHAKE_PACKET_LEN];
    bytes[0] = RTMP_VERSION;
    bytes
}

fn handshaken_session() -> ServerSession {
    let mut session = ServerSession::with_defaults();
    session.handle_data(&c0_c1()).expect("C0+C1");
    session
        .handle_data(&vec![0u8; HANDSHAKE_PACKET_LEN])
        .expect("C2");
    session
}

/// A valid AMF0 `connect` command whose encoded body is `len` bytes: the
/// padding rides in a Long String property of the command object, so the body
/// stays a well-formed command at every length.
fn connect_payload_of_len(len: usize) -> Vec<u8> {
    // name (10) + transaction id (9) + object marker (1) + `app` key/value
    // (5 + 7) + `pad` key (5) + Long String prefix (5) + object end (3).
    const FIXED: usize = 45;
    let pad = len - FIXED;
    let mut payload = Vec::with_capacity(len);
    payload.extend(
        Amf0Value::String("connect".to_string())
            .try_to_bytes()
            .expect("name encodes"),
    );
    payload.extend(
        Amf0Value::Number(1.0)
            .try_to_bytes()
            .expect("transaction id encodes"),
    );
    payload.extend(
        Amf0Value::Object(vec![
            ("app".to_string(), Amf0Value::String("live".to_string())),
            ("pad".to_string(), Amf0Value::LongString("x".repeat(pad))),
        ])
        .try_to_bytes()
        .expect("command object encodes"),
    );
    payload
}

fn command_message(message_type_id: MessageTypeId, payload: Vec<u8>) -> Vec<u8> {
    let message = Message {
        chunk_stream_id: ChunkStreamId::new(CLIENT_CSID),
        timestamp: Timestamp::from_millis(0),
        message_type_id,
        message_stream_id: MessageStreamId::new(0),
        payload,
    };
    ChunkWriter::new().write(&message).expect("frames")
}

#[test]
fn the_command_length_cap_is_exactly_64_kib() {
    assert_eq!(MAX_COMMAND_LEN, 65_536);
    assert_eq!(MAX_AMF0_VALUES, 4096);
}

#[test]
fn a_command_body_of_exactly_the_cap_is_dispatched() {
    let payload = connect_payload_of_len(65_536);
    assert_eq!(payload.len(), 65_536);
    let mut session = handshaken_session();
    let (out, events) = session
        .handle_data(&command_message(MessageTypeId::CommandAmf0, payload))
        .expect("a body exactly at the cap is legal");
    assert!(!out.is_empty(), "the connect reply is written");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ServerEvent::Connected { app } if app == "live")),
        "the connect command ran: {events:?}"
    );
}

#[test]
fn one_byte_past_the_cap_is_rejected() {
    let mut session = handshaken_session();
    let err = session
        .handle_data(&command_message(
            MessageTypeId::CommandAmf0,
            vec![rtmp_runtime::amf0::marker::NULL; 65_537],
        ))
        .expect_err("one byte past the cap is refused");
    assert!(
        matches!(err, RtmpError::CommandTooLong { len, max } if len == 65_537 && max == 65_536),
        "{err:?}"
    );
}

/// A single chunk declaring a message body of `len` bytes and carrying one
/// whole default-size (128-byte) chunk of it: the assembler reserves the
/// declaration on the first push and waits for the rest.
fn chunk_declaring(len: u32) -> Vec<u8> {
    const DEFAULT_CHUNK: usize = 128;
    let mut wire = vec![CLIENT_CSID as u8];
    wire.extend_from_slice(&[0, 0, 0]);
    let len_bytes = len.to_be_bytes();
    wire.extend_from_slice(&len_bytes[1..]);
    wire.push(0x09);
    wire.extend_from_slice(&[0, 0, 0, 1]);
    wire.extend(core::iter::repeat_n(0xAAu8, DEFAULT_CHUNK));
    wire
}

#[test]
fn the_configured_receive_budget_is_honoured() {
    let config = rtmp_runtime::server::RtmpServerConfig::default().with_max_in_progress_bytes(1234);
    let mut session = ServerSession::new(config);
    session.handle_data(&c0_c1()).expect("C0+C1");
    session
        .handle_data(&vec![0u8; HANDSHAKE_PACKET_LEN])
        .expect("C2");
    // A declaration past the configured budget is consumed without being
    // buffered and reported: the connection keeps framing instead of ending.
    let (out, events) = session
        .handle_data(&chunk_declaring(2000))
        .expect("a declaration past the configured budget is skipped");
    assert!(out.is_empty());
    assert_eq!(
        events.len(),
        1,
        "the skipped message is reported: {events:?}"
    );
    assert!(matches!(
        &events[0],
        ServerEvent::Damage(d)
            if d.kind() == broadcast_core::damage::DamageKind::LimitReached
    ));

    let mut default_session = handshaken_session();
    let (out, events) = default_session
        .handle_data(&chunk_declaring(2000))
        .expect("the default budget accepts the same declaration");
    assert!(out.is_empty() && events.is_empty(), "still incomplete");
}
