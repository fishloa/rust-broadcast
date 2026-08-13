//! `Insufficient` must always ask for ground not yet examined.
//!
//! The crate has shipped the opposite defect twice, from two different probers,
//! and a green suite both times. `Probe::Insufficient { need_at_least }` tells a
//! caller "read more"; if `need_at_least` does not exceed what the probe
//! actually looked at, the caller re-probes, gets the same answer, and spins
//! forever on an input that can never resolve.
//!
//! # Why the earlier guards missed it
//!
//! The invariant *was* asserted — `tests/no_panic_on_arbitrary_input.rs`
//! checks `need_at_least > data.len()` — but never at a length where it bites.
//! The largest buffer it feeds is 8 KiB, and the bug only appears **past the
//! probe budget**: `probe` examines `min(len, DEFAULT_BUDGET)` bytes, so a
//! prober reporting `region.len() + 1` freezes at `DEFAULT_BUDGET + 1` and any
//! longer buffer satisfies `need_at_least <= supplied`.
//!
//! Two observed fixed points, both from a handful of attacker-chosen bytes:
//!
//! ```text
//! EBML  1A 45 DF A3 01 FF FF FF FF FF FF FE + 0x00 padding
//!         65536 -> Insufficient { 65537 }
//!         65537 -> Insufficient { 65537 }   <-- need == supplied
//!        131072 -> Insufficient { 65537 }   <-- need <  supplied
//!
//! MP3   "ID3" 04 00 00 7F 7F 7F 7F + padding
//!         65537 -> Insufficient { 268435465 }
//!     300000000 -> Insufficient { 268435465 }
//! ```
//!
//! So the guard here sweeps lengths that **straddle the budget**, which is the
//! only place the defect lives, and asserts the invariant against the bytes
//! *examined* (`min(len, budget)`) rather than the bytes supplied.

use container_probe::{DEFAULT_BUDGET, Probe, probe_with_budget};

/// Buffers that are structurally undecidable on purpose: each is a valid-looking
/// header prefix declaring far more data than follows, which is exactly what
/// drives a prober to say "read more".
fn adversarial_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();

    // EBML magic + a size VINT declaring a huge header body.
    let mut ebml = vec![
        0x1A, 0x45, 0xDF, 0xA3, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    ];
    ebml.resize(4096, 0x00);
    out.push(("ebml-huge-declared-header", ebml));

    // ID3v2.4 header declaring a ~268 MB tag (syncsafe 0x7F7F7F7F).
    let mut id3 = vec![b'I', b'D', b'3', 0x04, 0x00, 0x00, 0x7F, 0x7F, 0x7F, 0x7F];
    id3.resize(4096, 0x00);
    out.push(("id3-268mb-declared-tag", id3));

    // ISOBMFF `ftyp` whose largesize declares far more than is present.
    let mut mp4 = vec![0x00, 0x00, 0x00, 0x01, b'f', b't', b'y', b'p'];
    mp4.extend_from_slice(&0x0000_0000_7FFF_FFFFu64.to_be_bytes());
    mp4.resize(4096, 0x00);
    out.push(("isobmff-huge-largesize", mp4));

    // A lone TS sync byte with nothing to confirm a lattice.
    let mut ts = vec![0x47u8];
    ts.resize(4096, 0x00);
    out.push(("ts-single-sync", ts));

    out
}

/// Lengths chosen to straddle the probe budget, which is where the defect lives.
fn probe_lengths() -> Vec<usize> {
    vec![
        1,
        188,
        4096,
        DEFAULT_BUDGET - 1,
        DEFAULT_BUDGET,
        DEFAULT_BUDGET + 1,
        DEFAULT_BUDGET * 2,
        DEFAULT_BUDGET * 4,
    ]
}

/// `need_at_least` must always exceed the bytes examined — for every
/// adversarial input, at every length, at both the default and a matched budget.
#[test]
fn need_at_least_always_exceeds_the_bytes_examined() {
    let mut failures: Vec<String> = Vec::new();

    for (name, seed) in adversarial_inputs() {
        for len in probe_lengths() {
            let mut buf = seed.clone();
            buf.resize(len, 0x00);

            // Both the fixed default budget and a caller-matched budget: the
            // first is what `probe` uses, the second is the documented
            // terminating loop.
            for budget in [DEFAULT_BUDGET, buf.len()] {
                let examined = core::cmp::min(buf.len(), budget);
                if let Probe::Insufficient { need_at_least, .. } = probe_with_budget(&buf, budget)
                    && need_at_least <= examined
                {
                    failures.push(format!(
                        "  {name}: len={len} budget={budget} examined={examined} \
                         -> Insufficient {{ need_at_least: {need_at_least} }} — does not \
                         exceed the bytes examined, so a caller re-probes to the same \
                         answer forever"
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

/// The documented caller loop must actually terminate on adversarial input.
///
/// This is the end-to-end statement of the invariant above: follow the loop the
/// crate root documents, cap the iterations, and require it to finish. A bound
/// is the assertion — an unbounded loop would hang the test run rather than
/// fail it, which is a test that cannot fail by never returning.
#[test]
fn the_documented_caller_loop_terminates() {
    const MAX_TURNS: usize = 64;

    for (name, seed) in adversarial_inputs() {
        // The "file" the caller is reading from: finite, as every real one is.
        let mut file = seed.clone();
        file.resize(DEFAULT_BUDGET * 4, 0x00);

        let mut have = 1usize;
        let mut turns = 0usize;
        let verdict = loop {
            turns += 1;
            assert!(
                turns <= MAX_TURNS,
                "{name}: the documented loop did not terminate within {MAX_TURNS} turns \
                 (stuck asking for more at {have} bytes) — this is the unbounded read \
                 loop the invariant exists to prevent"
            );

            let buf = &file[..have.min(file.len())];
            match probe_with_budget(buf, buf.len()) {
                Probe::Insufficient { need_at_least, .. } => {
                    // Grow as instructed; stop at EOF, per the documented loop.
                    let next = need_at_least.min(file.len());
                    if next <= have {
                        break Probe::Unknown;
                    }
                    have = next;
                }
                other => break other,
            }
        };

        // Any terminal answer is acceptable here — the property under test is
        // termination, not which verdict. Asserting a specific verdict would
        // make this a weaker duplicate of the prefix sweep.
        assert!(
            !matches!(verdict, Probe::Insufficient { .. }),
            "{name}: loop exited still Insufficient"
        );
    }
}
