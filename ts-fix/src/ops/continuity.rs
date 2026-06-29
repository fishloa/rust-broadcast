//! Continuity counter repair operation.
//!
//! Renumbers the 4-bit `continuity_counter` per PID to a correct monotonic
//! sequence (mod 16), respecting the spec-defined exceptions per
//! ISO/IEC 13818-1 §2.4.3.3 / §2.4.3.5.
//!
//! # Spec semantics (§2.4.3.3 L1770–1774)
//!
//! - The counter increments (mod 16) **only** on payload-bearing packets
//!   (`adaptation_field_control` 01 or 11).  On adaptation-only (10) and
//!   reserved (00) packets, the CC shall NOT change.
//! - A packet MAY be sent **twice** with the **same** CC (duplicate).
//!   The duplicate packet carries the same payload bytes (except a
//!   re-encoded PCR) and same CC as the previous instance.  A repair
//!   MUST preserve these duplicates.
//! - A packet whose adaptation field has `discontinuity_indicator == 1`
//!   may legally have any CC value (§2.4.3.5 L1872).  A repair MUST NOT
//!   renumber CC across a signalled discontinuity.

use alloc::collections::BTreeMap;

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::{TsHeader, TS_PACKET_SIZE};

use crate::ops::{Op, StreamModel};

/// Bit mask for `discontinuity_indicator` in the adaptation field flags byte
/// (§2.4.3.5).  Equivalent to `mpeg_ts::ts::AF_DISCONTINUITY` (0x80), which is
/// `pub(crate)` in mpeg-ts.
const AF_DISCONTINUITY: u8 = 0x80;

/// Per-PID continuity-tracking state.
struct PidState {
    /// The CC we expect on the next payload-bearing packet (mod 16).
    expected: u8,
    /// The CC of the most recent payload-bearing packet (for duplicate
    /// detection).  This tracks the ORIGINAL wire CC, not the corrected
    /// value, so that a same-CC pair straddling a gap repair is still
    /// recognised.
    last_wire_cc: u8,
    /// A hash of the payload bytes of the most recent payload-bearing
    /// packet (used to identify legal duplicates, which have identical
    /// payload bytes except possibly re-encoded PCR).
    last_payload_hash: u64,
}

/// Continuity counter repair operation.
///
/// Per ISO/IEC 13818-1 §2.4.3.3, the 4-bit `continuity_counter` in the TS
/// header (byte 3, bits [3:0]) increments (mod 16) **only** on packets that
/// carry a payload (when `adaptation_field_control` is 01 or 11).  On
/// adaptation-only packets (10) or reserved/no-payload packets (00), the
/// counter does not increment.
///
/// Additionally (§2.4.3.3 L1772, §2.4.3.5 L1872):
/// - Duplicate packets (same CC, same payload) must be preserved
///   without renumbering.
/// - Packets with `discontinuity_indicator == 1` in their adaptation
///   field may have any CC value — the repair must not "fix" a signalled
///   discontinuity.
pub(crate) struct ContinuityOp {
    per_pid: BTreeMap<u16, PidState>,
}

impl ContinuityOp {
    /// Create a new continuity counter repair operation.
    pub(crate) fn new() -> Self {
        Self {
            per_pid: BTreeMap::new(),
        }
    }

    /// Extract the adaptation_field_control bits from byte 3.
    ///
    /// Returns the 2-bit `adaptation_field_control` value (ISO/IEC 13818-1
    /// §2.4.3.3, Table 2-5): 00=reserved, 01=payload-only, 10=adaptation-only,
    /// 11=both.
    #[inline]
    #[allow(dead_code)]
    fn afc(b3: u8) -> u8 {
        (b3 >> 4) & 0x03
    }

    /// Check whether a packet has `discontinuity_indicator == 1` in its
    /// adaptation field (§2.4.3.5).
    ///
    /// Only valid when `has_adaptation == true` AND `adaptation_field_length > 0`.
    /// `pkt` is the full 188-byte TS packet.
    fn has_discontinuity(pkt: &[u8]) -> bool {
        debug_assert!(pkt.len() == TS_PACKET_SIZE);
        let af_len = pkt[4] as usize;
        if af_len == 0 {
            return false;
        }
        // pkt[5] is the adaptation field flags byte; bit 7 = discontinuity_indicator.
        pkt[5] & AF_DISCONTINUITY != 0
    }

    /// Compute a hash of the payload bytes of a TS packet.
    ///
    /// The hash skips the PCR field (6 bytes at adaptation-field body offset 1)
    /// so that a duplicate with a re-encoded PCR still matches.  This is a
    /// best-effort heuristic — the spec says "bytes identical except a
    /// re-encoded valid PCR".
    fn payload_hash(pkt: &[u8]) -> u64 {
        debug_assert!(pkt.len() == TS_PACKET_SIZE);
        let hdr = match TsHeader::parse(&pkt[..4]) {
            Ok(h) => h,
            Err(_) => return 0,
        };

        if !hdr.has_payload {
            return 0;
        }

        // Build a hash of all non-PCR bytes: header + AF(no PCR) + payload.
        // We hash the 4-byte header, the AF bytes skipping the PCR if present,
        // and the payload.  Using a simple FNV-1a-like hash.
        let mut hash = 0xCBF29CE484222325u64;

        // Hash the 4-byte header.
        for &b in &pkt[..4] {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001B3);
        }

        if hdr.has_adaptation {
            let af_len = pkt[4] as usize;
            if af_len > 0 {
                // af_len counts the flags byte + optional fields + stuffing.
                // AF flags byte = pkt[5]; PCR (if present) occupies pkt[6..12].
                let has_pcr = (pkt[5] & 0x10) != 0;

                // Hash the flags byte (always present).
                hash ^= pkt[5] as u64;
                hash = hash.wrapping_mul(0x100000001B3);

                if has_pcr {
                    // Hash AF bytes that come after the 6-byte PCR field:
                    // pkt[12..5 + af_len].
                    let after_pcr_start = 12usize;
                    let af_body_end = 5 + af_len;
                    for &b in &pkt[after_pcr_start..af_body_end] {
                        hash ^= b as u64;
                        hash = hash.wrapping_mul(0x100000001B3);
                    }
                } else {
                    // No PCR — hash the rest of the AF body.
                    for &b in &pkt[6..5 + af_len] {
                        hash ^= b as u64;
                        hash = hash.wrapping_mul(0x100000001B3);
                    }
                }
            }
        }

        // Determine payload start.
        let mut payload_start = 4usize;
        if hdr.has_adaptation {
            let af_len = pkt[4] as usize;
            payload_start += 1 + af_len;
        }

        // Hash the payload bytes.
        for &b in &pkt[payload_start..TS_PACKET_SIZE] {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001B3);
        }

        hash
    }
}

impl Op for ContinuityOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            out(packet);
            return;
        }

        let header = match TsHeader::parse(&packet[..4]) {
            Ok(h) => h,
            Err(_) => {
                out(packet);
                return;
            }
        };

        let pid = header.pid;
        let current_cc = header.continuity_counter;
        let has_payload = header.has_payload;

        // §2.4.3.3: CC shall NOT increment on adaptation-only (10) or
        // reserved (00) packets.  Pass through unchanged; do NOT update
        // per-PID expected state.
        if !has_payload {
            out(packet);
            return;
        }

        // Payload-bearing packet (afc 01 or 11).

        // Check for signalled discontinuity (§2.4.3.5 L1872).
        // When discontinuity_indicator == 1, any CC value is legal.
        // We reset our per-PID expected state so subsequent packets are
        // not falsely flagged as errors.
        let is_discontinuity = header.has_adaptation && Self::has_discontinuity(packet);
        if is_discontinuity {
            // Remove existing per-PID state so the next packet starts
            // fresh.  Pass through unchanged.
            self.per_pid.remove(&pid);
            out(packet);
            return;
        }

        let payload_hash = Self::payload_hash(packet);

        let mut state_initialised = false;

        // Get or initialise per-PID state.  If this is the first payload-bearing
        // packet for this PID, we initialise with the observed CC and pass
        // through (no repair needed for the first occurrence).
        let state = self.per_pid.entry(pid).or_insert_with(|| {
            state_initialised = true;
            PidState {
                expected: current_cc,
                last_wire_cc: current_cc,
                last_payload_hash: payload_hash,
            }
        });

        if state_initialised {
            // First payload-bearing packet on this PID: pass through unchanged,
            // advance expected for next.
            let next = (current_cc + 1) & 0x0F;
            state.expected = next;
            out(packet);
            return;
        }

        // Check for legal duplicate (§2.4.3.3 L1772): a packet with the
        // same CC as the previous payload-bearing packet on this PID and
        // IDENTICAL payload (except a re-encoded PCR).  Per the spec a
        // "duplicate" has the same CC AND the same payload bytes (except
        // the PCR field which may be re-encoded).
        //
        // When preserving a duplicate we do NOT advance `state.expected` —
        // the next packet must still match the pre-duplicate expectation.
        // We DO update `state.last_wire_cc` so that a third identical repeat
        // is also preserved.
        if current_cc == state.last_wire_cc && payload_hash == state.last_payload_hash {
            // Legal duplicate repeat — pass through unchanged.
            // Don't advance expected, but update last_wire_cc for next comparison.
            state.last_wire_cc = current_cc;
            state.last_payload_hash = payload_hash;
            out(packet);
            return;
        }

        // Normal case: check CC against expected value.
        if current_cc == state.expected {
            // Correct CC.  Advance expected for next packet.
            let next = (current_cc + 1) & 0x0F;
            state.expected = next;
            state.last_wire_cc = current_cc;
            state.last_payload_hash = payload_hash;
            out(packet);
            return;
        }

        // Genuine CC error: unsignalled, non-duplicate gap.
        // Rewrite the CC to the expected value.
        let correct_cc = state.expected;
        let mut buf = [0u8; TS_PACKET_SIZE];
        buf.copy_from_slice(packet);
        OwnedTsPacket::set_continuity_counter(&mut buf, correct_cc);
        out(&buf[..]);

        // Advance state using the **corrected** value.  Set
        // `state.last_wire_cc` to the OUTPUT CC (`correct_cc`) so that
        // subsequent duplicate detection sees the repaired value, not the
        // original error.
        state.expected = (correct_cc + 1) & 0x0F;
        state.last_wire_cc = correct_cc;
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Nothing buffered.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpeg_ts::ts::TS_SYNC_BYTE;

    /// Build a TS packet with the given header fields and payload.
    /// `b3_template` provides the base "byte 3" value (AF bits, CC masked out).
    fn make_payload_packet(pid: u16, cc: u8, has_adaptation: bool, payload: &[u8]) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        let mut b3 = cc & 0x0F;
        b3 |= 0x10; // payload flag
        if has_adaptation {
            b3 |= 0x20; // adaptation flag
        }
        pkt[3] = b3;

        let mut cursor = 4usize;
        if has_adaptation {
            // Minimal adaptation field: length=1, flags=0, no PCR
            pkt[cursor] = 1; // af_length
            pkt[cursor + 1] = 0; // flags (no discontinuity, no PCR)
            cursor += 2;
        }
        let payload_len = payload.len().min(188 - cursor);
        pkt[cursor..cursor + payload_len].copy_from_slice(&payload[..payload_len]);
        pkt
    }

    /// Build a packet with discontinuity_indicator set.
    fn make_discontinuity_packet(pid: u16, cc: u8, payload: &[u8]) -> [u8; 188] {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = cc & 0x0F;
        pkt[3] |= 0x10 | 0x20; // payload + adaptation flags

        // adaptation field: length=1, flags=0x80 (discontinuity)
        pkt[4] = 1; // af_length
        pkt[5] = 0x80; // discontinuity_indicator = 1
        let cursor = 6usize;
        let payload_len = payload.len().min(188 - cursor);
        pkt[cursor..cursor + payload_len].copy_from_slice(&payload[..payload_len]);
        pkt
    }

    fn run_op(packets: &[[u8; 188]]) -> Vec<[u8; 188]> {
        let mut op = ContinuityOp::new();
        let mut model = StreamModel::default();
        let mut output = Vec::new();
        for pkt in packets {
            op.process(pkt, &mut model, &mut |out| {
                let mut buf = [0u8; 188];
                buf.copy_from_slice(out);
                output.push(buf);
            });
        }
        op.flush(&mut model, &mut |_| {});
        output
    }

    #[test]
    fn non_payload_does_not_advance_cc() {
        // §2.4.3.3: CC only increments on payload-bearing packets.
        let pkt1 = make_payload_packet(0x0100, 0, false, &[0xAA]);
        let mut pkt2 = make_payload_packet(0x0100, 0, false, &[0xBB]);
        pkt2[3] &= 0xCF; // clear AFC bits
        pkt2[3] |= 0x20; // afc = 10 (adaptation-only)

        let mut pkt3 = make_payload_packet(0x0100, 0, false, &[0xCC]);
        pkt3[3] &= 0xCF;
        pkt3[3] |= 0x00; // afc = 00 (reserved, no payload)

        let output = run_op(&[pkt1, pkt2, pkt3]);
        assert_eq!(output.len(), 3);
        // pkt1: payload, CC=0, first packet → expected.next=1, pass through
        assert_eq!(output[0][3] & 0x0F, 0);
        // pkt2: adaptation-only, CC=0, pass through unchanged
        assert_eq!(output[1][3] & 0x0F, 0);
        // pkt3: reserved/00, CC=0, pass through unchanged
        assert_eq!(output[2][3] & 0x0F, 0);
    }

    #[test]
    fn duplicate_packet_is_preserved() {
        // §2.4.3.3 L1772: same PID, same CC, same payload bytes.
        let payload = &[0xAB, 0xCD, 0xEF];
        let pkt1 = make_payload_packet(0x0200, 0, false, payload);
        let pkt2 = make_payload_packet(0x0200, 0, false, payload); // duplicate

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        // Both must have CC=0 (duplicate preserved).
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 0);
    }

    #[test]
    fn same_cc_with_different_payload_is_repaired() {
        // Same CC but different payload — per §2.4.3.3 L1772 this is NOT a
        // legal duplicate (payload must be identical).  The repair renumbers
        // it to the expected CC.
        let pkt1 = make_payload_packet(0x0200, 0, false, &[0xAB]);
        let pkt2 = make_payload_packet(0x0200, 0, false, &[0xCD]); // same CC, different payload

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][3] & 0x0F, 0);
        // pkt2: not a duplicate (different payload), should be repaired to CC=1
        assert_eq!(output[1][3] & 0x0F, 1);
    }

    #[test]
    fn discontinuity_packet_is_preserved() {
        // §2.4.3.5: discontinuity_indicator = 1 → any CC is valid.
        let pkt1 = make_payload_packet(0x0300, 0, false, &[0xAA]);
        let pkt2 = make_discontinuity_packet(0x0300, 0x0F, &[0xBB]);

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][3] & 0x0F, 0);
        // Discontinuity packet keeps its CC=15
        assert_eq!(output[1][3] & 0x0F, 0x0F);
    }

    #[test]
    fn cc_after_discontinuity_resets() {
        // After a discontinuity, the next packet's CC should not be forced
        // to match the sequence from before the discontinuity.
        let pkt1 = make_payload_packet(0x0300, 0, false, &[0xAA]);
        let pkt2 = make_discontinuity_packet(0x0300, 0x0F, &[0xBB]);
        let pkt3 = make_payload_packet(0x0300, 3, false, &[0xCC]); // after discontinuity

        let output = run_op(&[pkt1, pkt2, pkt3]);
        assert_eq!(output.len(), 3);
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 0x0F);
        // pkt3 (CC=3) is the *first* post-discontinuity packet — it initialises
        // fresh state and passes through unchanged.
        assert_eq!(output[2][3] & 0x0F, 3);
    }

    #[test]
    fn genuine_cc_gap_is_repaired() {
        // Uns signalled, non-duplicate gap: 0 → should be 1.
        let pkt1 = make_payload_packet(0x0400, 0, false, &[0xAA]);
        let pkt2 = make_payload_packet(0x0400, 3, false, &[0xBB]); // gap: expected 1, got 3

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 1); // repaired
    }

    #[test]
    fn normal_cc_sequence_passes_unmodified() {
        let pkt1 = make_payload_packet(0x0500, 0, false, &[0xAA]);
        let pkt2 = make_payload_packet(0x0500, 1, false, &[0xBB]);
        let pkt3 = make_payload_packet(0x0500, 2, false, &[0xCC]);

        let output = run_op(&[pkt1, pkt2, pkt3]);
        assert_eq!(output.len(), 3);
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 1);
        assert_eq!(output[2][3] & 0x0F, 2);
    }

    #[test]
    fn cc_wrap_around() {
        let pkt1 = make_payload_packet(0x0600, 0x0F, false, &[0xAA]);
        let pkt2 = make_payload_packet(0x0600, 0x00, false, &[0xBB]); // 15→0, correct wrap

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][3] & 0x0F, 0x0F);
        assert_eq!(output[1][3] & 0x0F, 0x00); // 0 expected after 15
    }

    #[test]
    fn multiple_pids_independent() {
        let pkt1 = make_payload_packet(0x0100, 0, false, &[0xAA]);
        let pkt2 = make_payload_packet(0x0200, 0, false, &[0xBB]);
        let pkt3 = make_payload_packet(0x0100, 1, false, &[0xCC]); // correct next CC for PID 0x0100
        let pkt4 = make_payload_packet(0x0200, 1, false, &[0xDD]); // correct next CC for PID 0x0200
        let pkt5 = make_payload_packet(0x0100, 2, false, &[0xEE]); // correct

        let output = run_op(&[pkt1, pkt2, pkt3, pkt4, pkt5]);
        assert_eq!(output.len(), 5);
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 0);
        assert_eq!(output[2][3] & 0x0F, 1);
        assert_eq!(output[3][3] & 0x0F, 1);
        assert_eq!(output[4][3] & 0x0F, 2);
    }

    #[test]
    fn afc_11_packet_with_adaptation_advances_cc() {
        // afc = 11 (both adaptation and payload) — CC SHOULD increment.
        let pkt1 = make_payload_packet(0x0700, 0, true, &[0xAA]);
        let pkt2 = make_payload_packet(0x0700, 1, true, &[0xBB]);

        let output = run_op(&[pkt1, pkt2]);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][3] & 0x0F, 0);
        assert_eq!(output[1][3] & 0x0F, 1);
    }

    #[test]
    fn duplicate_with_pcr_difference_is_preserved() {
        // Two packets with same PID, same CC, same payload, but different PCR.
        // The repair should preserve the duplicate (payload hash skips PCR bytes).
        let mut pkt1 = [0xFFu8; 188];
        pkt1[0] = TS_SYNC_BYTE;
        pkt1[1] = 0x00;
        pkt1[2] = 0x50; // PID = 0x0050
        pkt1[3] = 0x10; // afc=01 payload-only, CC=0
        pkt1[4..10].copy_from_slice(b"PAYLOA");
        // First packet with PCR adaptation: afc=11, CC=1
        let mut pkt1b = [0xFFu8; 188];
        pkt1b[0] = TS_SYNC_BYTE;
        pkt1b[1] = 0x00;
        pkt1b[2] = 0x50;
        pkt1b[3] = 0x30 | 0x01; // afc=11, CC=1
        pkt1b[4] = 7; // af_length
        pkt1b[5] = 0x10; // PCR flag set
        // PCR bytes (6)
        pkt1b[6] = 0x00;
        pkt1b[7] = 0x00;
        pkt1b[8] = 0x00;
        pkt1b[9] = 0x00;
        pkt1b[10] = 0x00;
        pkt1b[11] = 0x00;
        // Payload
        pkt1b[12..18].copy_from_slice(b"PAYLOA");

        // Duplicate with different PCR
        let mut pkt2 = [0xFFu8; 188];
        pkt2[0] = TS_SYNC_BYTE;
        pkt2[1] = 0x00;
        pkt2[2] = 0x50;
        pkt2[3] = 0x30 | 0x01; // afc=11, CC=1 (same CC!)
        pkt2[4] = 7; // af_length
        pkt2[5] = 0x10; // PCR flag set
        // Different PCR
        pkt2[6] = 0x12;
        pkt2[7] = 0x34;
        pkt2[8] = 0x56;
        pkt2[9] = 0x78;
        pkt2[10] = 0x9A;
        pkt2[11] = 0xBC;
        // Same payload
        pkt2[12..18].copy_from_slice(b"PAYLOA");

        let output = run_op(&[pkt1, pkt1b, pkt2]);
        assert_eq!(output.len(), 3);
        // pkt1 (afc=01, CC=0) preserved
        assert_eq!(output[0][3] & 0x0F, 0);
        // pkt1b (afc=11, CC=1) preserved — correct next in sequence
        assert_eq!(output[1][3] & 0x0F, 1);
        // pkt2 (CC=1, duplicate with different PCR) — should be preserved
        assert_eq!(output[2][3] & 0x0F, 1);
    }
}
