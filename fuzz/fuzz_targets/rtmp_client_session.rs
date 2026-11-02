#![no_main]

use libfuzzer_sys::fuzz_target;
use rtmp_runtime::client::{ClientConfig, ClientMode, ClientSession};
use rtmp_runtime::server::ServerSession;

const MIN_CHUNK_SIZE: usize = 1;
const MAX_CHUNK_SIZE: usize = 64;
const MODE_BIT_MASK: u8 = 1;
const SECOND_BYTE_PARITY_DIVISOR: u8 = 2;
const FLAG_CURSOR: usize = 2;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let mode = if data[0] & MODE_BIT_MASK == 0 {
        ClientMode::Play
    } else {
        ClientMode::Publish
    };

    let mut config = ClientConfig::default();
    config.mode = mode;

    let mut client = ClientSession::new(config);

    let mut cursor = 1usize;
    let prepend_handshake = data.len() > 1 && data[1].is_multiple_of(SECOND_BYTE_PARITY_DIVISOR);
    let mut input: Vec<u8> = Vec::new();

    if prepend_handshake {
        cursor = FLAG_CURSOR;
        let c0_c1 = client.start();
        let mut server = ServerSession::with_defaults();
        if let Ok((server_bytes, _)) = server.handle_data(&c0_c1) {
            input.extend_from_slice(&server_bytes);
        }
    } else {
        let _ = client.start();
    }

    input.extend_from_slice(&data[cursor..]);

    let mut offset = 0usize;
    while offset < input.len() {
        let size_byte = usize::from(input[offset]);
        let chunk_size = (size_byte % MAX_CHUNK_SIZE) + MIN_CHUNK_SIZE;
        offset += MIN_CHUNK_SIZE;
        let end = (offset + chunk_size).min(input.len());
        let chunk = &input[offset..end];
        offset = end;

        let _ = client.handle_data(chunk);
    }
});
