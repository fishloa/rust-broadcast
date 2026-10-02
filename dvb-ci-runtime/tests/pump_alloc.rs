//! r10-O-11: `Driver::pump` must not copy each received frame before handing
//! it to the stack. A counting global allocator records the largest single
//! allocation made during one `pump` of a full-size junk frame, so the check
//! is deterministic (no timing).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dvb_ci_runtime::{Driver, MockCaDevice};

struct Counting;
static LARGEST: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call unchanged to `System`; only records a size.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LARGEST.fetch_max(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LARGEST.fetch_max(n, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn pump_does_not_copy_the_received_frame() {
    const FRAME: usize = 4096; // the driver's whole receive buffer
    let mock = MockCaDevice::new([vec![0u8; FRAME]]);
    let mut driver = Driver::new(mock);
    LARGEST.store(0, Ordering::Relaxed);
    let _ = driver.pump(Duration::from_millis(1));
    let largest = LARGEST.load(Ordering::Relaxed);
    eprintln!("largest single allocation during pump: {largest} bytes");
    assert!(
        largest < FRAME / 2,
        "largest single allocation was {largest} bytes: the frame was copied"
    );
}
