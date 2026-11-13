//! A session or origin must stay movable between threads, or it cannot be run
//! as its own task on a multi-thread runtime. A `Cell`/`Rc` added later would
//! break that silently; these fail to compile instead.

use hls_runtime::client::HlsClient;
#[cfg(feature = "std")]
use hls_runtime::server::HlsOrigin;

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn a_client_can_run_as_its_own_task() {
    assert_send::<HlsClient>();
}

#[cfg(feature = "std")]
#[test]
fn an_origin_can_be_shared_between_request_tasks() {
    assert_send_sync::<HlsOrigin>();
}
