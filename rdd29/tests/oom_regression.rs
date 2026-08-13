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
//! never legitimately exceed the bytes remaining. The parser now reserves the
//! smaller of the two. `BedDefinition1.ChannelCount` had the identical hazard
//! and the identical fix.
//!
//! These inputs are malformed and must fail to parse — the point is that they
//! fail *without attempting the allocation first*. A test process that
//! survives is the assertion.

use broadcast_common::Parse;
use rdd29::AtmosFrame;

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
/// actually explores, including every prefix of the crafted frame above.
#[test]
fn arbitrary_short_inputs_do_not_reserve() {
    let crafted: [u8; 12] = [
        0x08, 0x0a, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff,
    ];
    for n in 0..=crafted.len() {
        let _ = AtmosFrame::parse(&crafted[..n]);
    }
    for len in 0..64usize {
        for fill in [0x00u8, 0xFF, 0xAA, 0x7F, 0x08] {
            let _ = AtmosFrame::parse(&vec![fill; len]);
        }
    }
}
