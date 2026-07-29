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
//!
//! ## Thread isolation (why the counters are `thread_local!`, not global statics)
//!
//! Under plain `cargo test` (what CI's `cargo test --workspace --all-features
//! --locked` runs), the `#[test]` fns in one binary run **concurrently on a
//! thread pool**. A process-global allocation counter would count every
//! allocation on every thread, so one test's `reset_counters()` .. `encrypt()`
//! .. `snapshot_counters()` window would pick up unrelated allocations from a
//! sibling test's `clear_video_media()` (which demuxes a whole TS file —
//! thousands of allocations) running in parallel — flaking the `assert_eq!`
//! tests below for a reason that has nothing to do with the code under test.
//! Under `cargo nextest run`, each test is its own **process**, so this failure
//! mode is invisible there; a harness that only proves itself under one runner
//! and silently flakes under the other is worse than one that just fails, so
//! don't remove this and re-derive the flake later.
//!
//! Thread-local counters make cross-thread noise structurally impossible: each
//! test only ever sees allocations made by its own thread. `Cell<usize>` has no
//! `Drop` impl, and the `const { .. }` initializer means no lazy first-access
//! allocation, so accessing the `thread_local!` from inside `GlobalAlloc::alloc`
//! itself cannot re-entrantly allocate (verified below by running the gate
//! twice and diffing the printed counts — see the module's test-running notes
//! in the PR description).

#![cfg(feature = "cenc")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;

use broadcast_common::{Encrypt, Unpackage};
use transmux::{
    CencEncryptor, CencScheme, CodecConfig, ConstantIvSenc, EncryptConfig, IvGen, Media,
    RtpPacketiser, Sample,
    SubsamplePolicy, TsDemux,
};

/// Counts every allocation/deallocation made **by the calling thread**.
/// Thread-local (see the module doc for why) so parallel `#[test]` fns under
/// plain `cargo test` cannot pollute each other's measurement window.
struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    static ALLOC_BYTES: Cell<usize> = const { Cell::new(0) };
    static DEALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.with(|c| c.set(c.get() + 1));
        ALLOC_BYTES.with(|c| c.set(c.get() + layout.size()));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOC_COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn reset_counters() {
    ALLOC_COUNT.with(|c| c.set(0));
    ALLOC_BYTES.with(|c| c.set(0));
    DEALLOC_COUNT.with(|c| c.set(0));
}

fn snapshot_counters() -> (usize, usize, usize) {
    (
        ALLOC_COUNT.with(Cell::get),
        ALLOC_BYTES.with(Cell::get),
        DEALLOC_COUNT.with(Cell::get),
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

/// Re-measured 2026-07-27 (second time same day — see below) against
/// `fixtures/ts/h264/main.ts` (post-refactor, `Sample.data: Bytes`), via
/// `cargo test -p transmux --all-features --locked --test alloc_measurement
/// -- --nocapture`, run twice with identical results both times (thread-local
/// counters, see module doc). If a legitimate change moves these, re-run the
/// gate, update the consts + this comment's date, and paste the new numbers
/// into the PR/report — don't just widen the tolerance.
///
/// This moved up again from the same-day measurement (51 / 2_888 / 34) after
/// the adversarial-review follow-up fix (F1: validate the *planned*
/// (track, sample) -> IV mapping in full, up front, before ciphering a single
/// byte, then cipher from that exact plan — see `cenc_encrypt.rs`'s
/// `CencEncryptor::plan_sample_ivs`/`assert_ivs_unique`) replaced the old
/// post-cipher-only backstop: computing the whole plan up front (one
/// `Vec<Vec<Vec<u8>>>`) before the cipher loop, rather than resolving each IV
/// inline as the loop went, is genuinely more allocation, not a regression —
/// it is what makes a rejected config leave `media` byte-identical instead of
/// checking that too late. It previously moved up from the 2026-07-26
/// measurement (48 / 2216 / 31) the very next commit on this branch
/// (76b9325d, "CENC IV uniqueness across tracks"): fixing the AES-CTR
/// keystream-reuse bug made `IvGen::Counter`'s sample index run across the
/// whole `Media` instead of resetting per track, plus added a
/// post-generation duplicate-IV backstop — genuinely more work per
/// `encrypt()` call, not a regression. Tightening these to `assert_eq!` (the
/// original T2 story) is what caught both drifts: the previous "<= 2x"
/// tolerance band would have silently absorbed them.
const CENC_MEASURED_ALLOCS: usize = 65;
const CENC_MEASURED_ALLOC_BYTES: usize = 2_720;
const CENC_MEASURED_DEALLOCS: usize = 48;
// NOTE: the three pinned metrics (allocs/alloc_bytes/deallocs) are asserted
// with `assert_eq!` below, not a tolerance band. A tolerance multiple (e.g.
// "within 2x") is wide enough that short-circuiting `CencEncryptor::encrypt`
// to `Ok(())` — allocating and freeing *nothing* — still satisfies "<= 2x a
// nonzero pinned count", so the assertion would pass against a no-op. Pin
// the exact measured value instead: if a legitimate change moves it,
// re-measure with `cargo test -p transmux --all-features --locked --test
// alloc_measurement -- --nocapture` (run twice to confirm stability — see
// the module doc), then update the const + this comment's date.

/// The measurement: allocation count/bytes/deallocs for one `CencEncryptor::encrypt`
/// pass (`cenc` scheme, NAL-aware subsample map — the common real-world config)
/// over every sample of the real fixture's video track, counted in isolation
/// from fixture load/demux. Printed to stderr (`--nocapture`); asserted against
/// the pinned measurement above (a small tolerance, not a 10x ceiling) so a
/// regression is actually caught rather than merely reported.
#[test]
fn cenc_encrypt_allocation_count_over_real_fixture() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let sample_count = media.tracks[0].samples.len();
    assert!(sample_count > 1, "fixture must carry more than one sample");
    // Total cleartext payload size across every sample of the track, captured
    // BEFORE reset_counters() so it costs nothing in the measurement window.
    // This is the number a per-sample/per-subsample full-payload-copy
    // regression would show up against in alloc_bytes below — a copy-based
    // regression allocates on the order of this many bytes; genuine in-place
    // rewrite overhead (subsample-map Vecs, IV buffers) does not.
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();

    let cfg = EncryptConfig {
        scheme: CencScheme::Cenc,
        kid: KID,
        iv: IvGen::Counter,
        pattern: None,
        subsample: SubsamplePolicy::Video,
        constant_iv_senc: ConstantIvSenc::default(),
    };

    reset_counters();
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let (allocs, alloc_bytes, deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT cenc_encrypt: samples={sample_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} deallocs={deallocs} total_payload={total_payload} \
         allocs_per_sample={:.2}",
        allocs as f64 / sample_count as f64
    );

    // The metric that actually detects a copy: encrypting in place must
    // allocate far less than the payload it encrypts. A per-sample full
    // payload copy would allocate ~total_payload bytes (~100%); the measured
    // in-place rewrite bookkeeping (subsample-map Vecs, IV Vecs) on this
    // fixture is 2888/22927 ≈ 12.6% — a quarter of the payload gives ~2x
    // headroom over that measured ratio (tolerating platform/measurement
    // jitter) while still catching a copy regression by a wide margin (which
    // would land at or above 100%, not 25%).
    assert!(
        alloc_bytes < total_payload / 4,
        "encrypt allocated {alloc_bytes} bytes over a {total_payload}-byte payload \
         (>25%) — looks like a per-sample or per-subsample payload copy regression"
    );
    assert_eq!(
        alloc_bytes, CENC_MEASURED_ALLOC_BYTES,
        "encrypt allocated {alloc_bytes} bytes — the pinned measurement is \
         {CENC_MEASURED_ALLOC_BYTES}; re-measure and update the const (with a fresh date) if \
         this is a legitimate change, otherwise this is a regression (a no-op encrypt would \
         allocate 0, not the pinned amount)"
    );
    assert_eq!(
        allocs, CENC_MEASURED_ALLOCS,
        "encrypt allocated {allocs} times over {sample_count} samples — the pinned measurement \
         is {CENC_MEASURED_ALLOCS}; likely a per-subsample or per-NAL copy regression (or, if \
         lower, encrypt did less work than it should have)"
    );
    assert_eq!(
        deallocs, CENC_MEASURED_DEALLOCS,
        "dealloc count {deallocs} drifted from the pinned measurement of \
         {CENC_MEASURED_DEALLOCS} — re-measure and update the const if this is a legitimate \
         change"
    );
}

/// Re-measured 2026-07-27 (second time same day) against
/// `fixtures/ts/h264/main.ts` (post-refactor, `Sample.data: Bytes`) the same
/// way as the `cenc` consts above — same plan-then-cipher (F1) rationale for
/// the move from 51/2_888/34, which itself had moved from 48/2216/31 for the
/// 76b9325d IV-uniqueness fix.
const CBCS_MEASURED_ALLOCS: usize = 65;
const CBCS_MEASURED_ALLOC_BYTES: usize = 2_720;
const CBCS_MEASURED_DEALLOCS: usize = 48;

/// Same measurement for `cbcs` (AES-CBC pattern) — a different code path
/// through `cenc_crypto::cbcs_sample` with its own subsample walk.
#[test]
fn cbcs_encrypt_allocation_count_over_real_fixture() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let sample_count = media.tracks[0].samples.len();
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();

    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID,
        iv: IvGen::Counter,
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::Video,
        constant_iv_senc: ConstantIvSenc::default(),
    };

    reset_counters();
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let (allocs, alloc_bytes, deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT cbcs_encrypt: samples={sample_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} deallocs={deallocs} total_payload={total_payload} \
         allocs_per_sample={:.2}",
        allocs as f64 / sample_count as f64
    );

    // See the `cenc` test above for the rationale on each assertion (same
    // fixture/measured ratio: 2888/22927 ≈ 12.6%, so a quarter of the payload
    // gives the same ~2x headroom).
    assert!(
        alloc_bytes < total_payload / 4,
        "cbcs encrypt allocated {alloc_bytes} bytes over a {total_payload}-byte payload \
         (>25%) — looks like a per-sample or per-subsample payload copy regression"
    );
    assert_eq!(
        alloc_bytes, CBCS_MEASURED_ALLOC_BYTES,
        "cbcs encrypt allocated {alloc_bytes} bytes — the pinned measurement is \
         {CBCS_MEASURED_ALLOC_BYTES}; re-measure and update the const (with a fresh date) if \
         this is a legitimate change, otherwise this is a regression (a no-op encrypt would \
         allocate 0, not the pinned amount)"
    );
    assert_eq!(
        allocs, CBCS_MEASURED_ALLOCS,
        "cbcs encrypt allocated {allocs} times over {sample_count} samples — the pinned \
         measurement is {CBCS_MEASURED_ALLOCS}; likely a per-subsample or per-NAL copy \
         regression (or, if lower, encrypt did less work than it should have)"
    );
    assert_eq!(
        deallocs, CBCS_MEASURED_DEALLOCS,
        "cbcs dealloc count {deallocs} drifted from the pinned measurement of \
         {CBCS_MEASURED_DEALLOCS} — re-measure and update the const if this is a legitimate \
         change"
    );
}

// ---------------------------------------------------------------------------
// The `Bytes` capability side (media plane step 2b): `Bytes::clone` is a
// refcount bump and `Bytes::slice` shares the buffer without copying. These
// are the capabilities `Sample.data` moved off `Vec<u8>` to gain — confirmed
// here by `Bytes::as_ptr()` identity (same buffer, not merely equal content)
// and the same counting allocator the encrypt-path regression numbers above
// use.
//
// IMPORTANT — this proves the capability, not that transmux exploits it yet:
// - No production fan-out path exists yet (the Trunk isn't built).
// - The production RTP packetiser does NOT use it: `rtp.rs:323`
//   (`packetise_video`) returns `Vec<Vec<u8>>`, built by copying
//   (`pkt.extend_from_slice(nal)` at `rtp.rs:365,488-489,540` and
//   `payload[off..end].to_vec()` at `rtp.rs:905,996`), and `rtp_stream.rs:365`
//   does `rtp_packet.to_vec()`. `grep -rn "\.slice(" transmux/src/` currently
//   returns zero matches — every production copy point above is real.
//   Issue #777 tracks converting the packetiser to `Bytes::slice` for a
//   genuinely zero-copy egress path.
// ---------------------------------------------------------------------------

/// Cloning a sample's `Bytes` to fan it out to N consumers (WHEP, RTMP-out,
/// SRT-out, loudness, DVR, catch-up — media plane §1) allocates zero payload
/// bytes and every clone shares the identical underlying buffer. This
/// measures a capability `Bytes` provides; no production fan-out path
/// consumes it yet (the Trunk isn't built) — see the section header above.
#[test]
fn bytes_clone_shares_buffer_enabling_zero_copy_fan_out() {
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

/// The production RTP packetiser (issue #777) — feed the real fixture's
/// H.264 track through the production `packetise_video` (the same code
/// path `Package::package` calls, minus SDP generation and with STAP-A
/// disabled to isolate per-packet allocation). Pin the exact measured
/// allocation: if a copy-based regression doubles it, the assertion fails.
///
/// The baseline measurement: 39 packets, 70 allocations, 9 632 bytes.
/// This covers the per-sample NAL-split vectors (15 Vec<&[u8]>),
/// per-packet header BytesMut slices (39), and the output Vec<RtpPacket>.
/// None of it scales with the NAL payload bytes — a copy-based packetiser
/// would add at least `total_payload` (22 927) bytes, landing near 39 000.
///
/// Re-measure after any legitimate change that moves these (run twice to
/// confirm stability — see the module doc) and update the consts.
const RTP_MEASURED_ALLOCS: usize = 70;
const RTP_MEASURED_ALLOC_BYTES: usize = 9_632;

#[test]
fn rtp_packetiser_allocates_zero_payload_bytes() {
    let Some(media) = clear_video_media() else {
        return;
    };
    let sample_count = media.tracks[0].samples.len();
    assert!(sample_count > 1, "fixture must carry more than one sample");
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();
    assert!(
        total_payload > 10_000,
        "fixture payload must be non-trivial"
    );

    // Disable STAP-A so the measurement isolates per-sample packetising.
    let pkt = RtpPacketiser {
        stap_a_parameter_sets: false,
        ..RtpPacketiser::default()
    };

    reset_counters();
    let result = pkt.packetise_video(&media.tracks[0], 96);
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();

    let packets = result.expect("packetise video");
    let packet_count = packets.len();
    eprintln!(
        "MEASUREMENT rtp_packetise: packets={packet_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} total_payload={total_payload}"
    );

    // Pin the exact measured allocation — a copy-based regression would
    // add at least `total_payload` bytes (~23 000), so the pinned value
    // would move by >2× and fail.
    assert_eq!(
        allocs, RTP_MEASURED_ALLOCS,
        "RTP packetiser allocation count {allocs} drifted from the pinned \
         measurement of {RTP_MEASURED_ALLOCS} — re-measure and update the const \
         if this is a legitimate change; a copy-based packetiser would allocate \
         many more (one per NAL + one per packet body)."
    );
    assert_eq!(
        alloc_bytes,
        RTP_MEASURED_ALLOC_BYTES,
        "RTP packetiser allocated {alloc_bytes} bytes — the pinned measurement \
         is {RTP_MEASURED_ALLOC_BYTES}. A copy-based packetiser would add \
         {total_payload} bytes (the sum of all NAL payloads) on top, moving \
         this to ~{}. Re-measure and update the const if this is a legitimate \
         change; otherwise this is a payload-copy regression (issue #777).",
        RTP_MEASURED_ALLOC_BYTES + total_payload
    );
}

/// Independence proof: packetise the original fixture at default MTU (39
/// packets, ~9 632 bytes). Then packetise a 10×-payload-per-sample track
/// at a high enough MTU that every NAL still fits in one packet. The
/// per-packet allocation constant should be unchanged regardless of
/// payload size. A copy-based packetiser would allocate payload bytes and
/// show a large per-packet jump.
#[test]
fn rtp_packetiser_per_packet_cost_is_independent_of_payload_size() {
    let Some(media) = clear_video_media() else {
        return;
    };

    use transmux::media::Track;

    let orig_track = &media.tracks[0];
    let spec = orig_track.spec.clone();

    // Build 10× payload: each sample's data repeated 10× (10× the NALs,
    // 10× the packets at the same per-packet header cost). Use the
    // DEFAULT MTU so we measure the same code path as the baseline.
    let scaled_samples: Vec<Sample> = orig_track
        .samples
        .iter()
        .map(|s| {
            let mut big = Vec::with_capacity(s.data.len() * 10);
            for _ in 0..10 {
                big.extend_from_slice(&s.data);
            }
            Sample::new(big, s.dts, s.pts, s.duration, s.flags.is_sync)
        })
        .collect();
    let scaled_track = Track::new(spec, scaled_samples);
    let total_payload_10x: usize = scaled_track.samples.iter().map(|s| s.data.len()).sum();
    eprintln!("PROOF 10× payload: {total_payload_10x} bytes");

    let pkt = RtpPacketiser {
        stap_a_parameter_sets: false,
        ..RtpPacketiser::default()
    };

    reset_counters();
    let result = pkt.packetise_video(&scaled_track, 96);
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();

    let packets = result.expect("packetise 10x video");
    let packet_count = packets.len();
    eprintln!(
        "MEASUREMENT rtp_packetise_10x: packets={packet_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} total_payload={total_payload_10x}"
    );

    // The per-packet allocation constant: baseline ~247 bytes/packet
    // (9 632 / 39). The 10× track has 10× the NALs (and hence 10× the
    // packets: 390), so the total bytes scale linearly with packet count
    // — but the *per-packet* constant should stay ~247, NOT jump to
    // include per-packet payload bytes. A copy-based packetiser would add
    // payload bytes to every packet (>500 bytes/packet).
    let per_packet_10x = alloc_bytes as f64 / packet_count as f64;
    let per_packet_baseline = RTP_MEASURED_ALLOC_BYTES as f64 / 39.0;
    eprintln!("10× per-packet: {per_packet_10x:.1} bytes (baseline: {per_packet_baseline:.1})");
    assert!(
        per_packet_10x < per_packet_baseline * 2.0,
        "RTP packetiser 10× payload: per-packet allocation {per_packet_10x:.1} bytes \
         vs baseline {per_packet_baseline:.1} bytes. A copy-based packetiser would \
         add payload bytes to every packet, pushing this well past 2×."
    );
}
