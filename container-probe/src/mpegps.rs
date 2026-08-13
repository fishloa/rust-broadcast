//! MPEG Program Stream prober — ISO/IEC 13818-1 §2.5.3 (Pack Header, Table 2-39).
//!
//! A Program Stream begins with a Pack Header: `pack_start_code` `00 00 01 BA`,
//! a 6-byte System Clock Reference field (with a `'01'` prefix and four
//! `marker_bit`s), a 22-bit `program_mux_rate`, marker bits at byte 12 `[1:0]`,
//! and a reserved/stuffing byte. Layout per `mpeg-ps/src/pack_header.rs` and
//! `mpeg-ps/src/scr.rs`, which parse this against real fixtures.
//!
//! - Start code **plus** valid marker bits -> `STRUCTURAL`.
//! - Start code alone with marker bits that do not validate -> `HEURISTIC`.
//! - `Detail::None`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The MPEG-PS pack start code (ISO/IEC 13818-1 §2.5.3.2, `00 00 01 BA`).
const PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
/// The `'01'` prefix bits at the top of the SCR field's first byte — the
/// `marker_bits`/prefix make the pack distinguishable from a bare start code.
/// Valid prefixes have top two bits `01` (`0x40`..`0x7F` after the marker bits).
const SCR_PREFIX_MASK: u8 = 0xC0;
const SCR_PREFIX_EXPECTED: u8 = 0x40;
/// SCR `marker_bit` positions within the 6-byte SCR field (offset by the
/// pack's byte 4): `[0][2]`, `[2][2]`, `[4][2]`, `[5][0]` (mpeg-ps/src/scr.rs).
const SCR_MARKER_BITS: [(usize, u8); 4] = [
    (0, 0x04), // SCR byte 0 bit 2  (pack byte 4)
    (2, 0x04), // SCR byte 2 bit 2  (pack byte 6)
    (4, 0x04), // SCR byte 4 bit 2  (pack byte 8)
    (5, 0x01), // SCR byte 5 bit 0  (pack byte 9)
];
/// Offset of the SCR field within the pack header (after `pack_start_code`).
const SCR_OFFSET: usize = 4;
/// Byte 12 of the pack header, whose `[1:0]` bits are the two
/// `program_mux_rate` marker bits and must be `11` (ISO/IEC 13818-1 §2.5.3.3).
const MUX_RATE_MARKERS_BYTE: usize = 12;
/// The `[1:0]` marker mask on byte 12.
const MUX_RATE_MARKERS_MASK: u8 = 0x03;
/// The pack header fixed length before any stuffing (14 bytes:
/// start code + SCR + 3 mux bytes + reserved/stuffing byte).
const PACK_HEADER_MIN_LEN: usize = 14;

/// The registered MPEG-PS prober: pack start code + marker validation.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if region.len() < PACK_HEADER_MIN_LEN {
        // Shorter than a full pack header: cannot reach the marker bits, so a
        // truncated .ps read a few bytes at a time is undecided (`Insufficient`),
        // not `Unknown`.
        return Outcome::Insufficient(PACK_HEADER_MIN_LEN);
    }
    if region[..PACK_START_CODE.len()] != PACK_START_CODE {
        return Outcome::None;
    }

    let detail = Detail::None;
    Outcome::Match(Evidence {
        confidence: if markers_valid(region) {
            Confidence::STRUCTURAL
        } else {
            Confidence::HEURISTIC
        },
        detail,
    })
}

/// `true` when every marker bit in the Pack Header is set, giving a structurally
/// plausible MPEG-PS pack (ISO/IEC 13818-1 §2.5.3.3; mpeg-ps/src/pack_header.rs,
/// mpeg-ps/src/scr.rs).
fn markers_valid(p: &[u8]) -> bool {
    // '01' prefix on the SCR field.
    if p[SCR_OFFSET] & SCR_PREFIX_MASK != SCR_PREFIX_EXPECTED {
        return false;
    }
    let scr = &p[SCR_OFFSET..];
    for &(byte_off, bit) in &SCR_MARKER_BITS {
        if scr[byte_off] & bit == 0 {
            return false;
        }
    }
    // The two program_mux_rate marker bits at byte 12 [1:0] are both 1.
    if p[MUX_RATE_MARKERS_BYTE] & MUX_RATE_MARKERS_MASK != MUX_RATE_MARKERS_MASK {
        return false;
    }
    true
}

#[cfg(test)]
mod drift {
    //! Pins this module's `PACK_START_CODE` to `mpeg-ps`.
    //!
    //! Lives here rather than in `tests/drift_guard.rs` because the constant is
    //! private: an integration test can only compare upstream against a
    //! literal, which catches upstream changing but not this crate's copy
    //! drifting. A unit test sees the real constant.

    /// `mpeg-ps` keeps its own `PACK_START_CODE` in a private module
    /// (`mpeg-ps/src/pack_header.rs`) and does not re-export it, so the exact
    /// 4-byte value cannot be compared. Its public
    /// `PACKET_START_CODE_PREFIX` is the `00 00 01` that opens every
    /// pack/system/PSM start code — pin our first three bytes to it. The `BA`
    /// suffix is ISO/IEC 13818-1 §2.5.3.3 and is asserted here as a literal
    /// because no public upstream constant carries it.
    #[test]
    fn pack_start_code_prefix_matches_mpeg_ps() {
        assert_eq!(
            super::PACK_START_CODE[..3],
            mpeg_ps::PACKET_START_CODE_PREFIX[..],
            "container-probe's PACK_START_CODE prefix has drifted from mpeg-ps's"
        );
        assert_eq!(
            super::PACK_START_CODE[3],
            0xBA,
            "pack_start_code must end in BA (ISO/IEC 13818-1 §2.5.3.3)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// Finding 4: a 13-byte prefix of a real MPEG-PS fixture (1 byte short of
    /// the 14-byte pack header) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/ps/h264_ac3.ps");
        let region = &data[..PACK_HEADER_MIN_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, PACK_HEADER_MIN_LEN),
            other => panic!("13-byte MPEG-PS prefix must be Insufficient(14), got {other:?}"),
        }
    }
}
