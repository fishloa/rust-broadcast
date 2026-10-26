use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

const FIRST_CONN_ID: u32 = 1;

type LiveIds = Arc<Mutex<BTreeSet<u32>>>;

/// Connection-id allocator shared by every adapter of one session. An id stays reserved while its
/// [`ConnLease`] lives, so a counter that wraps never hands out an id still in use.
#[derive(Debug, Clone)]
pub(crate) struct ConnAlloc {
    next: Arc<AtomicU32>,
    live: LiveIds,
}

/// A reserved connection id, released on drop.
#[derive(Debug)]
pub(crate) struct ConnLease {
    id: u32,
    live: LiveIds,
}

impl ConnLease {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for ConnLease {
    fn drop(&mut self) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl ConnAlloc {
    pub(crate) fn new() -> Self {
        Self {
            next: Arc::new(AtomicU32::new(FIRST_CONN_ID)),
            live: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub(crate) fn lease(&self) -> ConnLease {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let candidate = self
                .next
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                    Some(c.checked_add(1).unwrap_or(FIRST_CONN_ID))
                })
                .unwrap_or(FIRST_CONN_ID);
            if live.insert(candidate) {
                return ConnLease {
                    id: candidate,
                    live: self.live.clone(),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl ConnAlloc {
        fn starting_at(n: u32) -> Self {
            let alloc = Self::new();
            alloc.next.store(n, Ordering::Relaxed);
            alloc
        }
    }

    #[test]
    fn ids_start_at_one_and_count_up() {
        let alloc = ConnAlloc::new();
        let a = alloc.lease();
        let b = alloc.lease();
        assert_eq!((a.id(), b.id()), (FIRST_CONN_ID, FIRST_CONN_ID + 1));
    }

    #[test]
    fn lease_skips_ids_still_held_after_the_counter_wraps() {
        let alloc = ConnAlloc::starting_at(u32::MAX - 1);
        let a = alloc.lease();
        let b = alloc.lease();
        assert_eq!((a.id(), b.id()), (u32::MAX - 1, u32::MAX));
        let c = alloc.lease();
        assert_eq!(c.id(), FIRST_CONN_ID);

        alloc.next.store(u32::MAX - 1, Ordering::Relaxed);
        let d = alloc.lease();
        assert!(
            ![a.id(), b.id(), c.id()].contains(&d.id()),
            "reused live id {}",
            d.id()
        );

        drop(a);
        alloc.next.store(u32::MAX - 1, Ordering::Relaxed);
        assert_eq!(
            alloc.lease().id(),
            u32::MAX - 1,
            "a released id is reusable"
        );
    }

    #[test]
    fn clones_share_one_id_space() {
        let alloc = ConnAlloc::new();
        let other = alloc.clone();
        let a = alloc.lease();
        let b = other.lease();
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn a_dropped_lease_releases_its_id_but_a_live_one_does_not() {
        let alloc = ConnAlloc::starting_at(7);
        let held = alloc.lease();
        alloc.next.store(7, Ordering::Relaxed);
        let next = alloc.lease();
        assert_ne!(
            next.id(),
            held.id(),
            "a live lease is never handed out again"
        );
        drop(held);
        alloc.next.store(7, Ordering::Relaxed);
        assert_eq!(alloc.lease().id(), 7);
    }
}
