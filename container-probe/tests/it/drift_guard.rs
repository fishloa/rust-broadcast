//! Constant drift guard (WP4, section 1).
//!
//! This crate re-declares constants that other workspace crates already
//! validate against real fixtures. The guard pins those copies so a future
//! edit cannot silently diverge — if an upstream constant ever changes, the
//! assertion here fails loudly and this crate's own copy must be re-reviewed.
//!
//! **This file guards ONE direction only: that upstream still declares what we
//! were written against.** The prober constants are private (by design — prober
//! internals are not public API), so an integration test can only compare an
//! upstream constant against a literal. That cannot catch this crate's own copy
//! drifting, which is the direction that matters more.
//!
//! The other direction lives in a `#[cfg(test)] mod drift` inside each prober
//! module (`src/ts.rs`, `src/mpegps.rs`, `src/mxf.rs`), where the private
//! constant is visible and is compared to the upstream one directly. Verified
//! necessary: editing `ts.rs`'s `TS_SYNC_BYTE` to `0x48` leaves every test in
//! THIS file green while `ts::drift::sync_byte_matches_mpeg_ts` turns red.
//!
//! Neither direction substitutes for the other; both are required.

/// Upstream `mpeg-ts` still declares the values this crate's TS prober was
/// written against.
///
/// **This asserts upstream against a literal, so it catches `mpeg-ts` changing
/// but NOT this crate's own copy drifting** — an integration test cannot see
/// `container-probe/src/ts.rs`'s private constants. The guard for that
/// direction is the `ts::drift` unit-test module inside `src/ts.rs`, which
/// compares the real constants to each other.
///
/// Both directions are needed and neither substitutes for the other: verified
/// by editing `TS_SYNC_BYTE` to `0x48`, which leaves this test green and turns
/// `ts::drift::sync_byte_matches_mpeg_ts` red.
#[test]
fn upstream_mpeg_ts_still_declares_the_expected_values() {
    assert_eq!(
        mpeg_ts::ts::TS_SYNC_BYTE,
        0x47,
        "mpeg-ts changed its sync byte; re-review container-probe/src/ts.rs"
    );
    assert_eq!(
        mpeg_ts::ts::TS_PACKET_SIZE,
        188,
        "mpeg-ts changed its packet size; re-review container-probe/src/ts.rs"
    );
}

/// The MPEG-PS `pack_start_code` (`0x000001BA`), our copy at
/// `container-probe/src/mpegps.rs` (`const PACK_START_CODE: [u8; 4] =
/// [0x00, 0x00, 0x01, 0xBA];`).
///
/// `mpeg-ps` keeps its own `PACK_START_CODE` in a **private** module
/// (`pack_header`, `mpeg-ps/src/pack_header.rs:12` — `pub const PACK_START_CODE:
/// u32 = 0x0000_01BA;`) and does not re-export it, so it cannot be referenced
/// from here. The nearest public constant is the 3-byte
/// `packet_start_code_prefix` that opens every pack/system/PSM start code
/// (`00 00 01`), exported as `mpeg_ps::PACKET_START_CODE_PREFIX`; the `BA`
/// suffix turning it into the pack start code is pinned by the citation above.
/// That is the best indirect guard available without a `pub(crate)` widening.
#[test]
fn ps_pack_start_code_prefix_matches_mpeg_ps() {
    // This crate's MpegPs prober matches `00 00 01 BA`; the prefix is the
    // public `packet_start_code_prefix` (cited `container-probe/src/mpegps.rs`).
    assert_eq!(
        mpeg_ps::PACKET_START_CODE_PREFIX,
        [0x00, 0x00, 0x01],
        "MPEG-PS pack start-code prefix must match mpeg_ps::PACKET_START_CODE_PREFIX"
    );
}

/// The MXF partition-pack key prefix (`06 0E 2B 34 02 05 01`), our copy at
/// `container-probe/src/mxf.rs` (`const MXF_KEY_PREFIX`).
///
/// `st377-1`'s own `PARTITION_KEY_PREFIX` (its `src/partition.rs`) is private,
/// so there is no exact public constant to compare against. The nearest public
/// equivalent is the SMPTE Universal-Label organisation header that every MXF
/// KLV key shares — `06 0E 2B 34` — which is the first four bytes of both
/// `st377_1::op1a::OP1A_UL_PREFIX` and `st377_1::klv::FILL_ITEM_KEY_PREFIX`
/// (and of this crate's `MXF_KEY_PREFIX`). Asserting that shared prefix pins
/// the header against `st377-1` without a `pub(crate)` widening.
#[test]
fn mxf_key_prefix_header_matches_st377_1() {
    use st377_1::FILL_ITEM_KEY_PREFIX;
    use st377_1::op1a::OP1A_UL_PREFIX;
    // The "06 0E 2B 34" organisation header is common to every MXF UL.
    let ours: [u8; 4] = [0x06, 0x0E, 0x2B, 0x34];
    assert_eq!(
        ours,
        [
            OP1A_UL_PREFIX[0],
            OP1A_UL_PREFIX[1],
            OP1A_UL_PREFIX[2],
            OP1A_UL_PREFIX[3]
        ]
    );
    assert_eq!(
        ours,
        [
            FILL_ITEM_KEY_PREFIX[0],
            FILL_ITEM_KEY_PREFIX[1],
            FILL_ITEM_KEY_PREFIX[2],
            FILL_ITEM_KEY_PREFIX[3]
        ]
    );
    // Both upstream prefixes begin with the same 4-byte header, so they agree
    // with each other and with this crate's copy.
    assert_eq!(OP1A_UL_PREFIX[..4], FILL_ITEM_KEY_PREFIX[..4]);
}
