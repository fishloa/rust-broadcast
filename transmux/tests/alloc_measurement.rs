//! Allocation-count measurement for the CENC encrypt path (media plane step 2b,
//! G12 — `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
//!
//! The spec explicitly admits that `Sample.data: Bytes` (immutable/shared) may
//! cost extra allocations on the encrypt/decrypt in-place rewrite paths versus
//! the old `Vec<u8>`, and says the design "owes a measurement rather than an
//! assumption". This file is that measurement: a counting `#[global_allocator]`
//! instruments `CencEncryptor::encrypt` over the real committed fixture
//! (`fixtures/ts/h264/main.ts`), isolated to just the `encrypt()` call (counters
//! are reset immediately before it and read immediately after, so fixture
//! load/demux overhead is excluded).
//!
//! Deliberately public-API-only (same fixture/config shape as
//! `tests/cenc_encrypt.rs`) so this file compiles unchanged against BOTH the
//! pre-refactor (`Sample.data: Vec<u8>`) and post-refactor (`Bytes`) checkouts
//! — the BEFORE numbers in the media-plane-step-2b report were captured by
//! running this exact file (via `git stash`) against the pre-refactor tree.
//! Run with `--nocapture` to see the numbers on stderr.

#![cfg(feature = "cenc")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use broadcast_common::{Encrypt, Unpackage};
use transmux::{
    CencEncryptor, CencScheme, CodecConfig, EncryptConfig, IvGen, Media, SubsamplePolicy, TsDemux,
};

/// Counts every allocation/deallocation the process makes, globally. Isolated
/// to one call by resetting the counters immediately before it.
struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
static DEALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn reset_counters() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    DEALLOC_COUNT.store(0, Ordering::Relaxed);
}

fn snapshot_counters() -> (usize, usize, usize) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
        DEALLOC_COUNT.load(Ordering::Relaxed),
    )
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts")
}

/// The real cleartext H.264 capture (same fixture `cenc_encrypt.rs`'s unit
/// tests use), narrowed to its single AVC video track. `None` if the fixture
/// is absent (skip cleanly, matching the other CENC test files' convention).
fn clear_video_media() -> Option<Media> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!("alloc_measurement: SKIPPED — {path:?} not found.");
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut demux = TsDemux::new();
    let media = demux.unpackage(bytes.as_slice()).expect("demux main.ts");
    Some(
        media
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("AVC video track present"),
    )
}

const KID: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
];
const KEY: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

/// The measurement: allocation count/bytes/deallocs for one `CencEncryptor::encrypt`
/// pass (`cenc` scheme, NAL-aware subsample map — the common real-world config)
/// over every sample of the real fixture's video track, counted in isolation
/// from fixture load/demux. Printed to stderr (`--nocapture`) rather than
/// asserted exactly — the point of this file is the number, not a pass/fail
/// gate — but a sanity ceiling catches a wild blow-up (e.g. an accidental
/// per-subsample copy).
#[test]
fn cenc_encrypt_allocation_count_over_real_fixture() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let sample_count = media.tracks[0].samples.len();
    assert!(sample_count > 1, "fixture must carry more than one sample");

    let cfg = EncryptConfig {
        scheme: CencScheme::Cenc,
        kid: KID,
        key: KEY,
        iv: IvGen::Counter { base: 0 },
        pattern: None,
        subsample: SubsamplePolicy::Video,
    };

    reset_counters();
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let (allocs, alloc_bytes, deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT cenc_encrypt: samples={sample_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} deallocs={deallocs} \
         allocs_per_sample={:.2}",
        allocs as f64 / sample_count as f64
    );

    // Sanity ceiling: encrypting must not allocate wildly more than a small
    // constant number of allocations per sample (subsample-map Vec + IV Vec +
    // whatever the in-place rewrite needs) — catches a per-subsample-entry
    // copy regression without hardcoding the exact count (which is legitimately
    // allowed to move — that's what this file measures and reports).
    assert!(
        allocs < sample_count * 10,
        "encrypt allocated {allocs} times over {sample_count} samples — \
         far more than expected; likely a per-subsample or per-NAL copy regression"
    );
}

/// Same measurement for `cbcs` (AES-CBC pattern) — a different code path
/// through `cenc_crypto::cbcs_sample` with its own subsample walk.
#[test]
fn cbcs_encrypt_allocation_count_over_real_fixture() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let sample_count = media.tracks[0].samples.len();

    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID,
        key: KEY,
        iv: IvGen::Counter { base: 0 },
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::Video,
    };

    reset_counters();
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let (allocs, alloc_bytes, deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT cbcs_encrypt: samples={sample_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} deallocs={deallocs} \
         allocs_per_sample={:.2}",
        allocs as f64 / sample_count as f64
    );

    assert!(
        allocs < sample_count * 10,
        "cbcs encrypt allocated {allocs} times over {sample_count} samples — \
         far more than expected"
    );
}

// ---------------------------------------------------------------------------
// The WIN side (media plane step 2b): fan-out is a refcount bump, and RTP
// packetisation can slice a sample without copying it. These are the reasons
// `Sample.data` moved off `Vec<u8>` in the first place — confirmed here by
// `Bytes::as_ptr()` identity (same buffer, not merely equal content) and the
// same counting allocator the encrypt-path regression numbers above use.
// ---------------------------------------------------------------------------

/// Cloning a sample's `Bytes` to fan it out to N consumers (WHEP, RTMP-out,
/// SRT-out, loudness, DVR, catch-up — media plane §1) allocates zero payload
/// bytes and every clone shares the identical underlying buffer.
#[test]
fn fan_out_clone_allocates_no_payload_and_shares_the_buffer() {
    let Some(media) = clear_video_media() else {
        return;
    };
    let sample = &media.tracks[0].samples[0];
    assert!(!sample.data.is_empty(), "sample must carry payload bytes");
    let original_ptr = sample.data.as_ptr();

    const N_CONSUMERS: usize = 6;
    let mut fanned_out: Vec<bytes::Bytes> = Vec::with_capacity(N_CONSUMERS);

    reset_counters();
    for _ in 0..N_CONSUMERS {
        fanned_out.push(sample.data.clone());
    }
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT fan_out: consumers={N_CONSUMERS} allocs={allocs} alloc_bytes={alloc_bytes}"
    );

    for (i, clone) in fanned_out.iter().enumerate() {
        assert_eq!(
            clone.as_ptr(),
            original_ptr,
            "consumer {i}'s clone must share the original buffer, not copy it"
        );
        assert_eq!(clone.len(), sample.data.len());
    }
    assert_eq!(
        allocs, 0,
        "fanning a sample out to {N_CONSUMERS} consumers must allocate zero payload bytes \
         (got {allocs} allocations, {alloc_bytes} bytes)"
    );
}

/// An RTP packetiser slicing a sample into fixed-size packets (`Bytes::slice`)
/// must allocate nothing — a `Vec<u8>` slice can't be independently owned
/// without copying, which is the whole reason `Sample.data` is `Bytes`.
#[test]
fn rtp_sized_subrange_slicing_allocates_nothing() {
    let Some(media) = clear_video_media() else {
        return;
    };
    let sample = &media.tracks[0].samples[0];
    let total_len = sample.data.len();
    assert!(
        total_len > 32,
        "sample must be long enough to slice meaningfully"
    );

    const RTP_PAYLOAD_SIZE: usize = 32; // deliberately small so the fixture yields several packets
    let packet_count = total_len.div_ceil(RTP_PAYLOAD_SIZE);
    let mut packets: Vec<bytes::Bytes> = Vec::with_capacity(packet_count);

    reset_counters();
    let mut offset = 0usize;
    while offset < total_len {
        let end = (offset + RTP_PAYLOAD_SIZE).min(total_len);
        packets.push(sample.data.slice(offset..end));
        offset = end;
    }
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT rtp_slice: packets={} allocs={allocs} alloc_bytes={alloc_bytes}",
        packets.len()
    );

    assert_eq!(
        allocs,
        0,
        "slicing a sample into {} RTP-sized packets must allocate zero bytes \
         (got {allocs} allocations, {alloc_bytes} bytes)",
        packets.len()
    );

    // Each packet's bytes must equal the corresponding subrange of the
    // original — not just the same length (a zero-copy slice that grabbed
    // the wrong range would still pass a length-only check).
    let mut off = 0usize;
    for packet in &packets {
        let end = (off + RTP_PAYLOAD_SIZE).min(total_len);
        assert_eq!(&packet[..], &sample.data[off..end]);
        off = end;
    }
}
