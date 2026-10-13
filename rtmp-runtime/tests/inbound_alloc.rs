//! r08-RTMP-O1 (#1085): the steady-state inbound path (post-handshake reads)
//! must not copy each read into a scratch `Vec` before the chunk assembler
//! buffers it — it used to `to_vec()` the input, then copy it again.

use rtmp_runtime::client::{ClientConfig, ClientEvent, ClientSession};
use rtmp_runtime::server::{ServerConfig, ServerEvent, ServerSession};
use test_alloc::ThreadCounting;

#[global_allocator]
static ALLOC: ThreadCounting = ThreadCounting::new();

/// Drive a client and a server session in memory until the client is
/// publishing; returns both, ready for media.
fn publishing_pair() -> (ClientSession, ServerSession) {
    let mut config = ClientConfig::default();
    config.app = "live".into();
    config.stream_key = "key".into();
    let mut client = ClientSession::new(config);
    let mut server = ServerSession::new(ServerConfig::default());
    let mut to_server = client.start();
    let mut publishing = false;
    let mut server_published = false;
    for _ in 0..32 {
        let (to_client, sevents) = server.handle_data(&to_server).expect("server");
        server_published |= sevents
            .iter()
            .any(|e| matches!(e, ServerEvent::Publish { .. }));
        let (next, cevents) = client.handle_data(&to_client).expect("client");
        publishing |= cevents.iter().any(|e| matches!(e, ClientEvent::Publishing));
        to_server = next;
        if publishing && server_published && to_server.is_empty() {
            return (client, server);
        }
    }
    panic!("handshake/publish did not complete");
}

#[test]
fn steady_state_read_is_not_copied_before_buffering() {
    let (mut client, mut server) = publishing_pair();
    // A 100 000-byte video message, cut mid-message: the assembler can only
    // buffer it, so the only legitimate copies are its own.
    let wire = client
        .send_video(0, &vec![0xAB; 100_000])
        .expect("send_video");
    let cut = 60_000;
    let (res, snap) = ThreadCounting::measure(|| server.handle_data(&wire[..cut]));
    let (out, events) = res.expect("partial message is not an error");
    assert!(events.is_empty() && out.is_empty());
    eprintln!("bytes allocated: {}", snap.bytes);
    // The assembler's own message buffer (100 000) + one copy of the input
    // (`cut`) + ~27 000 of bookkeeping measured; before the fix a second full
    // copy of `cut` bytes preceded it (~247 000), so 200 000 separates them.
    assert!(
        snap.bytes < 100_000 + cut + 40_000,
        "allocated {} bytes feeding {cut}",
        snap.bytes
    );
}
