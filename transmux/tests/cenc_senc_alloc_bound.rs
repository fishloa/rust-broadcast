//! `senc` alloc-DoS bound — the *allocation* half of the regression proof.
//!
//! `SampleEncryptionBox::parse_body` reads `sample_count` as a 32-bit field
//! straight off the wire from third-party media, and it used to size
//! `Vec::with_capacity(sample_count)` from it before any bounds check. A
//! 20-byte box carrying `FF FF FF FF` therefore asked the allocator for ~4.3
//! billion `SampleEncryptionEntry`s — on the order of 206 GB — from a single
//! remote input.
//!
//! An in-crate unit test cannot see that: the oversized `Vec` is created, the
//! per-entry length checks then fail, and the call still returns
//! `BufferTooShort`. On a platform that reserves address space lazily (macOS,
//! or Linux with overcommit) the huge request does not even abort — it just
//! consumes the machine. So this file installs a recording
//! [`GlobalAlloc`](core::alloc::GlobalAlloc) and asserts on the **largest
//! single allocation** the parse asks for. That is the property under test, and
//! it fails loudly if the bound is ever removed.
//!
//! A global allocator applies to its whole test binary, which is why this lives
//! in its own file (mirroring `tests/alloc_measurement.rs`'s reason for the same
//! pattern).

#![cfg(feature = "cenc")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use transmux::{SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION, SampleEncryptionBox};

/// Largest single allocation size seen while [`ARMED`] is set.
static MAX_ALLOC: AtomicUsize = AtomicUsize::new(0);
/// Whether [`RecordingAlloc`] is recording. Off outside the measured window so
/// the test harness's own allocations are not attributed to the parse.
static ARMED: AtomicBool = AtomicBool::new(false);

struct RecordingAlloc;

unsafe impl GlobalAlloc for RecordingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            MAX_ALLOC.fetch_max(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            MAX_ALLOC.fetch_max(new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: RecordingAlloc = RecordingAlloc;

/// Ceiling on any single allocation the parse of a **20-byte** `senc` may make.
/// Generous by four orders of magnitude over anything legitimate (a well-formed
/// 20-byte box needs a handful of small `Vec`s), and eight orders below the
/// ~206 GB a `sample_count`-sized `Vec::with_capacity` asks for — so this
/// catches the regression without being brittle about allocator bookkeeping.
const MAX_SINGLE_ALLOC: usize = 1 << 20; // 1 MiB

/// A `senc` body declaring `sample_count == 0xFFFFFFFF` in 20 bytes total.
fn hostile_body() -> Vec<u8> {
    let mut body = vec![0xFFu8; 4];
    body.extend_from_slice(&[0u8; 16]);
    body
}

#[test]
fn hostile_senc_sample_count_allocates_nothing_large() {
    // Every (per_sample_iv_size, flags) shape that reaches the entry loop.
    for (iv_size, flags) in [
        (8u8, 0u32),
        (8, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
        (16, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
        (0, SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION),
        (0, 0),
    ] {
        let body = hostile_body();
        MAX_ALLOC.store(0, Ordering::Relaxed);
        ARMED.store(true, Ordering::Relaxed);
        let result = SampleEncryptionBox::parse_body(&body, 0, flags, iv_size);
        ARMED.store(false, Ordering::Relaxed);
        let max_alloc = MAX_ALLOC.load(Ordering::Relaxed);

        assert!(
            result.is_err(),
            "iv_size={iv_size} flags={flags:#x}: a 20-byte senc declaring 0xFFFFFFFF samples \
             must be rejected"
        );
        assert!(
            max_alloc <= MAX_SINGLE_ALLOC,
            "iv_size={iv_size} flags={flags:#x}: parsing a 20-byte senc requested a single \
             {max_alloc}-byte allocation (limit {MAX_SINGLE_ALLOC}) — the untrusted sample_count \
             is being used to size a Vec before it is bounded against the box length"
        );
    }
}

/// The bound must not cost well-formed boxes anything: a body exactly sized for
/// its declared entries parses, and allocates in proportion to its real size.
#[test]
fn well_formed_senc_still_parses_within_the_bound() {
    const SAMPLES: usize = 4;
    let mut body = (SAMPLES as u32).to_be_bytes().to_vec();
    for i in 0..SAMPLES {
        body.extend_from_slice(&[i as u8; 8]);
    }

    MAX_ALLOC.store(0, Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    let parsed = SampleEncryptionBox::parse_body(&body, 0, 0, 8);
    ARMED.store(false, Ordering::Relaxed);
    let max_alloc = MAX_ALLOC.load(Ordering::Relaxed);

    let parsed = parsed.expect("a well-formed senc must still parse");
    assert_eq!(parsed.entries.len(), SAMPLES);
    assert!(
        max_alloc <= MAX_SINGLE_ALLOC,
        "a {}-byte senc requested a single {max_alloc}-byte allocation",
        body.len()
    );
}
