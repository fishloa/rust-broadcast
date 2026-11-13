//! Sessions must stay movable between threads, and a server session shareable
//! behind an `Arc`, or neither can be run as its own task on a multi-thread
//! runtime. A `Cell`/`Rc` added later would break that silently; these fail
//! to compile instead.

use rtsp_runtime::{ClientSession, PlaySession, ServerSession};

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn sessions_can_run_as_their_own_task() {
    assert_send::<ClientSession>();
    assert_send::<PlaySession>();
}

#[test]
fn a_server_session_can_be_shared_behind_an_arc() {
    assert_send_sync::<ServerSession>();
}

#[cfg(feature = "tokio")]
#[test]
fn the_socket_adapters_can_run_as_their_own_task() {
    use rtsp_runtime::{AsyncRtspClient, AsyncRtspServer};
    use tokio::net::TcpStream;
    assert_send::<AsyncRtspClient<TcpStream>>();
    assert_send::<AsyncRtspServer<TcpStream>>();
}
