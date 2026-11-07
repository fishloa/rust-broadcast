use srt_runtime::Error;
use srt_runtime::packet::handshake::{ExtensionType, HandshakeExtensionBlock};

#[test]
fn a_stream_id_that_is_not_utf8_is_invalid_stream_id_utf8() {
    let block = HandshakeExtensionBlock {
        ext_type: ExtensionType::Sid,
        contents: &[0xFF; 4],
    };
    assert_eq!(block.as_stream_id().unwrap_err(), Error::InvalidStreamIdUtf8);
}
