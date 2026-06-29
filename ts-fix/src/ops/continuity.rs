//! Continuity counter repair operation.
//!
//! Renumbers the 4-bit `continuity_counter` per PID to a correct monotonic
//! sequence (mod 16), respecting the "no increment on adaptation-only / no-payload"
//! rules per ISO/IEC 13818-1 §2.4.3.3.

use alloc::collections::BTreeMap;

use mpeg_ts::ts::{TsHeader, CC_MASK, TS_PACKET_SIZE};

use crate::ops::{Op, StreamModel};

/// Continuity counter repair operation.
///
/// Per ISO/IEC 13818-1 §2.4.3.3, the 4-bit `continuity_counter` in the TS
/// header (byte 3, bits [3:0]) increments (mod 16) **only** on packets that
/// carry a payload (when `adaptation_field_control` is 01 or 11).  On
/// adaptation-only packets (10) or no-payload packets, the counter does not
/// increment — the next payload-bearing packet repeats the expected value or
/// starts the sequence anew.
///
/// This op maintains per-PID expected-counter state and rewrites the CC nibble
/// to the correct value on every packet.
pub(crate) struct ContinuityOp {
    /// Per-PID expected continuity counter (mod 16).
    ///
    /// On the first payload-bearing packet from a PID, we initialize the
    /// counter to what we see and expect (mod 16).  On each subsequent
    /// payload-bearing packet from that PID, we increment and check.
    per_pid: BTreeMap<u16, u8>,
}

impl ContinuityOp {
    /// Create a new continuity counter repair operation.
    pub(crate) fn new() -> Self {
        Self {
            per_pid: BTreeMap::new(),
        }
    }
}

impl Op for ContinuityOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        // Parse the TS header to extract PID and adaptation field info.
        // We already validated sync byte + 188 bytes in engine::push.
        if packet.len() != TS_PACKET_SIZE {
            // Should not happen (engine validated), but be safe.
            out(packet);
            return;
        }

        let header = match TsHeader::parse(&packet[..4]) {
            Ok(h) => h,
            Err(_) => {
                // Malformed header — pass through unchanged.
                out(packet);
                return;
            }
        };

        let pid = header.pid;
        let has_payload = header.has_payload;
        let current_cc = header.continuity_counter;

        // Only repair payload-bearing packets; adaptation-only packets are fine
        // because the spec allows undefined CC on them.
        if !has_payload {
            // For adaptation-only: repair if we have per-PID state and the CC
            // doesn't match the last payload CC. But the spec says this is optional.
            // For safety and to minimize modifications, leave adaptation-only packets alone.
            out(packet);
            return;
        }

        // Payload-bearing packet: check if it matches expected CC.
        let expected = self.per_pid.entry(pid).or_insert_with(|| current_cc);

        let is_correct = *expected == current_cc;

        if is_correct {
            // Already correct, pass through unchanged.
            out(packet);
        } else {
            // Repair: clone and rewrite the CC.
            let mut buf = [0u8; TS_PACKET_SIZE];
            buf.copy_from_slice(packet);
            buf[3] = (buf[3] & !CC_MASK) | (*expected & CC_MASK);
            out(&buf[..]);
        }

        // Advance to next expected CC for this PID.
        *expected = (*expected + 1) & 0x0F;
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Nothing buffered.
    }
}
