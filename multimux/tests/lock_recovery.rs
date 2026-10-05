//! SP6.6: `parking_lot` has no poisoning; a panicking holder's guard drops
//! and the next acquire succeeds. Pre-fix this test cannot even compile as
//! written (the lock types were `std`'s), and the equivalent `std` behaviour
//! needed the deleted `crate::lock` recovery wrapper.

use std::sync::Arc;

#[test]
fn a_panicking_lock_holder_does_not_wedge_the_next_acquire() {
    let shared: Arc<parking_lot::Mutex<Vec<u32>>> = Arc::new(parking_lot::Mutex::new(vec![1]));
    let clone = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _g = clone.lock();
        panic!("holder dies mid-hold");
    })
    .join(); // the panic is expected

    // The next acquire succeeds and sees the pre-panic state.
    let got = shared.lock();
    assert_eq!(*got, vec![1], "the lock must be usable after a panicking holder");
}
