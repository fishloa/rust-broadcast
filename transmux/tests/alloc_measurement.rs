//! Allocation-count measurement for the CENC encrypt path (media plane step 2b,
//! G12 — `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
//!
//! The spec explicitly admits that `Sample.data: Bytes` (immutable/shared) may
//! cost extra allocations on the encrypt/decrypt in-place rewrite paths versus
//! the old `Vec<u8>`, and says the design "owes a measurement rather than an
//! assumption". This file is that measurement: a counting `#[global_allocator]`
//! instruments the `CencEncryptStage` pass over the real committed fixture
//! (`fixtures/ts/h264/main.ts`), isolated to just that pass (counters
//! are reset immediately before it and read immediately after, so fixture
//! load/demux overhead is excluded).
//!
//! Deliberately public-API-only (same fixture/config shape as
//! `tests/it/cenc_encrypt.rs`) so this file compiles unchanged against BOTH the
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

use broadcast_common::stage::{Stage, Timestamp};
use test_alloc::ThreadCounting;
use test_harness::collect::Collected as Media;
use test_harness::collect::CollectedSelectExt;
use transmux::{
    CencEncryptStage, CencScheme, CodecConfig, EncryptConfig, RtpPacketiserStage, Sample,
    SubsamplePolicy,
};

// Counts every allocation/deallocation made **by the calling thread**; thread-local
// (see the module doc for why) so parallel `#[test]` fns under plain `cargo test`
// cannot pollute each other's measurement window.
#[global_allocator]
static GLOBAL: ThreadCounting = ThreadCounting::new();

fn reset_counters() {
    ThreadCounting::reset();
}

fn snapshot_counters() -> (usize, usize, usize) {
    let s = ThreadCounting::snapshot();
    (s.allocs, s.bytes, s.deallocs)
}

/// The real cleartext H.264 capture (same fixture `cenc_encrypt.rs`'s unit
/// tests use), narrowed to its single AVC video track.
fn clear_video_media() -> Media {
    let bytes = test_fixtures::read("ts/h264/main.ts");

    let media = test_harness::collect::media_from_ts(bytes.as_slice()).expect("demux main.ts");
    media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present")
}

/// Drive the encrypt stage over `media`, discarding its output: the cipher
/// work (and its allocations) happen in `feed`, which is what is measured.
fn encrypt_through_stage(media: Media, cfg: EncryptConfig) {
    use transmux::mux_stage::MuxInput;
    let mut stage = CencEncryptStage::new(KEY, cfg).expect("stage config");
    for track in media.tracks {
        let track_id = track.spec.track_id;
        stage
            .feed(MuxInput::Track(Box::new(track.spec)), Timestamp::ZERO)
            .expect("announce");
        while stage.poll().is_some() {}
        for sample in track.samples {
            stage
                .feed(
                    MuxInput::Sample {
                        track: track_id,
                        sample,
                    },
                    Timestamp::ZERO,
                )
                .expect("encrypt sample");
            while stage.poll().is_some() {}
        }
    }
    stage.finish().expect("finish");
}

const KID: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
];
const KEY: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

/// The pinned `cenc_encrypt` measurements (allocs / alloc_bytes / deallocs)
/// against `fixtures/ts/h264/main.ts`, via
/// `cargo test -p transmux --all-features --locked --test alloc_measurement
/// -- --nocapture` (thread-local counters — see the module doc).
///
/// History (each a deliberate, reviewed move, not a drift):
/// - 48 / 2_216 / 31 (2026-07-26) — baseline.
/// - 51 / 2_888 / 34 — `76b9325d` IV-uniqueness fix: `IvGen::Counter`'s index
///   runs across the whole `Media`, plus a duplicate-IV backstop.
/// - 57 / 4_040 / 40 — F1: plan the whole `(track, sample) -> IV` map and
///   validate it before ciphering a byte (`plan_sample_ivs`/
///   `assert_ivs_unique`), then cipher from that exact plan, so a rejected
///   config leaves `media` byte-identical.
/// - 57 / 4_040 / 40 (2026-09-25) — r05-C5: subsample maps are planned up
///   front too (`plan_subsamples`), moving the last content-dependent failure
///   out of the cipher loop.
/// - 60 / 4_936 / 43 (2026-09-30) — r05-W3: `assert_ctr_ranges_disjoint`
///   collects one small `CtrRange` per sample and sorts it.
/// - 74 / 5_296 / 43 (2026-11-25) — per-sample protection: each sample now
///   owns an `Arc<SampleProtection>` (one allocation per sample) instead of a
///   parallel per-track entry list.
/// - 68 / 4_760 / 105 (2026-11-26) — encryption is a per-sample stage: the
///   measurement now covers the stage queue (and drops the input `Media`
///   inside the window, hence more deallocs than allocs), while the whole-media
///   plan that used to copy every sample handle is gone.
/// - 68 / 4_800 / 105 (2026-11-27) — `Sample` gained an 8-byte `SampleTrim`
///   field (the IR's per-sample gapless trim), so the per-sample vectors the
///   stage rebuilds are 8 bytes wider each.
/// - 68 / 4_808 / 105 (2026-11-27) — `TrackSpec` gained an
///   `Option<DecoderPreRoll>` (the container's decoder pre-roll requirement),
///   so the one spec clone is 8 bytes wider.
///
/// Re-measure and update these (with a fresh date) if a legitimate change
/// moves them; do not widen a tolerance instead.
const CENC_MEASURED_ALLOCS: usize = 68;
const CENC_MEASURED_ALLOC_BYTES: usize = 4_808;
const CENC_MEASURED_DEALLOCS: usize = 105;
// NOTE: the three pinned metrics (allocs/alloc_bytes/deallocs) are asserted
// with `assert_eq!` below, not a tolerance band. A tolerance multiple (e.g.
// "within 2x") is wide enough that short-circuiting the `CencEncryptStage` feed
// to `Ok(())` — allocating and freeing *nothing* — still satisfies "<= 2x a
// nonzero pinned count", so the assertion would pass against a no-op. Pin
// the exact measured value instead: if a legitimate change moves it,
// re-measure with `cargo test -p transmux --all-features --locked --test
// alloc_measurement -- --nocapture` (run twice to confirm stability — see
// the module doc), then update the const + this comment's date.

/// The measurement: allocation count/bytes/deallocs for one `CencEncryptStage`
/// pass (`cenc` scheme, NAL-aware subsample map — the common real-world config)
/// over every sample of the real fixture's video track, counted in isolation
/// from fixture load/demux. Printed to stderr (`--nocapture`); asserted against
/// the pinned measurement above (a small tolerance, not a 10x ceiling) so a
/// regression is actually caught rather than merely reported.
#[test]
fn cenc_encrypt_allocation_count_over_real_fixture() {
    let media = clear_video_media();
    let sample_count = media.tracks[0].samples.len();
    assert!(sample_count > 1, "fixture must carry more than one sample");
    // Total cleartext payload size across every sample of the track, captured
    // BEFORE reset_counters() so it costs nothing in the measurement window.
    // This is the number a per-sample/per-subsample full-payload-copy
    // regression would show up against in alloc_bytes below — a copy-based
    // regression allocates on the order of this many bytes; genuine in-place
    // rewrite overhead (subsample-map Vecs, IV buffers) does not.
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();

    let cfg = EncryptConfig::new(CencScheme::Cenc, KID, SubsamplePolicy::Video);

    reset_counters();
    encrypt_through_stage(media, cfg);
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
    // fixture is 4040/22927 ≈ 17.6% — a quarter of the payload gives ~1.4x
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

/// The pinned `cbcs_encrypt` measurements, same fixture/command as the `cenc`
/// consts above.
///
/// History:
/// - 57 / 4_040 / 40 — inherited the F1/r05-C5 planning changes above.
/// - 57 / 4_160 / 40 (2026-09-30) — r05-W4: a default `cbcs` `IvGen::Counter`
///   now emits 16-byte per-sample IVs (not the 8-byte shape Bento4 no-ops), so
///   the 15 planned IVs are 8 bytes wider each — same count, +120 bytes.
/// - 71 / 4_520 / 40 (2026-11-25) — per-sample protection (one
///   `Arc<SampleProtection>` per sample, replacing the parallel entry list).
/// - 65 / 3_680 / 102 (2026-11-26) — the per-sample encrypt stage (see the
///   `cenc` history above for why deallocs now exceed allocs).
/// - 65 / 3_720 / 102 (2026-11-27) — `Sample` gained an 8-byte `SampleTrim`
///   field (the IR's per-sample gapless trim), so the per-sample vectors the
///   stage rebuilds are 8 bytes wider each.
/// - 65 / 3_728 / 102 (2026-11-27) — the `TrackSpec` pre-roll field (above).
const CBCS_MEASURED_ALLOCS: usize = 65;
const CBCS_MEASURED_ALLOC_BYTES: usize = 3_728;
const CBCS_MEASURED_DEALLOCS: usize = 102;

/// Same measurement for `cbcs` (AES-CBC pattern) — a different code path
/// through `cenc_crypto::cbcs_sample` with its own subsample walk.
#[test]
fn cbcs_encrypt_allocation_count_over_real_fixture() {
    let media = clear_video_media();
    let sample_count = media.tracks[0].samples.len();
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();

    let cfg =
        EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::Video).with_pattern((1, 9));

    reset_counters();
    encrypt_through_stage(media, cfg);
    let (allocs, alloc_bytes, deallocs) = snapshot_counters();

    eprintln!(
        "MEASUREMENT cbcs_encrypt: samples={sample_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} deallocs={deallocs} total_payload={total_payload} \
         allocs_per_sample={:.2}",
        allocs as f64 / sample_count as f64
    );

    // See the `cenc` test above for the rationale on each assertion (same
    // fixture/measured ratio: 4040/22927 ≈ 17.6%, so a quarter of the payload
    // gives the same ~1.4x headroom).
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

// The RTP packetiser stage emits one whole RTP datagram per packet: the fixed
// header and the sample's payload in one buffer, because `MuxOutput::Bytes` is
// a single contiguous datagram. One buffer of each packet's own size is the
// floor; a second copy per packet (framing into an output buffer, then copying
// that into the sink) is what these two tests pin as absent.

/// Bytes of per-packet bookkeeping a datagram may cost beyond its own size:
/// the fixed header, the NAL split vector, and allocator rounding.
const RTP_PER_PACKET_SLACK: usize = 512;

/// Feed one track through `stage` and count the packets it emits.
fn feed_track(
    stage: &mut RtpPacketiserStage,
    track: &test_harness::collect::CollectedTrack,
) -> usize {
    stage
        .feed(
            transmux::mux_stage::MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )
        .expect("announce track");
    let mut packets = count_packets(stage);
    for sample in &track.samples {
        stage
            .feed(
                transmux::mux_stage::MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("feed sample");
        packets += count_packets(stage);
    }
    packets
}

fn count_packets(stage: &mut RtpPacketiserStage) -> usize {
    let mut packets = 0;
    while let Some(transmux::mux_stage::MuxOutput::Bytes(_)) = stage.poll() {
        packets += 1;
    }
    packets
}

#[test]
fn rtp_packetiser_allocates_one_buffer_per_packet() {
    let media = clear_video_media();
    let total_payload: usize = media.tracks[0].samples.iter().map(|s| s.data.len()).sum();
    assert!(
        total_payload > 10_000,
        "fixture payload must be non-trivial"
    );

    // Disable STAP-A so the measurement isolates per-sample packetising.
    let mut stage = RtpPacketiserStage::new().with_stap_a_parameter_sets(false);
    reset_counters();
    let packet_count = feed_track(&mut stage, &media.tracks[0]);
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();
    eprintln!(
        "MEASUREMENT rtp_packetise: packets={packet_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} total_payload={total_payload}"
    );

    assert!(packet_count > 0, "the fixture packetises");
    // One datagram buffer per packet (plus the per-sample NAL split vector),
    // never a second full-size copy of each packet.
    assert!(
        allocs <= packet_count * 4,
        "RTP packetiser made {allocs} allocations for {packet_count} packets — \
         more than a datagram buffer and a split vector per packet"
    );
    assert!(
        alloc_bytes <= total_payload + packet_count * RTP_PER_PACKET_SLACK,
        "RTP packetiser allocated {alloc_bytes} bytes for {total_payload} payload \
         bytes over {packet_count} packets — that is more than one copy of each \
         datagram, so a packet is being copied again after framing"
    );
}

/// Linear-cost proof: a 10× payload per sample costs 10× the datagram bytes,
/// not 100× and not a second full copy of everything.
#[test]
fn rtp_packetiser_cost_scales_with_payload_size() {
    let media = clear_video_media();

    use test_harness::collect::CollectedTrack as Track;

    let orig_track = &media.tracks[0];
    let spec = orig_track.spec.clone();

    let scaled_samples: Vec<Sample> = orig_track
        .samples
        .iter()
        .map(|s| {
            let mut big = Vec::with_capacity(s.data.len() * 10);
            for _ in 0..10 {
                big.extend_from_slice(&s.data);
            }
            Sample::new(
                big,
                s.timing.and_then(|t| t.dts()),
                s.timing.and_then(|t| t.pts()),
                s.timing.and_then(|t| t.duration()),
                s.flags.is_sync,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("valid timing");
    let scaled_track = Track::new(spec, scaled_samples);
    let total_payload_10x: usize = scaled_track.samples.iter().map(|s| s.data.len()).sum();

    let mut stage = RtpPacketiserStage::new().with_stap_a_parameter_sets(false);
    reset_counters();
    let packet_count = feed_track(&mut stage, &scaled_track);
    let (allocs, alloc_bytes, _deallocs) = snapshot_counters();
    eprintln!(
        "MEASUREMENT rtp_packetise_10x: packets={packet_count} allocs={allocs} \
         alloc_bytes={alloc_bytes} total_payload={total_payload_10x}"
    );

    assert!(packet_count > 0, "the scaled track packetises");
    assert!(
        allocs <= packet_count * 4,
        "RTP packetiser made {allocs} allocations for {packet_count} packets"
    );
    assert!(
        alloc_bytes <= total_payload_10x + packet_count * RTP_PER_PACKET_SLACK,
        "RTP packetiser allocated {alloc_bytes} bytes for {total_payload_10x} payload \
         bytes over {packet_count} packets — more than one copy of each datagram"
    );
}
