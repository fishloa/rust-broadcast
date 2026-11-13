//! The Smooth client must be movable into its own task and the origin shareable behind an `Arc`,
//! or neither can run on a multi-thread runtime. A `Cell`/`Rc` added later fails to compile here.

use smooth_runtime::{Action, SmoothClient, SmoothConfig};

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn the_client_runs_as_its_own_task() {
    assert_send::<SmoothClient>();
    assert_send_sync::<Action>();
    assert_send_sync::<SmoothConfig>();
}

#[cfg(feature = "std")]
#[test]
fn the_origin_and_its_config_are_shared_across_tasks() {
    use smooth_runtime::{SmoothOrigin, SmoothOriginConfig};
    assert_send_sync::<SmoothOrigin>();
    assert_send_sync::<SmoothOriginConfig>();
}
