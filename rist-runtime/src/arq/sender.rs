//! Sender-side NACK response — VSF TR-06-1:2020 §5.3.3 (Retransmitted
//! Packets) + §5.3.4 (Burst Control, informative).
//!
//! §5.3.3 spells out the sender's responsibility on receiving a NACK:
//! identify the flow via the SSRC field, locate the originally-sent packet,
//! and resend an exact copy (same sequence number and timestamp) with the
//! SSRC LSB flipped to 1. It explicitly does *not* prescribe how the sender
//! looks the packet up — "that storage/lookup mechanism is left to the
//! implementation" — so the bounded ring buffer here is **implementation
//! policy**, not a transcription. This module is deliberately RTP-framing
//! agnostic (`rist-runtime` has no dependency on an RTP codec crate): it
//! stores whatever opaque payload bytes + timestamp the caller handed it at
//! send time, and hands back exactly that — flipping the SSRC LSB and
//! rebuilding the actual RTP packet is the caller's job.
//!
//! §5.3.4 flags that a single Range-Based request field can nominally
//! demand up to 65536 retransmissions (`Additional = 0xFFFF`) and states an
//! implementation "must be prepared to throttle/reject this rather than
//! attempt it literally" — [`super::MAX_RANGE_EXPANSION`] is this engine's
//! throttle: any range beyond that many entries is truncated rather than
//! expanded literally.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::nack::BLP_BIT_WIDTH;
use crate::{GenericNack, NackFci, PacketRange, RangeNack};

use super::MAX_RANGE_EXPANSION;
use super::seq;

/// One packet retransmitted in response to a NACK: the original sequence
/// number and timestamp, and the original payload bytes. The caller
/// rebuilds the actual RTP packet (with the SSRC LSB flipped to 1 per
/// §5.3.3) using whatever RTP codec it already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retransmission<'a> {
    /// The original RTP sequence number (unchanged on retransmission, §5.3.3).
    pub seq: u16,
    /// The original RTP timestamp (unchanged on retransmission, §5.3.3).
    pub timestamp: u32,
    /// The original payload bytes.
    pub payload: &'a [u8],
}

#[derive(Debug, Clone)]
struct SentPacket {
    seq: u16,
    timestamp: u32,
    payload: Vec<u8>,
}

/// Sender-side lookup buffer for NACK-triggered retransmission (§5.3.3).
#[derive(Debug)]
pub struct Sender {
    buffer: VecDeque<SentPacket>,
    max_buffered: usize,
}

impl Sender {
    /// A fresh sender retaining at most `max_buffered` sent packets for
    /// retransmission lookup. TR-06-1's only stated sender-buffer
    /// constraint is the qualitative "Sender Buffer >= Receiver Buffer"
    /// (Appendix B) — a *time*-based relationship this crate does not
    /// itself measure a sending rate to enforce, so this packet-count cap
    /// is **implementation policy**, not a transcription of that relation.
    pub fn new(max_buffered: usize) -> Self {
        Sender {
            buffer: VecDeque::new(),
            max_buffered: max_buffered.max(1),
        }
    }

    /// Number of packets currently retained for lookup.
    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }

    /// Record a freshly-sent packet so it can later be located and
    /// retransmitted (§5.3.3). Evicts the oldest buffered packet once
    /// `max_buffered` is exceeded.
    pub fn on_sent(&mut self, seq: u16, timestamp: u32, payload: &[u8]) {
        self.buffer.push_back(SentPacket {
            seq,
            timestamp,
            payload: payload.to_vec(),
        });
        while self.buffer.len() > self.max_buffered {
            self.buffer.pop_front();
        }
    }

    /// Look up every sequence number named by a [`RangeNack`] that is still
    /// in the lookup buffer; sequence numbers already evicted (too old, or
    /// never sent) are silently skipped — §5.3.3 does not define behaviour
    /// for a request naming a packet the sender no longer has.
    pub fn on_range_nack(&self, nack: &RangeNack) -> Vec<Retransmission<'_>> {
        let mut out = Vec::new();
        for range in &nack.ranges {
            for s in expand_range(*range) {
                if let Some(sent) = self.buffer.iter().find(|p| p.seq == s) {
                    out.push(Retransmission {
                        seq: sent.seq,
                        timestamp: sent.timestamp,
                        payload: &sent.payload,
                    });
                }
            }
        }
        out
    }

    /// Look up every sequence number named by a [`GenericNack`] (bitmask
    /// format), same lookup semantics as [`Self::on_range_nack`].
    pub fn on_generic_nack(&self, nack: &GenericNack) -> Vec<Retransmission<'_>> {
        let mut out = Vec::new();
        for fci in &nack.nacks {
            for s in expand_fci(*fci) {
                if let Some(sent) = self.buffer.iter().find(|p| p.seq == s) {
                    out.push(Retransmission {
                        seq: sent.seq,
                        timestamp: sent.timestamp,
                        payload: &sent.payload,
                    });
                }
            }
        }
        out
    }
}

fn expand_range(range: PacketRange) -> Vec<u16> {
    let count = (u32::from(range.additional) + 1).min(MAX_RANGE_EXPANSION as u32);
    let mut out = Vec::with_capacity(count as usize);
    let mut s = range.start;
    for _ in 0..count {
        out.push(s);
        s = seq::seq_next(s);
    }
    out
}

fn expand_fci(fci: NackFci) -> Vec<u16> {
    let mut out = alloc::vec![fci.pid];
    for bit in 0..BLP_BIT_WIDTH {
        if fci.blp & (1 << bit) != 0 {
            out.push(seq::seq_add(fci.pid, (bit + 1) as u16));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NackFci;

    #[test]
    fn on_sent_then_range_nack_locates_the_exact_payload() {
        let mut s = Sender::new(16);
        s.on_sent(100, 900_000, b"packet-100");
        s.on_sent(101, 900_090, b"packet-101");

        let nack = RangeNack {
            ssrc_media: 0x1234,
            ranges: alloc::vec![PacketRange {
                start: 100,
                additional: 0
            }],
        };
        let out = s.on_range_nack(&nack);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, 100);
        assert_eq!(out[0].timestamp, 900_000);
        assert_eq!(out[0].payload, b"packet-100");
    }

    #[test]
    fn range_nack_expands_a_contiguous_run() {
        let mut s = Sender::new(16);
        for seq in 10..15u16 {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 10,
                additional: 4
            }],
        };
        let out = s.on_range_nack(&nack);
        let seqs: Vec<u16> = out.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, alloc::vec![10, 11, 12, 13, 14]);
    }

    #[test]
    fn evicted_packets_are_silently_skipped() {
        let mut s = Sender::new(1);
        s.on_sent(1, 0, b"a");
        s.on_sent(2, 0, b"b"); // evicts seq 1
        assert_eq!(s.buffered_count(), 1);

        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }],
        };
        assert!(s.on_range_nack(&nack).is_empty());
    }

    #[test]
    fn generic_nack_bitmask_expands_pid_and_blp() {
        let mut s = Sender::new(32);
        for seq in [100u16, 103, 117] {
            s.on_sent(seq, 0, b"x");
        }
        // PID=100 signals 100 lost, BLP bit3 (=103) also lost.
        let nack = GenericNack {
            ssrc_sender: 0,
            ssrc_media: 1,
            nacks: alloc::vec![
                NackFci {
                    pid: 100,
                    blp: 0b0000_0000_0000_0100
                },
                NackFci { pid: 117, blp: 0 },
            ],
        };
        let out = s.on_generic_nack(&nack);
        let seqs: Vec<u16> = out.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, alloc::vec![100, 103, 117]);
    }

    #[test]
    fn range_expansion_is_throttled_against_an_adversarial_additional_count() {
        let s = Sender::new(1);
        let nack = RangeNack {
            ssrc_media: 1,
            // TR-06-1 §5.3.4's own called-out worst case.
            ranges: alloc::vec![PacketRange {
                start: 0,
                additional: 0xFFFF
            }],
        };
        // Must not attempt to allocate/iterate 65536 entries unbounded in a
        // way that panics or hangs; with nothing in the lookup buffer the
        // result is simply empty, but the point is this returns promptly.
        assert!(s.on_range_nack(&nack).is_empty());
    }
}
