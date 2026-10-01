//! Poison-recovering lock helpers (audit run 7, W2).
//!
//! Every mutex/`RwLock` this crate holds guards **structurally consistent**
//! state: the guarded value is only ever mutated through short, panic-free
//! assignments (a `HashMap` insert/remove, an `Option` replacement, a
//! `VecDeque` push), never a multi-step invariant that a panic could leave
//! half-applied. A panic while such a lock is held (a third-party
//! `SchemeRegistry` factory, a `transmux` segmenter, an allocation failure)
//! therefore cannot corrupt the value — but `std::sync` still marks the
//! mutex poisoned, and every later `.lock().unwrap()` panics forever after.
//! One request could disable a route, a whole router rebuild, or the admin
//! API for the remaining lifetime of the process.
//!
//! [`lock`]/[`read`]/[`write`] recover from poisoning
//! ([`PoisonError::into_inner`]) rather than propagating it. This is
//! deliberately the opposite of the usual "poison is a strong signal"
//! guidance: for state that cannot be observed mid-mutation, recovery is
//! strictly better than a permanent panic cascade.
//!
//! # What recovery is NOT sound for
//!
//! Recovery is only sound for state where **every** mutation is a single,
//! panic-free assignment (insert/remove/replace). It is **not** sound for a
//! value mutated through several steps that must stay mutually consistent:
//! `ProgramServing::dvr` (`Mutex<Option<DvrRecorder>>`) is exactly that —
//! `DvrRecorder` holds `write_offset`, `index`, `periods` and `total_bytes`,
//! and a panic midway through `append_segment` (between the file write and
//! the index push, say) leaves them disagreeing. Recovering and continuing to
//! persist would then write an archive whose index does not match its data.
//! Callers that hold such state must **fail closed** on a poisoned lock
//! ([`lock_or_recovered`] returns whether a poison was seen, so the caller can
//! mark the subsystem failed instead of continuing).
//!
//! # Logging
//!
//! A poisoned lock stays poisoned for the rest of the process's life, so a
//! per-acquisition `error!` would flood the log. Each helper therefore
//! records that it has warned **per lock instance** (keyed by the lock's
//! address) and logs `error!` only the first time that instance is seen
//! poisoned; later acquisitions of the same lock log at `debug!`.

use std::collections::VecDeque;
use std::sync::{
    Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
};

/// How many recently-reported lock addresses to remember. Bounded so the set
/// cannot grow without limit over a long-lived process, and small enough that
/// a recycled address (a dropped lock's memory reused by a new one) is very
/// likely to have aged out and log again rather than have its first report
/// suppressed. A per-lock `AtomicBool` would be exact, but these helpers take
/// a bare `&Mutex<T>`/`&RwLock<T>` (so every call site keeps using `std`'s
/// types) and cannot carry one; the ring is the next best thing (item 7).
const WARNED_RING: usize = 64;

/// Ring of lock instances already reported as poisoned, so the `error!` is
/// logged once per recently-seen lock rather than on every acquisition (a
/// poison is permanent).
fn warned() -> &'static Mutex<VecDeque<usize>> {
    static WARNED: OnceLock<Mutex<VecDeque<usize>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(VecDeque::with_capacity(WARNED_RING)))
}

/// Record that `addr` has been reported; `true` the first time it is seen
/// within the ring.
fn first_warning(addr: usize) -> bool {
    // This lock never crosses an `.await` and is held only for a deque
    // operation, so a poisoned `warned()` itself is irrelevant — recover
    // rather than recurse.
    let mut ring = match warned().lock() {
        Ok(ring) => ring,
        Err(e) => e.into_inner(),
    };
    if ring.contains(&addr) {
        return false;
    }
    if ring.len() == WARNED_RING {
        ring.pop_front();
    }
    ring.push_back(addr);
    true
}

/// Lock `m`, recovering from poisoning. The returned bool is `true` when
/// recovery actually happened (a poison was observed) — a caller guarding
/// multi-step state must fail closed on `true` rather than continuing (see
/// the module doc).
pub(crate) fn lock_or_recovered<T>(m: &Mutex<T>) -> (MutexGuard<'_, T>, bool) {
    match m.lock() {
        Ok(guard) => (guard, false),
        Err(e) => {
            let addr = m as *const Mutex<T> as usize;
            if first_warning(addr) {
                tracing::error!(
                    "recovering from a poisoned mutex (state is structurally consistent)"
                );
            } else {
                tracing::debug!("recovering from an already-reported poisoned mutex");
            }
            (e.into_inner(), true)
        }
    }
}

/// Lock `m`, recovering from poisoning rather than panicking — for state
/// whose every mutation is a single panic-free assignment (see the module
/// doc).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    lock_or_recovered(m).0
}

/// Take `rw`'s read guard, recovering from poisoning rather than panicking.
pub(crate) fn read<T>(rw: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    rw.read().unwrap_or_else(|e: PoisonError<_>| {
        let addr = rw as *const RwLock<T> as usize;
        if first_warning(addr) {
            tracing::error!("recovering from a poisoned RwLock (state is structurally consistent)");
        } else {
            tracing::debug!("recovering from an already-reported poisoned RwLock");
        }
        e.into_inner()
    })
}

/// Take `rw`'s write guard, recovering from poisoning rather than panicking.
pub(crate) fn write<T>(rw: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    rw.write().unwrap_or_else(|e: PoisonError<_>| {
        let addr = rw as *const RwLock<T> as usize;
        if first_warning(addr) {
            tracing::error!("recovering from a poisoned RwLock (state is structurally consistent)");
        } else {
            tracing::debug!("recovering from an already-reported poisoned RwLock");
        }
        e.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_poisoned_mutex_is_still_readable_and_writable() {
        let m = Arc::new(Mutex::new(1u32));
        let m2 = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let _guard = m2.lock().unwrap();
            panic!("poison the mutex while holding the guard");
        })
        .join();

        // The pre-fix `.lock().unwrap()` would panic here, and on every later call.
        assert_eq!(*lock(&m), 1);
        *lock(&m) = 2;
        assert_eq!(*lock(&m), 2);
    }

    #[test]
    fn a_poisoned_rwlock_is_still_readable_and_writable() {
        let rw = Arc::new(RwLock::new(7u32));
        let rw2 = Arc::clone(&rw);
        let _ = std::thread::spawn(move || {
            let _guard = rw2.write().unwrap();
            panic!("poison the rwlock while holding the write guard");
        })
        .join();

        assert_eq!(*read(&rw), 7);
        *write(&rw) = 8;
        assert_eq!(*read(&rw), 8);
    }
}
