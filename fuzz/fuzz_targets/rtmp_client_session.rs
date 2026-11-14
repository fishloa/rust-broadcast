#![no_main]

use libfuzzer_sys::fuzz_target;
use rtmp_runtime::client::{ClientMode, ClientSession, RtmpClientConfig};
use rtmp_runtime::server::ServerSession;

// RTMP client entry point (#1460): the bytes a remote server sends, fed to
// `ClientSession` in play or publish mode, must never panic. The first byte
// picks the mode (bit 0), whether a real server handshake is prepended so the
// fuzzer starts past S0/S1/S2 (bit 1 clear), and the delivery chunk size
// (bits 2-7); the rest is the server's byte stream.
const MODE_PUBLISH_BIT: u8 = 0b01;
const RAW_STREAM_BIT: u8 = 0b10;
const CHUNK_SIZE_SHIFT: u32 = 2;
const MIN_CHUNK_SIZE: usize = 1;

fuzz_target!(|data: &[u8]| {
    let Some((&flags, stream)) = data.split_first() else {
        return;
    };
    let mut config = RtmpClientConfig::default();
    config.mode = if flags & MODE_PUBLISH_BIT == 0 {
        ClientMode::Play
    } else {
        ClientMode::Publish
    };
    let mut client = ClientSession::new(config);
    let c0_c1 = client.start();

    let mut input = Vec::new();
    if flags & RAW_STREAM_BIT == 0
        && let Ok((handshake, _)) = ServerSession::with_defaults().handle_data(&c0_c1)
    {
        input.extend_from_slice(&handshake);
    }
    input.extend_from_slice(stream);

    let chunk_size = usize::from(flags >> CHUNK_SIZE_SHIFT) + MIN_CHUNK_SIZE;
    for piece in input.chunks(chunk_size) {
        let _ = client.handle_data(piece);
    }
});
