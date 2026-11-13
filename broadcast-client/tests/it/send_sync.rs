use broadcast_client::{Action, Config, Event, Session};

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn a_session_can_run_as_its_own_task_on_a_multi_thread_runtime() {
    assert_send::<Session>();
}

#[test]
fn the_values_a_host_shares_or_moves_between_tasks_are_send_and_sync() {
    assert_send_sync::<Config>();
    assert_send_sync::<Event>();
    assert_send_sync::<Action>();
}
