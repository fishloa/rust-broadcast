//! Sessions must stay movable between threads, or they cannot be run as their
//! own task on a multi-thread runtime. A `Cell`/`Rc` added later would break
//! that silently; these fail to compile instead.

use rtmp_runtime::client::ClientSession;
use rtmp_runtime::server::ServerSession;

fn assert_send<T: Send>() {}

#[test]
fn sessions_can_run_as_their_own_task() {
    assert_send::<ClientSession>();
    assert_send::<ServerSession>();
}

#[cfg(feature = "tokio")]
#[test]
fn the_socket_adapters_can_run_as_their_own_task() {
    use rtmp_runtime::io::{AsyncRtmpClient, AsyncRtmpServer, RtmpConnection};
    assert_send::<AsyncRtmpClient>();
    assert_send::<RtmpConnection>();
    assert_send::<AsyncRtmpServer>();
}
