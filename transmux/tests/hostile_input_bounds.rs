//! Capped-allocation regression harness for hostile wire counts (P0 batch B,
//! items 3, 8 and 9).
//!
//! Every test here feeds a tiny fixture (tens of bytes to ~40 KB) whose
//! `u32` count fields are set to `0xFFFFFFFF`, which the pre-fix parsers used
//! directly as an iteration/allocation budget. A `#[global_allocator]` wraps
//! `System` and accounts live bytes **per thread** against a 64 MiB budget;
//! an allocation that would exceed it returns null, so the process aborts
//! immediately instead of eating the machine's memory. That makes these tests
//! safe to run BEFORE the fix — they abort (SIGABRT) where the pre-fix code
//! would have requested gigabytes.
//!
//! The per-thread accounting pattern (`thread_local!` with a `const { Cell::new(0) }`
//! initializer, so touching it from inside `GlobalAlloc::alloc` cannot
//! re-entrantly allocate) is taken from `tests/alloc_measurement.rs`; see that
//! module's docs for the full reasoning.

#![cfg(feature = "cenc")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr::null_mut;

use broadcast_common::{Parse, Unpackage};
use transmux::ProgressiveDemux;
use transmux::cenc_decrypt::CencDecryptor;
use transmux::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox, SgpdEntry};

/// Per-thread live-allocation budget. Generous versus every legitimate path
/// exercised below (the fixtures are tens of KB), and 2–3 orders of magnitude
/// below the ~17 GB / ~34 GB requests a hostile count provokes — so a bound
/// regression aborts here rather than swapping the machine.
const BUDGET: usize = 64 << 20;

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
        // `saturating_sub`: the test harness moves results (e.g. a parsed
        // `Media`) between pool threads, so a buffer allocated on one thread
        // can be freed on another whose counter never saw those bytes.
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

// ---------------------------------------------------------------------------
// Fixture patching helpers
// ---------------------------------------------------------------------------

const CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl"];

/// Recursively locate a box of `fourcc` within `[lo, hi)`. Returns
/// `(abs_offset, box_size)`. Descends into known container boxes.
fn find_box_range(data: &[u8], lo: usize, hi: usize, fourcc: &[u8; 4]) -> Option<(usize, usize)> {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        if &t == fourcc {
            return Some((off, size));
        }
        if CONTAINERS.contains(&&t)
            && let Some(found) = find_box_range(data, off + 8, off + size, fourcc)
        {
            return Some(found);
        }
        off += size;
    }
    None
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

/// Copy `fixture`, then overwrite the 4 bytes at `start + offset_in_box` of
/// the first box named `fourcc` with `value`. Panics if the box is absent —
/// a fixture that stops carrying the box must fail these tests loudly.
fn patch_fixture(
    fixture_path: &str,
    fourcc: &[u8; 4],
    offset_in_box: usize,
    value: u32,
) -> Vec<u8> {
    let mut file = fixture(fixture_path);
    let (start, size) = find_box_range(&file, 0, file.len(), fourcc).unwrap_or_else(|| {
        panic!(
            "fixture {fixture_path} must contain a {} box",
            String::from_utf8_lossy(fourcc)
        )
    });
    assert!(size >= offset_in_box + 4, "box too small to patch");
    let at = start + offset_in_box;
    file[at..at + 4].copy_from_slice(&value.to_be_bytes());
    file
}

const PROG_MP4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/transmux/h264_aac_prog.mp4"
);
const CENC_MP4: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/cenc.mp4");

/// `stts`/`ctts`: header(8) + version/flags(4) + entry_count(4) -> first
/// entry's `sample_count`.
const RUN_LENGTH_ENTRY_COUNT_OFFSET: usize = 16;
/// `stsz`: header(8) + version/flags(4) -> `sample_size`.
const STSZ_SAMPLE_SIZE_OFFSET: usize = 12;
/// `stsz`: ... + `sample_size`(4) -> `sample_count`.
const STSZ_SAMPLE_COUNT_OFFSET: usize = 16;
/// `stco`: header(8) + version/flags(4) -> `entry_count`.
const STCO_ENTRY_COUNT_OFFSET: usize = 12;

// ---------------------------------------------------------------------------
// Item 3 — progressive demux stts/ctts run expansion (r04-C5)
// ---------------------------------------------------------------------------

/// Patch the first `stts` entry's `sample_count` to `0xFFFFFFFF` in the real
/// progressive fixture. Pre-fix, `expand_stts` pushed 4.29 billion u32s
/// (~17 GB) before ever comparing against `total_samples`, and this test
/// aborted under the capped allocator. Post-fix the run is rejected against
/// the space left: the hostile track is skipped (or the whole parse errors),
/// never silently accepted with samples attached.
#[test]
fn hostile_stts_run_count_is_rejected() {
    let file = patch_fixture(PROG_MP4, b"stts", RUN_LENGTH_ENTRY_COUNT_OFFSET, u32::MAX);
    let mut demux = ProgressiveDemux::new(file.len() + 1024).expect("non-zero cap");
    match demux.unpackage(&file) {
        Err(_) => {}
        Ok(media) => assert!(
            media.skipped.iter().any(|s| s.reason.contains("stts")),
            "a stts declaring a 0xFFFFFFFF run must not be silently accepted: \
             tracks={:?} skipped={:?}",
            media.tracks.len(),
            media.skipped
        ),
    }
}

/// Same for `ctts`: one entry with `sample_count = 0xFFFFFFFF` used to make
/// `expand_ctts` push ~17 GB of i32 offsets from a valid-looking box.
#[test]
fn hostile_ctts_run_count_is_rejected() {
    let file = patch_fixture(PROG_MP4, b"ctts", RUN_LENGTH_ENTRY_COUNT_OFFSET, u32::MAX);
    let mut demux = ProgressiveDemux::new(file.len() + 1024).expect("non-zero cap");
    match demux.unpackage(&file) {
        Err(_) => {}
        Ok(media) => assert!(
            media.skipped.iter().any(|s| s.reason.contains("ctts")),
            "a ctts declaring a 0xFFFFFFFF run must not be silently accepted: \
             tracks={:?} skipped={:?}",
            media.tracks.len(),
            media.skipped
        ),
    }
}

// ---------------------------------------------------------------------------
// Item 8 — CencDecryptor progressive stsz/stco (r05-C4)
// ---------------------------------------------------------------------------

/// The item's hostile shape: `stsz` with `sample_size = 1`,
/// `sample_count = 0xFFFFFFFF`. Pre-fix, `stsz_sizes` ran
/// `Vec::with_capacity(count)` before any length check and the push loop was
/// entirely unbounded for a non-zero `sample_size`; under the capped
/// allocator this aborted. Post-fix both calls return `Err`.
#[test]
fn hostile_stsz_sample_count_is_rejected() {
    let mut file = fixture(CENC_MP4);
    let (start, size) =
        find_box_range(&file, 0, file.len(), b"stsz").expect("cenc.mp4 must contain a stsz");
    assert!(size >= STSZ_SAMPLE_COUNT_OFFSET + 4);
    file[start + STSZ_SAMPLE_SIZE_OFFSET..start + STSZ_SAMPLE_SIZE_OFFSET + 4]
        .copy_from_slice(&1u32.to_be_bytes());
    file[start + STSZ_SAMPLE_COUNT_OFFSET..start + STSZ_SAMPLE_COUNT_OFFSET + 4]
        .copy_from_slice(&u32::MAX.to_be_bytes());

    let dec = CencDecryptor::from_fmp4(&file).expect("harvest does not read stsz");
    let res = dec.demux();
    assert!(
        res.is_err(),
        "a ~24 KB file whose stsz declares 0xFFFFFFFF samples must be Err, got Ok with \
         {} track(s)",
        match res {
            Ok(m) => m.tracks.len(),
            Err(_) => unreachable!(),
        }
    );
}

/// `stco` with `entry_count = 0xFFFFFFFF`: pre-fix the capacity request
/// (~32 GB of u64s) came before the table-length check and aborted at once.
#[test]
fn hostile_stco_entry_count_is_rejected() {
    let file = patch_fixture(CENC_MP4, b"stco", STCO_ENTRY_COUNT_OFFSET, u32::MAX);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest does not read stco");
    assert!(
        dec.demux().is_err(),
        "a stco declaring 0xFFFFFFFF chunk offsets must be Err"
    );
}

// ---------------------------------------------------------------------------
// Item 9 — sgpd v0 non-roll zero-length entries (r05-C10)
// ---------------------------------------------------------------------------

/// Wrap a body in an `sgpd` box header.
fn sgpd_box(body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    bytes.extend_from_slice(b"sgpd");
    bytes.extend_from_slice(body);
    bytes
}

/// The item's 24-byte hostile box: version 0, `grouping_type = 'seig'`,
/// `entry_count = 0xFFFFFFFF`, no entry bytes. Pre-fix the v0 non-roll path
/// computed `entry_len = body.len() - c` — zero after the first iteration —
/// and pushed an empty `SgpdEntry::Unknown` for all 4.29 billion remaining
/// iterations, aborting under the capped allocator. Post-fix: `Err`.
#[test]
fn hostile_sgpd_zero_length_entries_are_rejected() {
    let mut body = vec![0u8; 4]; // version 0 + flags
    body.extend_from_slice(&GROUPING_TYPE_SEIG.to_be_bytes());
    body.extend_from_slice(&u32::MAX.to_be_bytes()); // entry_count
    let bytes = sgpd_box(&body);

    let res = SampleGroupDescriptionBox::parse(&bytes);
    assert!(
        res.is_err(),
        "a 20-byte v0 sgpd declaring 0xFFFFFFFF entries must be Err, got {res:?}"
    );
}

/// Regression: a legitimate version-0 non-roll `sgpd` with exactly ONE entry
/// (the deprecated "rest of the body is one blob" shape) still parses.
#[test]
fn v0_single_entry_sgpd_still_parses() {
    let mut body = vec![0u8; 4]; // version 0 + flags
    body.extend_from_slice(&GROUPING_TYPE_SEIG.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    body.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]); // the single blob entry
    let bytes = sgpd_box(&body);

    let parsed = SampleGroupDescriptionBox::parse(&bytes)
        .expect("a legitimate v0 non-roll sgpd with one entry must parse");
    assert_eq!(parsed.version, 0);
    assert_eq!(parsed.grouping_type, GROUPING_TYPE_SEIG);
    assert_eq!(
        parsed.entries,
        vec![SgpdEntry::Unknown(vec![0x11, 0x22, 0x33, 0x44])]
    );
}
