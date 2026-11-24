//! Counting / capping `GlobalAlloc` wrappers for allocation-bound tests.
//!
//! Eight test files used to each carry a hand-copied `unsafe impl GlobalAlloc`
//! (RUST_AUDIT_REPORT U7). This crate is the single audited copy. Every impl
//! forwards each call unchanged to [`System`] and only does safe bookkeeping
//! around it, so the whole `unsafe` surface is the delegating calls below.
//!
//! Install one as the test binary's allocator:
//!
//! ```ignore
//! #[global_allocator]
//! static A: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();
//! ```
//!
//! Pick the scoping the test needs:
//!
//! * [`ProcessCounting`] — process-global (all threads). For a test binary
//!   with a single `#[test]`, since a sibling test would pollute it.
//! * [`ProcessPeak`] — process-global live bytes plus their high-water mark, so
//!   a test can bound a writer's whole footprint, not one allocation.
//! * [`ThreadCounting`] — per-thread counters, so parallel `#[test]` fns
//!   cannot pollute each other's measurement window.
//! * [`ThreadCapped`] — per-thread live-byte budget; an allocation that would
//!   exceed it returns null (the process aborts) instead of exhausting RAM.
//!
//! Per-thread state is `thread_local!` with a `const { Cell::new(0) }`
//! initializer and no `Drop`, so touching it from inside `GlobalAlloc::alloc`
//! cannot allocate re-entrantly.
//!
//! Dev-dependency only (path, `publish = false`): never published.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Process-global counter (optionally gated, see [`set_armed`](Self::set_armed)): number of `alloc`/`realloc` calls and the largest
/// single requested size (the new size is what is recorded for `realloc`).
#[derive(Debug)]
pub struct ProcessCounting {
    allocs: AtomicUsize,
    largest: AtomicUsize,
    armed: AtomicBool,
}

impl ProcessCounting {
    /// A zeroed counter, usable in a `static`.
    pub const fn new() -> Self {
        Self {
            allocs: AtomicUsize::new(0),
            largest: AtomicUsize::new(0),
            armed: AtomicBool::new(true),
        }
    }

    /// Zero the largest-single-allocation high-water mark.
    pub fn reset_largest(&self) {
        self.largest.store(0, Ordering::SeqCst);
    }

    /// Largest single `alloc`/`realloc` size seen since the last
    /// [`reset_largest`](Self::reset_largest).
    pub fn largest(&self) -> usize {
        self.largest.load(Ordering::SeqCst)
    }

    /// Total `alloc`/`realloc` calls since process start.
    pub fn alloc_count(&self) -> usize {
        self.allocs.load(Ordering::SeqCst)
    }

    /// Run `f`, returning `(allocation_calls_during_f, result)`.
    pub fn allocs_during<R>(&self, f: impl FnOnce() -> R) -> (usize, R) {
        let before = self.alloc_count();
        let r = f();
        (self.alloc_count() - before, r)
    }

    /// Gate recording on or off (on by default). Lets a test exclude the
    /// harness's own allocations outside the measured window.
    pub fn set_armed(&self, armed: bool) {
        self.armed.store(armed, Ordering::SeqCst);
    }

    fn record(&self, size: usize) {
        if !self.armed.load(Ordering::Relaxed) {
            return;
        }
        self.allocs.fetch_add(1, Ordering::SeqCst);
        self.largest.fetch_max(size, Ordering::SeqCst);
    }
}

impl Default for ProcessCounting {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; only atomic counters are touched around it.
unsafe impl GlobalAlloc for ProcessCounting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        self.record(l.size());
        // SAFETY: same `Layout` the caller gave us; the caller's contract passes through.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: `p`/`l` came from a prior call on this allocator, i.e. `System`.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        self.record(n);
        // SAFETY: `p`/`l` came from this allocator (i.e. `System`); `n` is the caller's.
        unsafe { System.realloc(p, l, n) }
    }
}

/// Process-global **live bytes** and their high-water mark (all threads).
///
/// A test that asserts "this writer never holds more than N bytes" needs the
/// whole footprint across every thread the writer uses (a muxer, a buffered
/// file writer and a demuxer can sit on different threads), which
/// [`ProcessCounting`] does not track. The counters are process-wide, so tests
/// that each measure a window must serialise those windows themselves (a
/// `static Mutex` is enough).
#[derive(Debug)]
pub struct ProcessPeak {
    live: AtomicUsize,
    peak: AtomicUsize,
}

impl ProcessPeak {
    /// A zeroed tracker, usable in a `static`.
    pub const fn new() -> Self {
        Self {
            live: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// Lower the high-water mark to the current live total, so
    /// [`peak`](Self::peak) reports from this point on. Outstanding allocations
    /// are still counted, so live bytes are never misreported.
    pub fn reset(&self) {
        self.peak.store(self.live(), Ordering::SeqCst);
    }

    /// Bytes currently allocated and not yet freed, process-wide.
    pub fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// Largest live total seen since process start or the last
    /// [`reset`](Self::reset).
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    /// Run `f`, returning `(bytes_above_entry, result)`: the high-water mark of
    /// live bytes during `f` over the live total at entry. A writer that
    /// allocates nothing above what was already held reports `0`.
    pub fn peak_during<R>(&self, f: impl FnOnce() -> R) -> (usize, R) {
        let entry = self.live();
        self.peak.store(entry, Ordering::SeqCst);
        let result = f();
        (
            self.peak.load(Ordering::SeqCst).saturating_sub(entry),
            result,
        )
    }

    fn record_alloc(&self, size: usize) {
        let live = self
            .live
            .fetch_add(size, Ordering::SeqCst)
            .saturating_add(size);
        self.peak.fetch_max(live, Ordering::SeqCst);
    }

    fn record_free(&self, size: usize) {
        self.live.fetch_sub(size, Ordering::SeqCst);
    }
}

impl Default for ProcessPeak {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; only atomic counters are touched around it.
unsafe impl GlobalAlloc for ProcessPeak {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: same `Layout` the caller gave us; the caller's contract passes through.
        let ptr = unsafe { System.alloc(l) };
        if !ptr.is_null() {
            self.record_alloc(l.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        self.record_free(l.size());
        // SAFETY: `p`/`l` came from a prior call on this allocator, i.e. `System`.
        unsafe { System.dealloc(p, l) }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // SAFETY: `p`/`l` came from this allocator (i.e. `System`); `n` is the caller's.
        let ptr = unsafe { System.realloc(p, l, n) };
        if !ptr.is_null() {
            self.record_alloc(n);
            self.record_free(l.size());
        }
        ptr
    }
}

thread_local! {
    static T_ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    static T_ALLOC_BYTES: Cell<usize> = const { Cell::new(0) };
    static T_DEALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    static T_LIVE_BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Counters for one thread; see [`ThreadCounting::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// `alloc` calls on this thread (a default `realloc` counts here too).
    pub allocs: usize,
    /// Sum of requested sizes of those `alloc` calls.
    pub bytes: usize,
    /// `dealloc` calls on this thread.
    pub deallocs: usize,
}

/// Per-thread allocation counter: counts only allocations made by the calling
/// thread. `realloc` is deliberately NOT overridden, so the `GlobalAlloc`
/// default (`alloc` new + copy + `dealloc` old) applies and a `realloc` shows
/// up as one alloc of the new size plus one dealloc — the semantics the
/// recorded transmux budgets were measured with.
#[derive(Debug, Default)]
pub struct ThreadCounting;

impl ThreadCounting {
    /// A counter, usable in a `static`.
    pub const fn new() -> Self {
        Self
    }

    /// Zero this thread's counters.
    pub fn reset() {
        T_ALLOC_COUNT.with(|c| c.set(0));
        T_ALLOC_BYTES.with(|c| c.set(0));
        T_DEALLOC_COUNT.with(|c| c.set(0));
    }

    /// Read this thread's counters.
    pub fn snapshot() -> Snapshot {
        Snapshot {
            allocs: T_ALLOC_COUNT.with(Cell::get),
            bytes: T_ALLOC_BYTES.with(Cell::get),
            deallocs: T_DEALLOC_COUNT.with(Cell::get),
        }
    }

    /// Reset, run `f`, and return `(result, snapshot)` for this thread.
    pub fn measure<R>(f: impl FnOnce() -> R) -> (R, Snapshot) {
        Self::reset();
        let r = f();
        (r, Self::snapshot())
    }
}

// SAFETY: forwards unchanged to `System`; only thread-local `Cell` bookkeeping.
unsafe impl GlobalAlloc for ThreadCounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        T_ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        T_ALLOC_BYTES.with(|c| c.set(c.get() + layout.size()));
        // SAFETY: the caller's `Layout` contract passes through to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        T_DEALLOC_COUNT.with(|c| c.set(c.get() + 1));
        // SAFETY: `ptr`/`layout` came from this allocator, i.e. `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Per-thread live-byte budget of `BUDGET` bytes. An `alloc`/`realloc` that
/// would push this thread's live total over it returns null (so the process
/// aborts) rather than exhausting the host — safe to run against a hostile
/// input before the bound under test exists.
///
/// `dealloc` saturates at zero: the test harness moves results between pool
/// threads, so a buffer can be freed on a thread whose counter never saw it.
#[derive(Debug, Default)]
pub struct ThreadCapped<const BUDGET: usize>;

impl<const BUDGET: usize> ThreadCapped<BUDGET> {
    /// An allocator, usable in a `static`.
    pub const fn new() -> Self {
        Self
    }
}

// SAFETY: forwards unchanged to `System` (or returns null, which the contract
// allows); only thread-local `Cell` bookkeeping around it.
unsafe impl<const BUDGET: usize> GlobalAlloc for ThreadCapped<BUDGET> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = T_LIVE_BYTES.with(Cell::get);
        if live.saturating_add(layout.size()) > BUDGET {
            return null_mut();
        }
        // SAFETY: the caller's `Layout` contract passes through to `System`.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            T_LIVE_BYTES.with(|c| c.set(live + layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        T_LIVE_BYTES.with(|c| c.set(c.get().saturating_sub(layout.size())));
        // SAFETY: `ptr`/`layout` came from this allocator, i.e. `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let live = T_LIVE_BYTES.with(Cell::get);
        let net_live = live.saturating_sub(layout.size()).saturating_add(new_size);
        if net_live > BUDGET {
            return null_mut();
        }
        // SAFETY: `ptr`/`layout` came from this allocator; `new_size` is the caller's.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            T_LIVE_BYTES.with(|c| c.set(net_live));
        }
        new_ptr
    }
}
