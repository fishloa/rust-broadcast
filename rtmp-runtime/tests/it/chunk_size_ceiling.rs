//! #2741: a peer-announced chunk size above the engine ceiling is refused at
//! the session boundary, never accepted with the size silently clamped.

use rtmp_runtime::RtmpError;
use rtmp_runtime::chunk::{
    ChunkAssembler, ChunkWriter, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE, Message,
};
use rtmp_runtime::handshake::{HANDSHAKE_PACKET_LEN, RTMP_VERSION};
use rtmp_runtime::message::{
    CONTROL_CHUNK_STREAM_ID, CONTROL_MESSAGE_STREAM_ID, ChunkSize, MessageTypeId,
};
use rtmp_runtime::server::ServerSession;

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

#[test]
fn a_peer_chunk_size_above_the_engine_ceiling_closes_the_session() {
    let above_ceiling = MAX_CHUNK_SIZE + 1;
    assert!(
        above_ceiling <= ChunkSize::MAX,
        "the case under test needs a legal wire size the engine cannot adopt"
    );

    let announcement = Message {
        chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
        timestamp: 0,
        message_type_id: MessageTypeId::SetChunkSize,
        message_stream_id: CONTROL_MESSAGE_STREAM_ID,
        payload: above_ceiling.to_be_bytes().to_vec(),
    };
    let wire = ChunkWriter::new()
        .write(&announcement)
        .expect("the control message frames at the default chunk size");

    let mut session = handshaken_session();
    let err = session
        .handle_data(&wire)
        .expect_err("a chunk size the engine cannot adopt must be refused");
    assert!(matches!(err, RtmpError::Malformed { .. }), "{err:?}");
}

#[test]
fn the_ceiling_itself_is_adopted_verbatim() {
    let ceiling = ChunkSize::new(MAX_CHUNK_SIZE).expect("the ceiling is a legal size");
    assert_eq!(ceiling.value(), MAX_CHUNK_SIZE);
    assert_eq!(
        ChunkAssembler::new().with_chunk_size(ceiling).pending_len(),
        0
    );
    assert_eq!(DEFAULT_CHUNK_SIZE.value(), 128);
}
