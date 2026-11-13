//! The hub and its handles must stay shareable between threads: a `Trunk` is
//! shared by every task that publishes to or reads from it, and writers and
//! cursors are moved into those tasks. A `Cell`/`Rc` added later would break
//! that silently; these fail to compile instead.
#![cfg(feature = "std")]

use media_plane::trunk::{SampleCursor, SegmentCursor, SegmentWriter, Trunk, TrunkWriter};

fn assert_send<T: Send>() {}
fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn a_trunk_is_shared_between_tasks() {
    assert_send_sync::<Trunk>();
}

#[test]
fn writers_and_cursors_can_move_into_their_own_task() {
    assert_send::<TrunkWriter>();
    assert_send::<SegmentWriter>();
    assert_send::<SampleCursor>();
    assert_send::<SegmentCursor>();
}
