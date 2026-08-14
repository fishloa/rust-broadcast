//! `Insufficient` must ask for ground not yet examined, and must converge.
//!
//! The crate has shipped a non-converging `Insufficient` twice, from two
//! different probers, with a green suite both times.
//! `Probe::Insufficient { need_at_least }` tells a caller "read more"; if the
//! answer does not advance, the caller re-probes forever.
//!
//! # This file's own history, because it is the point
//!
//! The first version of this test **guarded nothing**. An audit reverted the
//! `ebml` fix — it passed. It neutered `normalise_need` to the identity — it
//! passed. Two of its four "adversarial" inputs never reached the code path at
//! all (`isobmff-huge-largesize` was `Identified` at every length ≥188;
//! `ts-single-sync` was `Unknown` at every length ≥4096), so they asserted
//! nothing about `Insufficient`. And its loop contained
//!
//! ```text
//! let next = need_at_least.min(file.len());
//! if next <= have { break Probe::Unknown; }   // <-- swallows the bug
//! ```
//!
//! where `next <= have` *is* the fixed-point condition. The test broke out of
//! the very failure it existed to detect and called it success.
//!
//! So this version enforces three things the first did not:
//!
//! 1. **Every input must actually reach `Insufficient`** at some length. An
//!    input that never triggers the path cannot witness a defect in it, and
//!    silently contributes nothing.
//! 2. **No swallowing.** A need that fails to advance is the failure, asserted
//!    directly — not a `break`.
//! 3. **Convergence, not merely termination.** A need derived from the buffer
//!    length rather than the structure still terminates, one byte at a time;
//!    over 256 KiB that was 1361 turns. The turn budget is therefore tight and
//!    scaled to the structure, so linear crawling fails.

use container_probe::{DEFAULT_BUDGET, Probe, probe_with_budget};

/// Inputs that must drive some prober to say "read more".
///
/// Each is checked below to actually do so; a seed that stops reaching the
/// path is a failure of this file, not a silent pass.
fn adversarial_inputs() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        // EBML magic + a size VINT declaring a header body far larger than the
        // buffer. Genuinely truncated: the declared end is the structural need.
        (
            "ebml-declared-body-past-region",
            vec![
                0x1A, 0x45, 0xDF, 0xA3, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
            ],
        ),
        // ID3v2.4 declaring a ~268 MB tag: an honest structural need that
        // exceeds the probe budget.
        (
            "id3-268mb-declared-tag",
            vec![b'I', b'D', b'3', 0x04, 0x00, 0x00, 0x7F, 0x7F, 0x7F, 0x7F],
        ),
        // A single TS sync byte: too little to confirm or rule out a lattice.
        ("ts-single-sync", vec![0x47]),
        // Two ADTS frames of VERY different declared lengths (7, then 8191).
        // A uniform-frame seed cannot expose a need computed from the buffer:
        // the bound that hides it uses the FIRST frame's length while
        // truncation is set by the LAST. This shape was constructed by an audit
        // specifically to disprove a claim that no such input existed.
        (
            "adts-mixed-frame-lengths",
            vec![
                0xFF, 0xF1, 0x00, 0x00, 0x00, 0xE0, 0x00, 0xFF, 0xF1, 0x00, 0x03, 0xFF, 0xE0, 0x00,
            ],
        ),
        // ISOBMFF box header cut mid-largesize.
        (
            "isobmff-cut-largesize",
            vec![0x00, 0x00, 0x00, 0x01, b'f', b't', b'y', b'p', 0x00, 0x00],
        ),
    ]
}

/// Pad `seed` to `len` with zeros.
fn at(seed: &[u8], len: usize) -> Vec<u8> {
    let mut b = seed.to_vec();
    b.resize(len, 0x00);
    b
}

/// Precondition for everything else here: each seed must reach `Insufficient`
/// at *some* length, or it is dead weight that cannot witness a defect.
///
/// This is the guard the first version of this file lacked, and it is why half
/// its inputs were inert without anyone noticing.
#[test]
fn every_adversarial_input_actually_reaches_insufficient() {
    for (name, seed) in adversarial_inputs() {
        let reached = [seed.len().max(1), 64, 1024, 4096, DEFAULT_BUDGET]
            .into_iter()
            .any(|len| {
                matches!(
                    probe_with_budget(&at(&seed, len), len),
                    Probe::Insufficient { .. }
                )
            });
        assert!(
            reached,
            "{name} never probes Insufficient at any tested length, so it cannot \
             witness a defect in the Insufficient contract — it is inert, and an \
             inert input makes every other assertion in this file vacuous for it"
        );
    }
}

/// `need_at_least` must always exceed the bytes **examined** — which is
/// `min(len, budget)`, not `len`. Swept across the budget boundary, the only
/// place the fixed point lived.
#[test]
fn need_at_least_always_exceeds_the_bytes_examined() {
    let lengths = [
        1,
        188,
        4096,
        DEFAULT_BUDGET - 1,
        DEFAULT_BUDGET,
        DEFAULT_BUDGET + 1,
        DEFAULT_BUDGET * 2,
    ];
    let mut failures: Vec<String> = Vec::new();

    for (name, seed) in adversarial_inputs() {
        for len in lengths {
            let buf = at(&seed, len);
            for budget in [DEFAULT_BUDGET, buf.len()] {
                let examined = core::cmp::min(buf.len(), budget);
                if let Probe::Insufficient { need_at_least, .. } = probe_with_budget(&buf, budget)
                    && need_at_least <= examined
                {
                    failures.push(format!(
                        "  {name}: len={len} budget={budget} examined={examined} -> \
                         need_at_least={need_at_least}, which does not exceed the bytes \
                         examined; a caller re-probes to the same answer forever"
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Insufficient must always ask for ground not yet examined:\n{}",
        failures.join("\n")
    );
}

/// The documented caller loop must **converge**, not merely finish.
///
/// Every turn must strictly advance, and the number of turns must stay tiny —
/// a structural need lands on the answer in a handful of reads, whereas a need
/// derived from the buffer length crawls. The tight budget is the assertion
/// that distinguishes them; a generous one would pass for both.
#[test]
fn the_documented_caller_loop_converges() {
    // The bound is derived, not guessed, and its job is to separate geometric
    // convergence from arithmetic crawling over this file size:
    //
    //   1.5x growth from 1 byte  ->   31 turns   (what the crate must do)
    //   arithmetic +188 (TS)     -> 1394 turns
    //   arithmetic +4  (Annex B) -> 65536 turns
    //
    // 48 clears the first with margin and rejects both of the others outright.
    // A generous bound (say 5000) would pass the TS crawl, and a bound tuned to
    // today's exact turn count would fail on any harmless future change — the
    // point is the growth *class*, not the count.
    const MAX_TURNS: usize = 48;
    const FILE_LEN: usize = DEFAULT_BUDGET * 4;

    for (name, seed) in adversarial_inputs() {
        let file = at(&seed, FILE_LEN);
        let mut have = 1usize;
        let mut turns = 0usize;

        let verdict = loop {
            turns += 1;
            assert!(
                turns <= MAX_TURNS,
                "{name}: the documented loop had not converged after {MAX_TURNS} turns \
                 (at {have} of {FILE_LEN} bytes). A need derived from the buffer length \
                 rather than the structure terminates but crawls; that is the defect."
            );

            let buf = &file[..have];
            match probe_with_budget(buf, buf.len()) {
                Probe::Insufficient { need_at_least, .. } => {
                    // NOT a `break`. A need that fails to advance is the fixed
                    // point this file exists to catch, so assert it directly —
                    // breaking here is how the previous version passed while
                    // the bug was live.
                    assert!(
                        need_at_least > have,
                        "{name}: at {have} bytes the probe asked for {need_at_least} — \
                         no advance, so the caller loops forever on this input"
                    );
                    // Clamp to EOF and probe the WHOLE file before giving up.
                    // Breaking here on `need >= file.len()` would be wrong: the
                    // floor deliberately over-asks when no structure can be
                    // named, so a need past EOF does not mean the file is
                    // undecidable — only that we have not yet looked at all of
                    // it. Give up only when a full-file probe still says
                    // Insufficient.
                    let next = need_at_least.min(file.len());
                    if next <= have {
                        assert_eq!(
                            have,
                            file.len(),
                            "{name}: stopped advancing at {have} without having read the \
                             whole {FILE_LEN}-byte file"
                        );
                        break Probe::Unknown;
                    }
                    have = next;
                }
                other => break other,
            }
        };

        assert!(
            !matches!(verdict, Probe::Insufficient { .. }),
            "{name}: loop exited still Insufficient"
        );
    }
}
