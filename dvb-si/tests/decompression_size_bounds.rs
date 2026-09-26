//! Capped-allocation regression harness for the `decompress_zlib` output
//! limit (dvb-si P0 item 1).
//!
//! `decompress_zlib` decompresses a `compressed_module_descriptor` zlib
//! stream (TR 101 202 §4.6.6.10 / EN 301 192 §10.2.11) taken from broadcast
//! object-carousel data. Before this crate's size cap it called
//! `read_to_end` with no output limit, so a small compressed stream of
//! repetitive bytes (zeros compress extremely well) could force an
//! allocation orders of magnitude larger than the input.
//!
//! A `#[global_allocator]` wraps `System` and accounts live bytes **per
//! thread** against a 256 MiB budget; an allocation that would exceed it
//! returns null, so the process aborts immediately instead of eating the
//! machine's memory. That makes this test safe to run against an unbounded
//! decompressor too — it aborts (SIGABRT) rather than requesting the
//! multi-gigabyte output a small compressed input can otherwise produce.
//!
//! The per-thread accounting pattern (`thread_local!` with a
//! `const { Cell::new(0) }` initializer, so touching it from inside
//! `GlobalAlloc::alloc` cannot re-entrantly allocate) is copied from
//! `transmux`'s equivalent capped-allocation test harness.

#![cfg(feature = "flate2")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr::null_mut;

use dvb_si::carousel::biop::message::{decompress_zlib, decompress_zlib_bounded};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

/// Per-thread live-allocation budget. Well above every legitimate path here
/// (the compressed input and the bounded output are both a few MiB) and far
/// below the output an unbounded `read_to_end` would request for the
/// oversized fixture below, so a bound regression aborts under this cap
/// rather than exhausting the host.
const BUDGET: usize = 256 << 20;

thread_local! {
    /// Bytes currently allocated by this thread (`alloc`/`realloc` add,
    /// `dealloc` subtracts).
    static LIVE_BYTES: Cell<usize> = const { Cell::new(0) };
}

struct CappedAlloc;

unsafe impl GlobalAlloc for CappedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE_BYTES.with(Cell::get);
        if live.saturating_add(layout.size()) > BUDGET {
            return null_mut();
        }
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.with(|c| c.set(live + layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.with(|c| c.set(c.get().saturating_sub(layout.size())));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let live = LIVE_BYTES.with(Cell::get);
        let net_live = live.saturating_sub(layout.size()).saturating_add(new_size);
        if net_live > BUDGET {
            return null_mut();
        }
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            LIVE_BYTES.with(|c| c.set(net_live));
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL: CappedAlloc = CappedAlloc;

/// Build a zlib stream that decompresses to `total_len` zero bytes, without
/// ever materializing `total_len` bytes in memory at once (that would itself
/// trip the capped allocator during *fixture construction*, not the code
/// under test). Feeds the encoder a small reused chunk repeatedly instead.
fn compressed_zeros(total_len: usize) -> Vec<u8> {
    const CHUNK: usize = 64 * 1024;
    let chunk = [0u8; CHUNK];
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    let mut written = 0usize;
    while written < total_len {
        let n = CHUNK.min(total_len - written);
        encoder.write_all(&chunk[..n]).unwrap();
        written += n;
    }
    encoder.finish().unwrap()
}

/// A small (well under 1 MiB) compressed stream of zeros that inflates to
/// 100 MiB — comfortably past `MAX_DECOMPRESSED_MODULE_SIZE` (64 MiB) but
/// comfortably under this harness's 256 MiB budget too, so an unbounded
/// decompressor allocates the full output successfully here (demonstrating
/// the unbounded accept, not just a capped-allocator abort) while the
/// size-capped one is rejected by the explicit check rather than by memory
/// pressure. This is the finding's scenario at a scale this harness can
/// observe both ways: about 1 MiB of zlib-compressed zeros forcing an
/// output far larger than any real broadcast carousel module.
#[test]
fn compressed_zeros_output_is_bounded() {
    const INFLATED_LEN: usize = 100 * 1024 * 1024;
    let compressed = compressed_zeros(INFLATED_LEN);
    // Sanity: this is genuinely a small stream forcing a much larger output,
    // not a fixture that just happens to be large itself.
    assert!(
        compressed.len() < 1024 * 1024,
        "fixture must compress to well under 1 MiB, got {} bytes",
        compressed.len()
    );

    let result = decompress_zlib(&compressed);
    assert!(
        result.is_err(),
        "a compressed stream inflating to {INFLATED_LEN} bytes (past \
         MAX_DECOMPRESSED_MODULE_SIZE = 64 MiB) must return Err, not silently \
         allocate and return the full output"
    );
}

/// Same fixture through the general bounded entry point with an explicit,
/// smaller cap — exercises the form a caller would use with a
/// `carousel_identifier_descriptor`-declared `OriginalSize`.
#[test]
fn compressed_zeros_bounded_explicit_cap_is_rejected() {
    let compressed = compressed_zeros(100 * 1024 * 1024);
    let result = decompress_zlib_bounded(&compressed, 1024 * 1024);
    assert!(
        result.is_err(),
        "output exceeding an explicit 1 MiB cap must return Err"
    );
}

/// A small legitimate stream still decompresses correctly under the real
/// default cap — the limit doesn't reject ordinary carousel modules.
#[test]
fn legitimate_small_module_still_decompresses() {
    let original = b"Hello, compressed BIOP world! ".repeat(100);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&original).unwrap();
    let compressed = encoder.finish().unwrap();

    let decompressed =
        decompress_zlib(&compressed).expect("small legitimate module must decompress");
    assert_eq!(decompressed, original);
}
