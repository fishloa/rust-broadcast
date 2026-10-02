//! Hostile-count allocation bounds (audit r10-O-6): an APDU whose wire count
//! claims far more entries than its body holds must be rejected without ever
//! allocating for the claimed count. A counting global allocator records the
//! largest single allocation, so the bound is deterministic (no timing, no
//! RSS sampling).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use broadcast_common::Parse;
use dvb_ci::ci_plus::cicam_player::PlayerCapabilitiesReply;

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
fn capabilities_reply_count_does_not_drive_allocation() {
    // CICAM_player_capabilities_reply = 9F A0 03, 1-byte length 2, then a
    // 16-bit count of 0xFFFF with no entries following.
    let apdu = [0x9F, 0xA0, 0x03, 0x02, 0xFF, 0xFF];
    LARGEST.store(0, Ordering::Relaxed);
    let r = PlayerCapabilitiesReply::parse(&apdu);
    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(r.is_err(), "a count with no entries must be rejected");
    // 0xFFFF entries x 2 bytes = 131 070 bytes if the count were trusted.
    assert!(
        largest < 1024,
        "largest single allocation was {largest} bytes: the wire count drove it"
    );
}
