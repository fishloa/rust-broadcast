//! PCR-discontinuity detection + repair — real-fixture tests (#562).
//!
//! Oracle: `fixtures/ts/france-pcr-discontinuity.ts` — a 2688-packet slice of
//! a real French DTT capture carrying 5 PCR PIDs. Independently walking every
//! adaptation field in the fixture (byte-level scan, cross-checked against
//! the existing `tests/pcr_restamp.rs` re-anchor test) finds **exactly one**
//! PCR jump in the whole file:
//!
//! ```text
//! PID 0x0208, packet index 1251: PCR jumps ~10,035 ms forward,
//! discontinuity_indicator == 1 (already legally flagged in the source capture).
//! ```
//!
//! All other PCR PIDs (0x0078, 0x00dc, 0x026c, 0x02d0) step by a steady
//! ~35 ms with `discontinuity_indicator == 0` throughout — no discontinuity.
//!
//! Because the real capture's only break is already legally flagged, this
//! file's **unflagged-defect** tests use a real-fixture *fault injection*
//! (the project's established pattern — see `tests/pcr_restamp.rs`'s
//! `corrupt_pcrs`): clear just the `discontinuity_indicator` bit at packet
//! 1251, leaving the genuine ~10 s PCR jump (100% real capture data) with no
//! indicator — the exact "defect" scenario ISO/IEC 13818-1 §2.4.3.5 /
//! ETSI TR 101 290 §5.2.2 indicator 2.3b describes.

use dvb_conformance::{ConformanceMonitor, Indicator};
use ts_fix::discontinuity::detect_pcr_discontinuities;
use ts_fix::{PcrRestamp, TsFix};

const PKT: usize = 188;
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/ts/france-pcr-discontinuity.ts"
);
const CLEAN_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");

/// Independently-determined offset of the fixture's only PCR discontinuity
/// (documented in the module doc above).
const DISC_PACKET_INDEX: u64 = 1251;
const DISC_PID: u16 = 0x0208;
/// Byte offset of the adaptation-field flags byte within a 188-byte packet.
const AF_FLAGS_OFFSET: usize = 5;
/// `discontinuity_indicator` bit (ISO/IEC 13818-1 §2.4.3.5).
const AF_DISCONTINUITY: u8 = 0x80;

fn load(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1f) as u16) << 8) | p[2] as u16
}

/// `Some(pcr_27mhz)` if this packet carries a PCR in its adaptation field.
fn pcr_27mhz(p: &[u8]) -> Option<u64> {
    let afc = (p[3] >> 4) & 0x3;
    if afc != 2 && afc != 3 {
        return None;
    }
    if *p.get(4)? == 0 {
        return None;
    }
    if *p.get(5)? & 0x10 == 0 {
        return None;
    }
    let b = p.get(6..12)?;
    let base = ((b[0] as u64) << 25)
        | ((b[1] as u64) << 17)
        | ((b[2] as u64) << 9)
        | ((b[3] as u64) << 1)
        | ((b[4] as u64) >> 7);
    let ext = (((b[4] & 0x01) as u16) << 8) | (b[5] as u16);
    Some(base * 300 + ext as u64)
}

/// Clear `discontinuity_indicator` at the given packet index — real-fixture
/// fault injection simulating a genuine, unflagged break (the PCR jump values
/// themselves are 100% real capture data; only the legal signal is removed).
fn unflag_discontinuity(buf: &mut [u8], packet_index: u64) {
    let start = packet_index as usize * PKT;
    assert_eq!(
        buf[start + AF_FLAGS_OFFSET] & AF_DISCONTINUITY,
        AF_DISCONTINUITY,
        "packet {packet_index} must be flagged before fault injection"
    );
    buf[start + AF_FLAGS_OFFSET] &= !AF_DISCONTINUITY;
}

/// Run every 188-byte packet in `bytes` through a fresh `ConformanceMonitor`
/// (dvb-conformance's public API, used directly here so the assertion is
/// independent of ts-fix's own `discontinuity` module) and collect every
/// `PcrDiscontinuityError` (TR 101 290 §5.2.2 indicator 2.3b) event.
fn pcr_disc_errors(bytes: &[u8]) -> Vec<(usize, Option<u16>)> {
    let mut monitor = ConformanceMonitor::new();
    let mut out = Vec::new();
    for (i, p) in bytes.chunks_exact(PKT).enumerate() {
        let t = std::time::Duration::from_micros(i as u64);
        for event in monitor.feed(p, t) {
            if event.indicator == Indicator::PcrDiscontinuityError {
                out.push((i, event.pid));
            }
        }
    }
    out
}

fn run<F: FnOnce(ts_fix::TsFixBuilder) -> ts_fix::TsFixBuilder>(input: &[u8], cfg: F) -> Vec<u8> {
    let mut fix = cfg(TsFix::builder()).build().expect("build");
    let mut out = Vec::with_capacity(input.len());
    for p in input.chunks_exact(PKT) {
        let _ = fix.push(p, |o| out.extend_from_slice(o));
    }
    fix.finish(|o| out.extend_from_slice(o));
    out
}

// ── Detection ────────────────────────────────────────────────────────────

#[test]
fn detects_flagged_discontinuity_at_known_offset() {
    let input = load(FIXTURE);
    let found = detect_pcr_discontinuities(&input);

    // The fixture's one legal break: flagged, at the independently-determined
    // offset (see module doc).
    let flagged: Vec<_> = found.iter().filter(|d| d.flagged).collect();
    assert_eq!(flagged.len(), 1, "exactly one flagged break: {found:?}");
    assert_eq!(flagged[0].pid, DISC_PID);
    assert_eq!(flagged[0].packet_index, DISC_PACKET_INDEX);

    // No unflagged (genuine-defect) entries in the untouched real capture —
    // its only break is legally signalled.
    assert!(
        found.iter().all(|d| d.flagged),
        "untouched fixture must have no unflagged breaks: {found:?}"
    );
}

#[test]
fn detects_genuine_unflagged_discontinuity_after_fault_injection() {
    let mut input = load(FIXTURE);
    unflag_discontinuity(&mut input, DISC_PACKET_INDEX);

    // Sanity: the real PCR jump is still there (only the flag was cleared).
    let pcrs_before: Vec<u64> = load(FIXTURE)
        .chunks_exact(PKT)
        .filter(|p| pid(p) == DISC_PID)
        .filter_map(pcr_27mhz)
        .collect();
    let pcrs_after: Vec<u64> = input
        .chunks_exact(PKT)
        .filter(|p| pid(p) == DISC_PID)
        .filter_map(pcr_27mhz)
        .collect();
    assert_eq!(
        pcrs_before, pcrs_after,
        "fault injection must not touch PCR values"
    );

    let found = detect_pcr_discontinuities(&input);
    let unflagged: Vec<_> = found.iter().filter(|d| !d.flagged).collect();
    assert_eq!(
        unflagged.len(),
        1,
        "the fault-injected break must be detected as unflagged: {found:?}"
    );
    assert_eq!(unflagged[0].pid, DISC_PID);
    assert_eq!(unflagged[0].packet_index, DISC_PACKET_INDEX);

    // No packet is now (incorrectly) reported as flagged at that offset.
    assert!(
        !found
            .iter()
            .any(|d| d.flagged && d.packet_index == DISC_PACKET_INDEX),
        "the fault-injected packet must not also appear as flagged: {found:?}"
    );
}

// ── Restamp: eliminates the genuine unflagged break ─────────────────────

#[test]
fn restamp_clears_pcr_disc_on_fault_injected_break() {
    let mut faulted = load(FIXTURE);
    unflag_discontinuity(&mut faulted, DISC_PACKET_INDEX);

    // BITE: prove the fault-injected input really does trip TR 101 290 2.3b
    // (dvb-conformance's own check, called directly — not via ts-fix).
    let before = pcr_disc_errors(&faulted);
    assert!(
        !before.is_empty(),
        "fault-injected input must trip PCR_discontinuity_indicator_error"
    );
    assert!(
        before
            .iter()
            .any(|&(i, p)| i == DISC_PACKET_INDEX as usize && p == Some(DISC_PID)),
        "the 2.3b event must land on packet {DISC_PACKET_INDEX} / PID 0x{DISC_PID:04X}: {before:?}"
    );

    let restamped = run(&faulted, |b| b.restamp_pcr(PcrRestamp::interpolate()));
    assert_eq!(restamped.len(), faulted.len(), "packet count preserved");

    // After restamp: dvb-conformance's own PCR checks must be clean.
    let after = pcr_disc_errors(&restamped);
    assert!(
        after.is_empty(),
        "restamp must leave no PCR_discontinuity_indicator_error: {after:?}"
    );

    // PCR must be monotonic (27 MHz, no wrap in this short slice) across the
    // former break on PID 0x0208.
    let outp: Vec<u64> = restamped
        .chunks_exact(PKT)
        .filter(|p| pid(p) == DISC_PID)
        .filter_map(pcr_27mhz)
        .collect();
    assert_eq!(outp.len(), 5, "5 PCRs on PID 0x0208");
    for w in outp.windows(2) {
        assert!(
            w[1] > w[0],
            "PCR monotonic across former break: {} !> {}",
            w[1],
            w[0]
        );
    }
}

// ── Honor mode: byte-exact except the flag bit ──────────────────────────

#[test]
fn honor_mode_sets_flag_and_changes_nothing_else() {
    let original = load(FIXTURE);
    let mut faulted = original.clone();
    unflag_discontinuity(&mut faulted, DISC_PACKET_INDEX);
    assert_ne!(
        faulted, original,
        "fault injection must actually change a byte"
    );

    let honored = run(&faulted, |b| b.honor_pcr_discontinuity());
    assert_eq!(honored.len(), faulted.len(), "packet count preserved");

    // Setting the flag back byte-for-byte reconstructs the original capture.
    assert_eq!(
        honored, original,
        "honor mode must exactly restore the only bit fault injection cleared"
    );

    // Prove the diff is EXACTLY one bit at one byte (independent of the
    // round-trip-to-original check above): walk every byte of every packet.
    let mut differing_bytes = Vec::new();
    for (i, (f, h)) in faulted
        .chunks_exact(PKT)
        .zip(honored.chunks_exact(PKT))
        .enumerate()
    {
        for (off, (&fb, &hb)) in f.iter().zip(h.iter()).enumerate() {
            if fb != hb {
                differing_bytes.push((i, off, fb, hb));
            }
        }
    }
    assert_eq!(
        differing_bytes,
        vec![(
            DISC_PACKET_INDEX as usize,
            AF_FLAGS_OFFSET,
            0x10, // PCR flag only (discontinuity cleared by fault injection)
            0x90, // PCR flag + discontinuity_indicator restored
        )],
        "honor mode must change exactly one byte (the AF flags byte) on exactly one packet"
    );
}

#[test]
fn honor_mode_is_noop_when_break_is_already_flagged() {
    // The untouched real fixture already legally flags its only break, so
    // honor mode (which only acts on genuine, UNFLAGGED breaks) must not
    // change anything.
    let input = load(FIXTURE);
    let out = run(&input, |b| b.honor_pcr_discontinuity());
    assert_eq!(
        out, input,
        "honor mode must not touch an already-flagged break"
    );
}

// ── Hard gate: no-op on a clean, discontinuity-free stream ──────────────

#[test]
fn restamp_is_noop_on_clean_stream() {
    let input = load(CLEAN_FIXTURE);
    assert!(!input.is_empty(), "clean fixture must be non-empty");
    assert!(
        pcr_disc_errors(&input).is_empty(),
        "clean fixture must not already trip PCR_discontinuity_indicator_error"
    );

    let out = run(&input, |b| b.restamp_pcr(PcrRestamp::interpolate()));
    assert_eq!(
        out, input,
        "restamp must not perturb a stream with no PCR discontinuity"
    );
}

#[test]
fn honor_mode_is_noop_on_clean_stream() {
    let input = load(CLEAN_FIXTURE);
    let out = run(&input, |b| b.honor_pcr_discontinuity());
    assert_eq!(
        out, input,
        "honor mode must not perturb a stream with no PCR discontinuity"
    );
}
