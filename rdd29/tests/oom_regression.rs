//! Regression guard: a Plex count read from the wire must never drive an
//! unbounded allocation.
//!
//! Found by the CI fuzz run. AddressSanitizer aborted the `rdd29` target with:
//!
//! ```text
//! ==5560==ERROR: AddressSanitizer: out of memory:
//! allocator is trying to allocate 0x37cbb80000 bytes
//! ```
//!
//! `0x37cbb80000` is 239.8 GB. `ATMOSFrame.SubElementCount` is a Plex field
//! read straight from the input, so it is attacker-controlled and effectively
//! unbounded — Plex escalates 8 → 16 → 32 bits, so twelve bytes encode a count
//! of `0x7FFF_FFFF`. `Vec::with_capacity` then reserved that many elements.
//!
//! Every sub-element consumes at least one byte of the body, so the count can
//! never legitimately exceed the bytes remaining, so the parser **rejects** an
//! impossible count outright rather than capping the reserve -- capping is
//! untestable here, because `Vec::with_capacity(2^31)` succeeds under ordinary
//! overcommit and only a sanitizer notices. `BedDefinition1.ChannelCount` had
//! the identical hazard and the identical fix, with its own ceiling of
//! `remaining_bits / 15` (a channel description costs at least 15 bits).
//!
//! These inputs are malformed and must fail to parse — the point is that they
//! fail *without attempting the allocation first*. A test process that
//! survives is the assertion.

use broadcast_common::Parse;
use rdd29::{AtmosFrame, BedDefinition1};

/// The exact shape the fuzzer found, reconstructed from the format rather than
/// copied from a crash artifact.
///
/// ```text
/// 08                    ElementID   = ATMOSFrame
/// 0a                    ElementSize = 10 (the body below)
/// 00                    ATMOSVersion
/// 00                    SampleRate | BitDepth | FrameRate
/// 00                    MaxRendered = 0
/// ff ff ff 7f ff ff ff  SubElementCount: Plex escalates 8 -> 16 -> 32,
///                       decoding to 0x7FFF_FFFF = 2_147_483_647
/// ```
///
/// At ~112 bytes per `AnyElement` that reserve is ~240 GB, matching the
/// sanitizer's figure.
///
/// MUTATION VERIFIED: reverting `atmos_frame.rs` to
/// `Vec::with_capacity(sub_element_count as usize)` makes this test abort the
/// process with a memory-allocation failure.
#[test]
fn a_huge_plex_sub_element_count_does_not_reserve() {
    let data: [u8; 12] = [
        0x08, 0x0a, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff,
    ];
    // The count must be REJECTED by name, not merely capped. Capping alone is
    // untestable here: `Vec::with_capacity(2^31)` succeeds under ordinary
    // overcommit, so only AddressSanitizer would notice. Asserting the specific
    // error is what makes this guard able to fail.
    match AtmosFrame::parse(&data) {
        Err(rdd29::Error::InvalidValue { field, value, .. }) => {
            assert_eq!(field, "ATMOSFrame.SubElementCount");
            assert_eq!(value, 0x7FFF_FFFF);
        }
        other => panic!(
            "a frame declaring 2^31 sub-elements in a 10-byte body must be \
             rejected as an invalid SubElementCount, got {other:?}"
        ),
    }
}

/// The same hazard reached through the arbitrary short inputs the fuzzer
/// actually explores, including every prefix of the crafted frame above —
/// with **real** assertions, not just "the process survived" (which only a
/// sanitizer could ever fail). Every prefix of the crafted frame is
/// malformed (truncated before the huge count's terminal 32-bit value), so it
/// must all return `Err` — none may parse successfully, because a success
/// would mean the huge `SubElementCount` was accepted and the body reserve
/// driven from it.
#[test]
fn arbitrary_short_inputs_do_not_reserve() {
    let crafted: [u8; 12] = [
        0x08, 0x0a, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff,
    ];
    for n in 0..=crafted.len() {
        assert!(
            AtmosFrame::parse(&crafted[..n]).is_err(),
            "prefix of len {n} (raw {crafted:02x?}) must not parse: \
             it is a truncated massive-SubElementCount body"
        );
    }
    // The all-ones fill escalates the Plex escape code past the end of the
    // input, so it must fail too — never silently succeed while the count is
    // attacker-controlled.
    let all_ones = vec![0xFFu8; 16];
    assert!(
        AtmosFrame::parse(&all_ones).is_err(),
        "an all-ones body must fail, not accept an escaped Plex count"
    );
    for len in 0..64usize {
        for fill in [0x00u8, 0xAA, 0x7F, 0x08] {
            let _ = AtmosFrame::parse(&vec![fill; len]);
        }
    }
}

/// `BedDefinition1.ChannelCount` has the identical Plex hazard as
/// `ATMOSFrame.SubElementCount`, but the fix is a **tight** provable ceiling:
/// each channel consumes at least 15 bits (Plex(4) `ChannelID` + Plex(8)
/// `AudioDataID` + 3 reserved bits), so `remaining_bits / 15` — not
/// `remaining_bits` — is the most channels the element can possibly hold.
///
/// This input declares 8 channels in a 112-bit element. After the 13-bit
/// header (Plex(8) MetaID = 0x00, 1 reserved bit, Plex(4) ChannelCount = 8)
/// there are 99 bits left, whose provable ceiling is `99 / 15 = 6`. The count
/// 8 sits strictly between that ceiling and the raw remaining bits (8 <= 99),
/// so:
///
/// * the `/ 15` bound REJECTS it as `InvalidValue` (this test's assertion), and
/// * a **loose** `remaining_bits` bound lets it through — the parser then reads
///   the 8 channels from only 99 bits and fails mid-loop with an unrelated bit
///   error (or, for a genuinely huge Plex count that the loose bound also
///   admits when the element is large, reserves unboundedly).
///
/// MUTATION VERIFIED, recorded verbatim: reverting `bed_definition.rs` to
/// `channel_count > remaining_bits` (the loose first-round bound) makes this
/// test FAIL with:
///
///     8 channels in a 112-bit element (provable ceiling 6) must be rejected as an invalid ChannelCount, got Err(Bits { what: "BedDefinition1.AudioDataID", source: OutOfBounds { needed_bits: 8, remaining_bits: 5 } })
///
/// — the parser runs past the impossible reserve and dies mid-channel with an
/// unrelated bit error instead of rejecting the count by name. Restoring
/// `/ 15` (and a `touch`) makes it pass again.
#[test]
fn a_channel_count_over_the_provable_ceiling_is_rejected() {
    // byte0: MetaID Plex(8) = 0. byte1: reserved(1) = 0, ChannelCount
    // Plex(4) = 8 (0b1000), then filler. 14 bytes = 112 bits total.
    let data: [u8; 14] = [
        0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    match BedDefinition1::parse(&data) {
        Err(rdd29::Error::InvalidValue { field, value, .. }) => {
            assert_eq!(field, "BedDefinition1.ChannelCount");
            assert_eq!(value, 8);
        }
        other => panic!(
            "8 channels in a 112-bit element (provable ceiling 6) must be \
             rejected as an invalid ChannelCount, got {other:?}"
        ),
    }
}
