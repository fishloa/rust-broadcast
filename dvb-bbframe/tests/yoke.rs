//! `yoke` feature smoke test: a zero-copy user-packet iterator view is yoked
//! to its owning buffer, so it outlives the source `Vec<u8>` without a
//! re-parse. (Mirrors dvb-si's `yoke` feature shape — exercises the
//! `Yokeable` derive on `NmTsIter` via `yoke::Yoke` directly.)
#![cfg(feature = "yoke")]

use std::sync::Arc;

use dvb_bbframe::packet::{NM_UP_SIZE, NmTsIter, TS_SYNC_BYTE};
use yoke::Yoke;

/// Build a data field holding two back-to-back NM user packets.
fn two_up_data_field() -> Vec<u8> {
    let mut data = Vec::with_capacity(2 * NM_UP_SIZE);
    for up in 0..2u8 {
        data.push(0x00); // CRC-8 byte (replaced with sync on iteration)
        for j in 1..NM_UP_SIZE {
            data.push(up.wrapping_mul(10).wrapping_add(j as u8));
        }
    }
    data
}

#[test]
fn yoked_iter_outlives_source_vec() {
    // The source `Vec<u8>` is consumed into the Arc cart and dropped from this
    // scope; the yoked iterator view must keep working afterwards.
    let yoked: Yoke<NmTsIter<'static>, Arc<[u8]>> = {
        let cart: Arc<[u8]> = Arc::from(two_up_data_field()); // source moved here
        Yoke::attach_to_cart(cart, |b| NmTsIter::new(b))
    };

    // The view still points at live, owned bytes: `size_hint` reports the two
    // unconsumed packets without re-parsing or borrowing the original Vec.
    let (lo, hi) = yoked.get().size_hint();
    assert_eq!(lo, 2);
    assert_eq!(hi, Some(2));

    // `get()` borrows from the yoke's cart; the iterator is `Copy`, so drive a
    // copy to completion — the bytes are intact long after `source` was dropped.
    let mut it: NmTsIter<'_> = *yoked.get();
    let first = it.next().expect("first UP");
    assert_eq!(first[0], TS_SYNC_BYTE);
    assert_eq!(it.count(), 1); // one UP remains
}

#[test]
fn yoked_iter_crosses_thread_boundary() {
    let yoked: Yoke<NmTsIter<'static>, Arc<[u8]>> =
        Yoke::attach_to_cart(Arc::from(two_up_data_field()), |b| NmTsIter::new(b));

    let count = std::thread::spawn(move || (*yoked.get()).count())
        .join()
        .unwrap();
    assert_eq!(count, 2);
}
