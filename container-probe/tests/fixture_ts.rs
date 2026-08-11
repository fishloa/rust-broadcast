//! Real-fixture and robustness tests for [`container_probe`] — Work-package 1:
//! the MPEG-2 TS prober ([`container_probe::ts`]).
//!
//! Paths are relative to the workspace root; from a test we build them with
//! [`env!("CARGO_MANIFEST_DIR")`](./env) joined to `../<path>` because fixtures
//! are committed at the workspace's `fixtures/` tree, outside this crate.
//!
//! The stride/phase values asserted here were measured from the real files and
//! are correct; if the code disagrees with them the code is wrong.

use container_probe::{Confidence, Detail, Format, Probe};
use std::fs;

/// Join a workspace-relative fixture path to an absolute path from this crate.
fn fixture(rel: &str) -> Vec<u8> {
    fs::read(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
        .unwrap_or_else(|e| panic!("failed to read fixture {rel}: {e}"))
}

/// `LATTICE_STRONG` confidence — reused by name so assertions read clearly.
const LATTICE_STRONG: Confidence = Confidence::LATTICE_STRONG;

/// M2TS: 192-byte packets (a 4-byte `TP_extra_header` per 188-byte TS packet),
/// first sync at byte offset 4.
///
/// # Mutation proof
///
/// Removing the 192 stride from `TS_STRIDES` (and dropping the array length to
/// 3) breaks this test. Re-measured after the confidence/coverage fix, the
/// file is then `Unknown`:
/// ```
/// assertion `left == right` failed: probe mismatch
///   left: Unknown
///   right: Identified { format: MpegTs, confidence: Confidence(144),
///          detail: Ts { stride: 192, phase: 4 } }
/// ```
/// because no remaining stride's lattice reaches the weak threshold. (Earlier
/// this surfaced as `Insufficient { need_at_least: 55484 }`, before defects 1-2
/// changed the no-candidate branch.) The stride was restored.
#[test]
fn m2ts_192_stride() {
    let p = probe_fixture("fixtures/container-probe/m2ts_192.m2ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 192,
            phase: 4,
        },
    );
}

/// The mid-packet phase test.
///
/// # Mutation proof
///
/// This test bites: **deleting the phase loop** — so that only `phase == 0` is
/// ever probed for each stride — makes this file fail to identify as TS,
/// because the capture begins 111 bytes into a packet and there is no `0x47`
/// at offset 0. Observed failure (re-measured after the confidence/coverage
/// fix; with the loop collapsed to `for phase in 0..0`):
/// `cargo test -p container-probe` panicked with
/// ```
/// assertion `left == right` failed: probe mismatch
///   left: Unknown
///   right: Identified { format: MpegTs, confidence: Confidence(144),
///          detail: Ts { stride: 188, phase: 111 } }
/// ```
/// (Earlier this mutation surfaced as `Insufficient { need_at_least: 65724 }`,
/// before defects 1-2 changed the no-candidate branch.) The loop was restored.
#[test]
fn ts_midpacket_phase() {
    let p = probe_fixture("fixtures/container-probe/ts_midpacket_phase.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 188,
            phase: 111,
        },
    );
}

#[test]
fn ts_204_stride_synthetic() {
    let p = probe_fixture("fixtures/container-probe/ts_204_stride_SYNTHETIC.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 204,
            phase: 0,
        },
    );
}

#[test]
fn h264_aac_188_stride() {
    let p = probe_fixture("fixtures/ts/h264_aac.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 188,
            phase: 0,
        },
    );
}

/// Large real DVB captures (multimegabyte) the suite previously did not cover —
/// each must resolve to a 188-byte-stride, phase-0 lattice at `LATTICE_STRONG`.
#[test]
fn france2_capture() {
    let p = probe_fixture("fixtures/ts/france2.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 188,
            phase: 0,
        },
    );
}

#[test]
fn gulli_opengop_capture() {
    let p = probe_fixture("fixtures/ts/gulli-opengop.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 188,
            phase: 0,
        },
    );
}

#[test]
fn tnt5w_capture() {
    let p = probe_fixture("fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts");
    assert_identified(
        p,
        Format::MpegTs,
        LATTICE_STRONG,
        Detail::Ts {
            stride: 188,
            phase: 0,
        },
    );
}

/// Assert a fully-read non-TS file concludes precisely `Unknown`.
///
/// Each of these files is far longer than any lattice needs to prove itself
/// (`TS_CONFIRM_FOR_STRONG * TS_PACKET_SIZE_208`), yet none reaches even the
/// weak threshold of contiguous syncs. `Unknown` — not `Insufficient` — is the
/// only correct verdict: a streaming caller must stop, not read more. (A stray
/// `0x47`, the ASCII letter "G", appears in essentially any binary file and is
/// not evidence a larger buffer would turn into a transport stream.)
fn assert_unknown_non_ts(rel: &str) {
    let p = probe_fixture(rel);
    assert_eq!(p, Probe::Unknown, "{rel} must be Unknown, got {p:?}");
}

#[test]
fn mp4_is_not_ts() {
    assert_unknown_non_ts("fixtures/mp4/h264_high.mp4");
}

#[test]
fn wav_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/pcm_s16le.wav");
}

#[test]
fn ogg_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/opus.ogg");
}

#[test]
fn mkv_is_not_ts() {
    assert_unknown_non_ts("fixtures/mkv/h264_aac.mkv");
}

#[test]
fn flv_is_not_ts() {
    assert_unknown_non_ts("fixtures/flv/av.flv");
}

#[test]
fn asf_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/video.asf");
}

/// Regression guard for the CENC false positive.
///
/// This CENC-encrypted MP4 (high-entropy encrypted payload) previously probed
/// to a confident `MpegTs` at `Ts { stride: 208, phase: 142 }`, conf 96
/// (`LATTICE_WEAK`): across the 792 lanes, three consecutive `0x47` bytes
/// aligned on one lane purely by chance. A confident wrong answer is the worst
/// outcome a probe can produce. The fix required a candidate lane to *cover* at
/// least `TS_MIN_COVERAGE_PCT` of its positions with sync bytes — a real TS
/// stream syncs at ~100% of positions, random noise at ~2.5% — so this file now
/// correctly reports `Unknown`.
///
/// # Mutation proof
///
/// This test bites: setting `TS_MIN_COVERAGE_PCT` to `0` (keeping only the
/// run-length test) makes `cenc.mp4` probe to a false positive again. Observed
/// failure:
/// ```
/// assertion `left == right` failed:
///   left: Identified { format: MpegTs, confidence: Confidence(96),
///          detail: Ts { stride: 208, phase: 142 } }
///   right: Unknown
/// ```
/// The constant was restored to `50` and the coverage gate is guarded here.
#[test]
fn cenc_mp4_is_not_ts() {
    assert_unknown_non_ts("fixtures/mp4/cenc.mp4");
}

#[test]
fn av1_mp4_is_not_ts() {
    assert_unknown_non_ts("fixtures/mp4/av1.mp4");
}

#[test]
fn vp9_opus_mkv_is_not_ts() {
    assert_unknown_non_ts("fixtures/mkv/vp9_opus.mkv");
}

#[test]
fn vp8_opus_webm_is_not_ts() {
    assert_unknown_non_ts("fixtures/webm/vp8_opus.webm");
}

#[test]
fn ps_is_not_ts() {
    assert_unknown_non_ts("fixtures/ps/h264_ac3.ps");
}

#[test]
fn mxf_is_not_ts() {
    assert_unknown_non_ts("fixtures/mxf/op1a_mpeg2_pcm.mxf");
}

#[test]
fn adts_aac_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/aac.adts");
}

#[test]
fn mp3_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/audio.mp3");
}

#[test]
fn annexb_is_not_ts() {
    assert_unknown_non_ts("fixtures/container-probe/h264.annexb");
}

// ---------------------------------------------------------------------------
// Negative / robustness cases — each must be `Unknown` or `Insufficient`,
// never a confident wrong answer, and must never panic.
// ---------------------------------------------------------------------------

#[test]
fn empty_slice() {
    let p = probe(&[]);
    assert!(
        matches!(p, Probe::Unknown | Probe::Insufficient { .. }),
        "got {p:?}"
    );
}

#[test]
fn single_byte() {
    let p = probe(&[0x42]);
    assert!(
        matches!(p, Probe::Unknown | Probe::Insufficient { .. }),
        "got {p:?}"
    );
}

#[test]
fn eight_zero_bytes() {
    let p = probe(&[0u8; 8]);
    assert!(
        matches!(p, Probe::Unknown | Probe::Insufficient { .. }),
        "got {p:?}"
    );
}

/// 4096 bytes of `0xFF`. No sync bytes at all -> no seed -> `Unknown`.
#[test]
fn ff_bytes() {
    let p = probe(&[0xFFu8; 4096]);
    assert!(
        matches!(p, Probe::Unknown | Probe::Insufficient { .. }),
        "got {p:?}"
    );
}

/// 70,000 zero bytes with a single `0x47` at offset 12,345. The lone sync byte
/// — the ASCII "G" — seeds a run of exactly 1 on every stride lane, but the
/// region is far longer than any lattice needs to prove itself, so nothing
/// reaching the weak threshold means `Unknown`, never an endless
/// `Insufficient`/"read more".
#[test]
fn zeros_with_single_sync_byte() {
    let mut data = vec![0u8; 70_000];
    data[12_345] = 0x47;
    let p = probe(&data);
    assert_eq!(p, Probe::Unknown, "got {p:?}");
}

/// A short but complete-looking transport stream: the first 600 bytes of
/// `h264_aac.ts` is 4 whole 188-byte packets, every lattice position a sync
/// byte. That is a **match** — weakly, since it is below the 8 confirmations
/// `LATTICE_STRONG` needs — never `Insufficient`.
///
/// This is the regression guard for a real defect. An earlier revision
/// downgraded any qualifying lane to `Insufficient` when the buffer was too
/// short to reach `LATTICE_STRONG`, on the reasoning that a truncated sample is
/// unproven. A sweep of all 145 media files in the repo showed what that
/// actually did: eleven real and *complete* TS files of 188 B - 1.1 KB
/// (`fixtures/ts/scte35-*.ts`, `fixtures/ts/pts-*.ts`,
/// `fixtures/mpeg-ts/af-*.ts`) answered `Insufficient { need_at_least: 1504 }`
/// — telling a caller to read past the end of a file it had fully read.
///
/// MUTATION VERIFIED: restoring the `could_reach_strong` downgrade turns this
/// red with `Insufficient { need_at_least: 1504 }`.
#[test]
fn short_but_complete_ts_is_a_weak_match_not_insufficient() {
    let data = fixture("fixtures/ts/h264_aac.ts");
    let p = probe(&data[..600]);
    match p {
        Probe::Identified {
            format,
            confidence,
            detail,
        } => {
            assert_eq!(format, Format::MpegTs);
            assert_eq!(confidence, Confidence::LATTICE_WEAK);
            assert_eq!(
                detail,
                Detail::Ts {
                    stride: 188,
                    phase: 0
                }
            );
        }
        other => panic!("600 bytes of whole TS packets must be a weak match, got {other:?}"),
    }
}

/// A single 188-byte packet is genuinely too little to conclude from: one sync
/// byte at offset 0 is one confirmation, below `TS_CONFIRM_FOR_WEAK`. Here
/// `Insufficient` IS correct — there is a coherent start and more bytes really
/// would settle it. This is the boundary case that proves the fix above did not
/// simply delete the `Insufficient` path.
#[test]
fn a_single_ts_packet_is_insufficient() {
    let data = fixture("fixtures/ts/h264_aac.ts");
    match probe(&data[..188]) {
        Probe::Insufficient { need_at_least } => {
            assert!(
                need_at_least > 188,
                "need_at_least {need_at_least} must exceed the 188 supplied"
            );
        }
        other => panic!("a single TS packet must be Insufficient, got {other:?}"),
    }
}

/// Small but complete TS fixtures carrying at least `TS_CONFIRM_FOR_WEAK`
/// packets identify, rather than asking for bytes that do not exist.
///
/// These are the files a sweep of every media file in the repo caught reporting
/// `Insufficient { need_at_least: 1504 }` — each is a whole file on disk of
/// 752-1128 bytes (4-6 whole packets, every lattice position a sync byte), so
/// the caller can never supply more. See
/// [`short_but_complete_ts_is_a_weak_match_not_insufficient`] for the defect.
///
/// The threshold is packet count, not file completeness: a fixture below
/// `TS_CONFIRM_FOR_WEAK` packets is covered by
/// [`tiny_ts_fixtures_below_the_weak_threshold_are_insufficient`] instead.
#[test]
fn small_complete_ts_fixtures_all_identify() {
    for path in [
        "fixtures/mpeg-ts/af-transport-private-data.ts", // 752 B — 4 packets
        "fixtures/ts/pcr-wrap.ts",                       // 940 B — 5 packets
        "fixtures/ts/pts-backward.ts",                   // 940 B — 5 packets
        "fixtures/ts/pts-wrap.ts",                       // 940 B — 5 packets
        "fixtures/ts/scte35-pcr.ts",                     // 1128 B — 6 packets
    ] {
        let data = fixture(path);
        match probe(&data) {
            Probe::Identified {
                format,
                confidence,
                detail,
            } => {
                assert_eq!(format, Format::MpegTs, "{path}");
                assert_eq!(confidence, Confidence::LATTICE_WEAK, "{path}");
                assert_eq!(
                    detail,
                    Detail::Ts {
                        stride: 188,
                        phase: 0
                    },
                    "{path}"
                );
            }
            other => panic!("{path} ({} bytes) must identify, got {other:?}", data.len()),
        }
    }
}

/// A complete file can still be too small to conclude from. These fixtures hold
/// one or two whole packets — below `TS_CONFIRM_FOR_WEAK` contiguous
/// confirmations — so `Insufficient` is the honest verdict even though no more
/// bytes exist on disk.
///
/// The probe takes a slice and is given no end-of-file signal, so it cannot know
/// the caller has nothing further; reporting a confident `MpegTs` from a single
/// `0x47` at offset 0 would be a guess. A caller that knows it is at EOF treats
/// `Insufficient` as "undecidable from this file", which is exactly right for
/// 188 bytes.
#[test]
fn tiny_ts_fixtures_below_the_weak_threshold_are_insufficient() {
    for path in [
        "fixtures/ts/emsg-pid4.ts",            // 188 B — 1 packet
        "fixtures/ts/scte35-balanced.ts",      // 188 B — 1 packet
        "fixtures/ts/scte35-real.ts",          // 188 B — 1 packet
        "fixtures/ts/scte35-unbalanced.ts",    // 188 B — 1 packet
        "fixtures/mpeg-ts/af-pcr-stuffing.ts", // 376 B — 2 packets
    ] {
        let data = fixture(path);
        match probe(&data) {
            Probe::Insufficient { need_at_least } => {
                assert!(need_at_least > data.len(), "{path}");
            }
            other => panic!(
                "{path} ({} bytes) is below the weak threshold and must be \
                 Insufficient, got {other:?}",
                data.len()
            ),
        }
    }
}

/// 4096 bytes of `0x47` — a pathological TS-shaped buffer with no real
/// structure.
///
/// A naive lattice gives *every* stride lane a run of ~20 syncs (the whole
/// buffer is `0x47`), which would score `LATTICE_STRONG`. That is a false
/// positive. Our prober rejects a region that is *uniformly* the sync byte:
/// `0x47` is "G", and a continuous run with no packet content to fill the
/// lanes is not a real transport stream (a genuine 188-byte packet has exactly
/// one sync byte and 187 other bytes). The uniform-buffer guard therefore
/// returns `Unknown` here — defensible because the all-`0x47` file carries no
/// evidence of packet structure, only of the sync character repeating.
#[test]
fn all_sync_bytes() {
    let p = probe(&[0x47u8; 4096]);
    assert!(
        matches!(p, Probe::Unknown | Probe::Insufficient { .. }),
        "got {p:?} — all-sync input must not be flagged as TS"
    );
}

/// Budget test. The 188-stride fixture probed with a 512-byte budget reads no
/// further than 512 bytes (the harness caps the region at `min(len, budget)`),
/// so a full strong lattice (8 confirmations at 188 bytes each = 1504+ bytes)
/// cannot form. The best lane reaches exactly 3 syncs -> a `LATTICE_WEAK`
/// verdict, a genuinely *weaker* conclusion than the unbounded `LATTICE_STRONG`
/// in `h264_aac_188_stride`. That is the saner of the two allowed outcomes and
/// shows the budget is actually clamped.
#[test]
fn budget_caps_the_read() {
    let data = fixture("fixtures/ts/h264_aac.ts");
    let p = probe_with_budget(&data, 512);
    match p {
        Probe::Identified {
            format,
            confidence,
            detail,
        } => {
            assert_eq!(format, Format::MpegTs);
            assert_eq!(confidence, Confidence::LATTICE_WEAK);
            assert_eq!(
                detail,
                Detail::Ts {
                    stride: 188,
                    phase: 0
                }
            );
        }
        Probe::Insufficient { need_at_least } => {
            // Also acceptable; the buffer was too short to prove the lattice.
            assert!(need_at_least > 512);
        }
        other => panic!("budget probe returned {other:?}; expected a weak verdict or Insufficient"),
    }
}

/// Run [`container_probe::ts::probe`]-level assertion helper: unwrap an
/// `Identified` and check all three fields.
fn assert_identified(p: Probe, format: Format, confidence: Confidence, detail: Detail) {
    assert_eq!(
        p,
        Probe::Identified {
            format,
            confidence,
            detail
        },
        "probe mismatch"
    );
}

fn probe(data: &[u8]) -> Probe {
    container_probe::probe(data)
}

fn probe_with_budget(data: &[u8], budget: usize) -> Probe {
    container_probe::probe_with_budget(data, budget)
}

/// Load a fixture and probe it.
fn probe_fixture(rel: &str) -> Probe {
    probe(&fixture(rel))
}
