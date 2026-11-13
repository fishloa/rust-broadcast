//! Handshakes must stay movable between threads, or a connection cannot be
//! run as its own task on a multi-thread runtime. A `Cell`/`Rc` added later
//! would break that silently; these fail to compile instead.

use srt_runtime::{CallerHandshake, ListenerHandshake, RendezvousHandshake};

fn assert_send<T: Send>() {}

#[test]
fn handshakes_can_run_as_their_own_task() {
    assert_send::<CallerHandshake>();
    assert_send::<ListenerHandshake>();
    assert_send::<RendezvousHandshake>();
}

#[cfg(feature = "tokio")]
#[test]
fn the_socket_adapters_can_run_as_their_own_task() {
    use srt_runtime::io::{SrtListener, SrtSocket};
    assert_send::<SrtSocket>();
    assert_send::<SrtListener>();
}
