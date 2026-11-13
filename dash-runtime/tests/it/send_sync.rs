//! A session must stay movable between threads, or it cannot be run as its
//! own task on a multi-thread runtime. A `Cell`/`Rc` added later would break
//! that silently; these fail to compile instead.

use dash_runtime::client::{DashClient, DashConfig};
use dash_runtime::ingest::DashifIngestSession;

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn a_client_can_run_as_its_own_task() {
    assert_send::<DashClient>();
}

#[test]
fn an_ingest_session_can_run_as_its_own_task() {
    assert_send::<DashifIngestSession>();
}

#[test]
fn the_configuration_can_be_shared() {
    assert_send_sync::<DashConfig>();
}
