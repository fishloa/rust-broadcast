//! ARQ sender-side reliability state — `draft-sharabayko-srt-01` §4.8
//! (Acknowledgement and Lost Packet Handling), §4.8.1 (ACKs/ACKACKs), §4.8.2
//! (NAKs), §4.10 (RTT). Curated rules: `specs/rules/srt-arq.md`.
//!
//! Sans-IO: [`Sender`] never reads a wall clock. [`Sender::on_data`] buffers
//! a freshly-submitted data packet (rule 1) and returns the wire bytes to
//! send; [`Sender::on_nak`] records the reported loss list for prioritized
//! retransmission (rules 5, 15, 16, 18); [`Sender::tick`] drains the pending
//! retransmit queue; [`Sender::on_ack`] frees acknowledged packets (rules 7,
//! 8, 16, 17) and, for a Full ACK, updates RTT/RTTVar (rule 33) and returns
//! the ACKACK reply (rules 3, 9).
//!
//! # Priority note (rules 5, 15, 16)
//! This sans-IO engine has no internal scheduler: [`Sender::on_data`] sends
//! its packet immediately rather than queuing it behind pending
//! retransmissions. A caller reproduces the spec's "loss list before first
//! transmission" priority by calling [`Sender::tick`] (which drains only the
//! retransmit queue) before submitting new application data each round.
//!
//! # Non-goals
//! Send-queue overflow / unsent-packet drop (rules 19-20) and RTO-based
//! periodic retransmission without a NAK (§5, FileCC) are out of scope — see
//! the `arq` module doc.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::time::Duration;

use crate::packet::{
    AckAckPacket, AckCif, AckPacket, ControlPacket, DataPacket, EncryptionKeyField, LossListEntry,
    NakPacket, PacketPosition,
};

use super::rtt::RttEstimator;
use super::{duration_to_wire_us, seq};

/// A NAK loss-list range is a compact wire encoding (Appendix A), but
/// nothing stops a malformed/adversarial range from declaring billions of
/// entries. This is not a `specs/rules/srt-arq.md` rule — a safety cap to
/// keep [`Sender::on_nak`] from doing unbounded work per range entry.
const MAX_RANGE_EXPANSION: u32 = 1 << 16;

/// One buffered, sent-but-not-yet-acknowledged data packet (rules 1, 16,
/// 18).
#[derive(Debug, Clone)]
struct SentPacket {
    seq: u32,
    message_number: u32,
    payload: Vec<u8>,
    /// Resend counter (rule 18): incremented on every retransmission.
    resend_count: u32,
}

/// ARQ sender-side state (`draft-sharabayko-srt-01` §4.8). See the module
/// doc for the sans-IO contract.
#[derive(Debug)]
pub struct Sender {
    dest_socket_id: u32,
    /// Send buffer of unacknowledged packets, oldest first (rule 1).
    buffer: VecDeque<SentPacket>,
    /// Sequence numbers the receiver has reported lost (via NAK), pending
    /// retransmission (rules 16, 18).
    pending_retransmit: BTreeSet<u32>,
    rtt: RttEstimator,
}

impl Sender {
    /// A fresh sender addressing `dest_socket_id` (the peer's SRT Socket ID,
    /// carried in every packet header, §3).
    pub fn new(dest_socket_id: u32) -> Self {
        Sender {
            dest_socket_id,
            buffer: VecDeque::new(),
            pending_retransmit: BTreeSet::new(),
            rtt: RttEstimator::new(),
        }
    }

    /// The current RTT estimate (rule 33 — updated from each Full ACK's
    /// carried value).
    pub fn rtt(&self) -> Duration {
        self.rtt.rtt()
    }

    /// The current RTTVar estimate.
    pub fn rtt_var(&self) -> Duration {
        self.rtt.rtt_var()
    }

    /// Number of packets still buffered, unacknowledged.
    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }

    /// Number of sequence numbers currently pending retransmission.
    pub fn pending_retransmit_count(&self) -> usize {
        self.pending_retransmit.len()
    }

    /// Submit a new data packet for first transmission (rule 1: the sender
    /// buffers every sent packet to enable retransmission). Returns the
    /// wire bytes to send now — see the module doc's priority note.
    pub fn on_data(
        &mut self,
        seq: u32,
        message_number: u32,
        payload: &[u8],
        now: Duration,
    ) -> Vec<u8> {
        self.buffer.push_back(SentPacket {
            seq,
            message_number,
            payload: payload.to_vec(),
            resend_count: 0,
        });
        let pkt = DataPacket {
            seq_number: seq,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: EncryptionKeyField::NotEncrypted,
            retransmitted: false,
            message_number,
            timestamp: duration_to_wire_us(now),
            dest_socket_id: self.dest_socket_id,
            data: payload,
        };
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)
            .expect("buffer sized from serialized_len");
        buf
    }

    /// Record a NAK's loss-list entries for prioritized retransmission
    /// (`specs/rules/srt-arq.md` rules 5, 15, 16, 18). Entries no longer in
    /// the send buffer (already freed by a since-received ACK) are silently
    /// ignored (rule 17).
    pub fn on_nak(&mut self, nak: &NakPacket<'_>) {
        for entry in nak.entries() {
            let Ok(entry) = entry else { continue };
            for seq in expand_loss_entry(entry) {
                if self.buffer.iter().any(|p| p.seq == seq) {
                    self.pending_retransmit.insert(seq);
                }
            }
        }
    }

    /// Drain the pending retransmit queue, returning the wire bytes of each
    /// retransmission (rules 16, 18 — the `R` flag is set, the resend
    /// counter incremented). A queued sequence number no longer in the
    /// buffer (freed by a since-received ACK) is dropped without emitting
    /// anything (rule 17).
    pub fn tick(&mut self, now: Duration) -> Vec<Vec<u8>> {
        let seqs: Vec<u32> = core::mem::take(&mut self.pending_retransmit)
            .into_iter()
            .collect();
        let mut out = Vec::with_capacity(seqs.len());
        for seq in seqs {
            let Some(sent) = self.buffer.iter_mut().find(|p| p.seq == seq) else {
                continue; // rule 17: already dropped from the buffer.
            };
            sent.resend_count += 1;
            let pkt = DataPacket {
                seq_number: sent.seq,
                position: PacketPosition::Solo,
                in_order: true,
                key_flag: EncryptionKeyField::NotEncrypted,
                retransmitted: true,
                message_number: sent.message_number,
                timestamp: duration_to_wire_us(now),
                dest_socket_id: self.dest_socket_id,
                data: &sent.payload,
            };
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf)
                .expect("buffer sized from serialized_len");
            out.push(buf);
        }
        out
    }

    /// Process an incoming ACK: free every acknowledged packet (rules 7, 8,
    /// 16, 17), and — for a Full ACK only — update RTT/RTTVar (rule 33) and
    /// return the ACKACK reply (rules 3, 9).
    pub fn on_ack(&mut self, ack: &AckPacket, now: Duration) -> Option<Vec<u8>> {
        let last_ack_seq = match ack.cif {
            AckCif::Full { last_ack_seq, .. }
            | AckCif::Small { last_ack_seq, .. }
            | AckCif::Light { last_ack_seq } => last_ack_seq,
        };
        // rule 8: every seq strictly before `last_ack_seq` is acknowledged.
        while let Some(front) = self.buffer.front() {
            if seq::seq_lt(front.seq, last_ack_seq) {
                let freed = self.buffer.pop_front().expect("front just matched");
                self.pending_retransmit.remove(&freed.seq); // rule 17
            } else {
                break;
            }
        }
        // rule 17: drop any now-stale entries dragged along by the loop
        // above (defensive; the loop already removes the freed seq, but a
        // NAK could have named a seq that a later ACK skipped over).
        let buffered: BTreeSet<u32> = self.buffer.iter().map(|p| p.seq).collect();
        self.pending_retransmit.retain(|s| buffered.contains(s));

        if let AckCif::Full { rtt_us, .. } = ack.cif {
            // rule 33: same EWMA as rules 29-30, `rtt` = the ACK's carried
            // value.
            self.rtt.update(Duration::from_micros(u64::from(rtt_us)));

            let pkt = ControlPacket::AckAck(AckAckPacket {
                ack_number: ack.ack_number,
                timestamp: duration_to_wire_us(now),
                dest_socket_id: self.dest_socket_id,
            });
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf)
                .expect("buffer sized from serialized_len");
            Some(buf)
        } else {
            // rule 12: a Light ACK does not trigger an ACKACK. A Small
            // ACK's ack_number is likewise "should be set to 0" (§3.2.4)
            // and is not part of the numbered ACK/ACKACK exchange (rule
            // 24) — srt-arq.md does not state this explicitly for Small
            // ACK, resolved the same way as Light for consistency with
            // that wire convention.
            None
        }
    }
}

fn expand_loss_entry(entry: LossListEntry) -> Vec<u32> {
    match entry {
        LossListEntry::Single(s) => alloc::vec![s],
        LossListEntry::Range(first, last) => {
            let mut out = Vec::new();
            let mut s = first;
            let mut n = 0u32;
            loop {
                out.push(s);
                if s == last || n >= MAX_RANGE_EXPANSION {
                    break;
                }
                s = seq::seq_add(s, 1);
                n += 1;
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::nak::build_loss_list;

    const PEER: u32 = 0xAAAA;

    fn nak_bytes(entries: &[LossListEntry]) -> Vec<u8> {
        let raw = build_loss_list(entries).unwrap();
        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: 0,
            dest_socket_id: PEER,
            raw_loss_list: &raw,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn on_data_buffers_and_returns_wire_bytes() {
        let mut s = Sender::new(PEER);
        let bytes = s.on_data(5, 5, b"hello", Duration::from_millis(1));
        assert_eq!(s.buffered_count(), 1);
        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, 5);
        assert!(!dp.retransmitted);
        assert_eq!(dp.data, b"hello");
    }

    #[test]
    fn nak_then_tick_retransmits_with_r_flag_set() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Duration::ZERO);
        s.on_data(1, 1, b"b", Duration::ZERO);

        let raw = nak_bytes(&[LossListEntry::Single(1)]);
        let ControlPacket::Nak(nak) = ControlPacket::parse(&raw).unwrap() else {
            panic!("expected NAK");
        };
        s.on_nak(&nak);
        assert_eq!(s.pending_retransmit_count(), 1);

        let out = s.tick(Duration::from_millis(5));
        assert_eq!(out.len(), 1);
        let dp = DataPacket::parse(&out[0]).unwrap();
        assert_eq!(dp.seq_number, 1);
        assert!(dp.retransmitted);
        assert_eq!(s.pending_retransmit_count(), 0);
    }

    #[test]
    fn nak_for_unbuffered_seq_is_ignored() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Duration::ZERO);
        let raw = nak_bytes(&[LossListEntry::Single(99)]);
        let ControlPacket::Nak(nak) = ControlPacket::parse(&raw).unwrap() else {
            panic!("expected NAK");
        };
        s.on_nak(&nak);
        assert_eq!(s.pending_retransmit_count(), 0);
        assert!(s.tick(Duration::ZERO).is_empty());
    }

    #[test]
    fn full_ack_frees_buffer_and_updates_rtt_and_replies_ackack() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Duration::ZERO);
        s.on_data(1, 1, b"b", Duration::ZERO);
        s.on_data(2, 2, b"c", Duration::ZERO);

        let ack = AckPacket {
            ack_number: 1,
            timestamp: 0,
            dest_socket_id: PEER,
            cif: AckCif::Full {
                last_ack_seq: 2,
                rtt_us: 20_000,
                rtt_var_us: 5_000,
                avail_buf_size: 0,
                pkt_recv_rate: 0,
                est_link_capacity: 0,
                recv_rate_bps: 0,
            },
        };
        let reply = s.on_ack(&ack, Duration::from_millis(1)).unwrap();
        assert_eq!(s.buffered_count(), 1); // seq 2 remains (not < last_ack_seq)
        assert!(s.rtt() < Duration::from_millis(100)); // moved from the 100ms init toward 20ms

        let ControlPacket::AckAck(ackack) = ControlPacket::parse(&reply).unwrap() else {
            panic!("expected ACKACK");
        };
        assert_eq!(ackack.ack_number, 1);
    }

    #[test]
    fn light_ack_does_not_trigger_ackack() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Duration::ZERO);
        let ack = AckPacket {
            ack_number: 0,
            timestamp: 0,
            dest_socket_id: PEER,
            cif: AckCif::Light { last_ack_seq: 1 },
        };
        assert!(s.on_ack(&ack, Duration::ZERO).is_none());
        assert_eq!(s.buffered_count(), 0);
    }
}
