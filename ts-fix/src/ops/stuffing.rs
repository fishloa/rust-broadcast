//! Null packet stuffing / drop operation.
//!
//! Inserts null (PID 0x1FFF) packets to reach a target packet rate, or drops
//! existing null packets.
//!
//! # Null packet format
//!
//! A null TS packet is a standard 188-byte TS packet with:
//! - PID = 0x1FFF (ISO/IEC 13818-1 §2.4.1)
//! - Adaptation field control = 01 (payload only, no adaptation field)
//! - No payload; rest is 0xFF padding
//!
//! Null packets are built via [`mpeg_ts::OwnedTsPacket::null_packet`]; this
//! module contains no raw wire-byte construction.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.1 (null packets).

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::TS_PACKET_SIZE;

use crate::ops::{Op, StreamModel};

/// Null packet PID (ISO/IEC 13818-1 §2.4.1).
const NULL_PID: u16 = 0x1FFF;

/// Configuration for [`TsFixBuilder::stuffing`](crate::TsFixBuilder::stuffing).
///
/// `#[non_exhaustive]` — future modes may be added without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Stuffing {
    /// Drop all null packets (PID 0x1FFF) from the output.
    ///
    /// Constructed via [`Stuffing::drop_nulls`].
    DropNulls,

    /// Insert null packets to reach a target packet rate.
    ///
    /// Constructed via [`Stuffing::pad_to`].
    PadTo {
        /// Target number of packets per input packet.
        ///
        /// If the input has 100 packets and `packets_per_input` is 2,
        /// the output will have approximately 200 packets (100 input × 2).
        /// Fractional scaling is achieved by accumulating the rate across
        /// the stream.
        packets_per_input: f64,
    },
}

impl Stuffing {
    /// Drop all null packets from the output.
    ///
    /// # Example
    /// ```
    /// use ts_fix::Stuffing;
    /// let cfg = Stuffing::drop_nulls();
    /// ```
    pub fn drop_nulls() -> Self {
        Self::DropNulls
    }

    /// Pad the stream to a target packet rate.
    ///
    /// Inserts null packets (PID 0x1FFF) after each input packet such that the
    /// total output is approximately `packets_per_input × number_of_input_packets`.
    ///
    /// The rate is accumulated across the stream; fractional rates are smoothed.
    ///
    /// # Example — double the packet count
    /// ```
    /// use ts_fix::Stuffing;
    /// let cfg = Stuffing::pad_to(2.0);
    /// ```
    pub fn pad_to(packets_per_input: f64) -> Self {
        Self::PadTo { packets_per_input }
    }
}

// ── Null packet construction ──────────────────────────────────────────────

/// Construct a standard null TS packet via the mpeg-ts writer.
fn make_null_packet(continuity_counter: u8) -> [u8; TS_PACKET_SIZE] {
    OwnedTsPacket::null_packet(continuity_counter)
}

// ── The operation ────────────────────────────────────────────────────────────

/// Null packet stuffing / drop operation.
pub(crate) struct StuffingOp {
    mode: StuffingMode,
}

enum StuffingMode {
    /// Drop all null packets.
    Drop,
    /// Pad to a target rate.
    Pad {
        /// How many null packets to insert per real packet (derived from rate).
        nulls_per_real: f64,
        /// Accumulated fractional packets; when ≥ 1.0, emit a null packet.
        accumulated: f64,
        /// Null packet continuity counter (per ISO 13818-1, CCs wrap 0-15).
        null_cc: u8,
    },
}

impl StuffingOp {
    pub(crate) fn new(cfg: Stuffing) -> Self {
        let mode = match cfg {
            Stuffing::DropNulls => StuffingMode::Drop,
            Stuffing::PadTo { packets_per_input } => {
                // If the target is 2.0, we want 2 total packets per input packet.
                // That means 1 null packet per input packet.
                let nulls_per_real = packets_per_input - 1.0;
                StuffingMode::Pad {
                    nulls_per_real,
                    accumulated: 0.0,
                    null_cc: 0,
                }
            }
        };
        Self { mode }
    }

    /// Check if a packet is a null packet (PID 0x1FFF).
    fn is_null_packet(packet: &[u8]) -> bool {
        if packet.len() < 3 {
            return false;
        }
        // Extract PID from bytes 1-2.
        let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
        pid == NULL_PID
    }
}

impl Op for StuffingOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        match &mut self.mode {
            StuffingMode::Drop => {
                // Drop null packets; pass everything else through.
                if !Self::is_null_packet(packet) {
                    out(packet);
                }
            }
            StuffingMode::Pad {
                nulls_per_real,
                accumulated,
                null_cc,
                ..
            } => {
                // Always emit the input packet (even null packets).
                out(packet);

                // Accumulate the number of null packets to insert.
                *accumulated += *nulls_per_real;

                // Emit null packets for every 1.0 units of accumulated nulls.
                while *accumulated >= 1.0 {
                    let null = make_null_packet(*null_cc);
                    out(&null);
                    *accumulated -= 1.0;
                    *null_cc = (*null_cc + 1) & 0x0F;
                }
            }
        }
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Stuffing has no buffered state to flush at end-of-stream.
        // In pad mode, the accumulated fractional packet is discarded
        // (typical for bitrate padding — the last partial packet is normal).
    }
}
