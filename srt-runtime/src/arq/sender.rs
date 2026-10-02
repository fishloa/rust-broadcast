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

use crate::error::{Error, Result};
use crate::packet::data::MESSAGE_NUMBER_MASK;
use crate::packet::{
    AckAckPacket, AckCif, AckPacket, ControlPacket, DataPacket, EncryptionKeyField, LossListEntry,
    NakPacket, PacketPosition, SEQ_NUMBER_MASK,
};

use super::rtt::RttEstimator;
use super::{duration_to_wire_us, seq};

/// A NAK loss-list range is a compact wire encoding (Appendix A), but
/// nothing stops a malformed/adversarial range from declaring billions of
/// entries. [`Sender::on_nak`] never expands a range: it intersects it with
/// the (small) send buffer, so the work per range is bounded by what is
/// actually buffered. This is not a `specs/rules/srt-arq.md` rule — it is a
/// further safety cap on how many buffered packets one NAK datagram may mark
/// for retransmission, bounding the work a single (possibly spoofed) NAK can
/// cause.
const MAX_NAK_SEQS_PER_PACKET: usize = 1 << 16;

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
    /// Work counter for the buffer lookups (one increment per buffer entry
    /// probed or visited while resolving a NAK range or a retransmit) — the
    /// deterministic evidence that a NAK costs `O(log buffer)` per entry plus
    /// the buffered packets it really names, never `O(buffer × range)`.
    lookup_steps: u64,
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
            lookup_steps: 0,
        }
    }

    /// Index of the buffered packet with sequence number `seq`, by binary
    /// search over the send buffer (which is held in strictly increasing
    /// circular sequence order — [`Self::on_data`]'s contract).
    fn index_of(&mut self, seq: u32) -> Option<usize> {
        let (mut lo, mut hi) = (0usize, self.buffer.len());
        while lo < hi {
            self.lookup_steps = self.lookup_steps.saturating_add(1);
            let mid = lo + (hi - lo) / 2;
            let d = seq::seq_diff(self.buffer[mid].seq, seq);
            match d.cmp(&0) {
                core::cmp::Ordering::Equal => return Some(mid),
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// Index of the first buffered packet at or after `seq` (circular
    /// order), `buffer.len()` if there is none.
    fn first_at_or_after(&mut self, seq: u32) -> usize {
        let (mut lo, mut hi) = (0usize, self.buffer.len());
        while lo < hi {
            self.lookup_steps = self.lookup_steps.saturating_add(1);
            let mid = lo + (hi - lo) / 2;
            if seq::seq_diff(self.buffer[mid].seq, seq) < 0 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
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
    ///
    /// `seq` must increase (circularly) from one call to the next — the send
    /// buffer is searched by binary search, and a debug build asserts it.
    ///
    /// # Errors
    /// [`Error::FieldTooWide`] if `seq` does not fit its 31-bit field or
    /// `message_number` its 26-bit one; nothing is buffered then.
    pub fn on_data(
        &mut self,
        seq: u32,
        message_number: u32,
        payload: &[u8],
        now: Duration,
    ) -> Result<Vec<u8>> {
        // The wire fields are 31 and 26 bits wide (§3.1): a wider value is a
        // caller bug, reported rather than silently wrapped into another
        // packet's number.
        if seq > SEQ_NUMBER_MASK {
            return Err(Error::FieldTooWide {
                what: "Packet Sequence Number",
                value: u64::from(seq),
                bits: 31,
            });
        }
        if message_number > MESSAGE_NUMBER_MASK {
            return Err(Error::FieldTooWide {
                what: "Message Number",
                value: u64::from(message_number),
                bits: 26,
            });
        }
        debug_assert!(
            self.buffer
                .back()
                .is_none_or(|last| seq::seq_lt(last.seq, seq)),
            "Sender::on_data requires circularly increasing sequence numbers"
        );
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
        pkt.serialize_into(&mut buf)?;
        self.buffer.push_back(SentPacket {
            seq,
            message_number,
            payload: payload.to_vec(),
            resend_count: 0,
        });
        Ok(buf)
    }

    /// Record a NAK's loss-list entries for prioritized retransmission
    /// (`specs/rules/srt-arq.md` rules 5, 15, 16, 18). Entries no longer in
    /// the send buffer (already freed by a since-received ACK) are silently
    /// ignored (rule 17).
    ///
    /// Each entry is resolved against the send buffer by binary search, and a
    /// range is intersected with the buffered window rather than expanded, so
    /// the work is bounded by the buffer and by `MAX_NAK_SEQS_PER_PACKET`
    /// no matter what range a (possibly spoofed) NAK declares. A range whose
    /// end precedes its start is malformed and ignored.
    pub fn on_nak(&mut self, nak: &NakPacket<'_>) {
        let mut budget = MAX_NAK_SEQS_PER_PACKET;
        for entry in nak.entries() {
            if budget == 0 {
                break;
            }
            let Ok(entry) = entry else { continue };
            match entry {
                LossListEntry::Single(s) => {
                    if self.index_of(s).is_some() {
                        self.pending_retransmit.insert(s);
                    }
                    budget -= 1;
                }
                LossListEntry::Range(first, last) => {
                    // A range whose `last` precedes `first` (malformed) yields
                    // nothing: its first candidate is already past `last`.
                    let mut i = self.first_at_or_after(first);
                    while budget > 0 && i < self.buffer.len() {
                        self.lookup_steps = self.lookup_steps.saturating_add(1);
                        let s = self.buffer[i].seq;
                        if seq::seq_diff(s, last) > 0 {
                            break;
                        }
                        self.pending_retransmit.insert(s);
                        budget -= 1;
                        i += 1;
                    }
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
        let mut seqs: Vec<u32> = core::mem::take(&mut self.pending_retransmit)
            .into_iter()
            .collect();
        // Oldest first in *circular* order: the set is ordered by raw value,
        // which interleaves the two sides of a sequence-number wrap.
        if let Some(front) = self.buffer.front() {
            let base = front.seq;
            seqs.sort_unstable_by_key(|&s| seq::seq_diff(s, base));
        }
        let mut out = Vec::with_capacity(seqs.len());
        for seq in seqs {
            let Some(idx) = self.index_of(seq) else {
                continue; // rule 17: already dropped from the buffer.
            };
            let sent = &mut self.buffer[idx];
            sent.resend_count = sent.resend_count.saturating_add(1);
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
            // Built from an already-buffered (so in-range) packet: cannot fail,
            // but a failure drops this retransmit rather than panicking.
            if pkt.serialize_into(&mut buf).is_ok() {
                out.push(buf);
            }
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
        while self
            .buffer
            .front()
            .is_some_and(|front| seq::seq_lt(front.seq, last_ack_seq))
        {
            if let Some(freed) = self.buffer.pop_front() {
                self.pending_retransmit.remove(&freed.seq); // rule 17
            }
        }
        // rule 17: nothing more to purge — `on_nak` only ever queues a seq
        // that is in the buffer, and the loop above removed each freed seq
        // from the queue, so the queue is still a subset of the buffer
        // (no per-ACK rebuild of the whole buffer's seq set).

        if let AckCif::Full { rtt_us, .. } = ack.cif {
            // rule 33: same EWMA as rules 29-30, `rtt` = the ACK's carried
            // value.
            self.rtt.update(Duration::from_micros(u64::from(rtt_us)));

            let pkt = ControlPacket::AckAck(AckAckPacket {
                ack_number: ack.ack_number,
                timestamp: duration_to_wire_us(now),
                dest_socket_id: self.dest_socket_id,
                libsrt_pad: true,
            });
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut buf).ok()?;
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
        let bytes = s.on_data(5, 5, b"hello", Duration::from_millis(1)).unwrap();
        assert_eq!(s.buffered_count(), 1);
        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, 5);
        assert!(!dp.retransmitted);
        assert_eq!(dp.data, b"hello");
    }

    #[test]
    fn nak_then_tick_retransmits_with_r_flag_set() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Duration::ZERO).unwrap();
        s.on_data(1, 1, b"b", Duration::ZERO).unwrap();

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
        s.on_data(0, 0, b"a", Duration::ZERO).unwrap();
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
        s.on_data(0, 0, b"a", Duration::ZERO).unwrap();
        s.on_data(1, 1, b"b", Duration::ZERO).unwrap();
        s.on_data(2, 2, b"c", Duration::ZERO).unwrap();

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
        s.on_data(0, 0, b"a", Duration::ZERO).unwrap();
        let ack = AckPacket {
            ack_number: 0,
            timestamp: 0,
            dest_socket_id: PEER,
            cif: AckCif::Light { last_ack_seq: 1 },
        };
        assert!(s.on_ack(&ack, Duration::ZERO).is_none());
        assert_eq!(s.buffered_count(), 0);
    }

    fn nak_for(entries: &[LossListEntry]) -> Vec<u8> {
        nak_bytes(entries)
    }

    fn feed_nak(s: &mut Sender, entries: &[LossListEntry]) {
        let raw = nak_for(entries);
        let ControlPacket::Nak(nak) = ControlPacket::parse(&raw).unwrap() else {
            panic!("expected NAK");
        };
        s.on_nak(&nak);
    }

    /// r08-SRT-W1: a NAK of ~180 maximal range entries against a 10 000-packet
    /// buffer used to cost `180 × 65 537 × buffer` comparisons. The lookup
    /// work counter (incremented on every buffer entry probed or visited)
    /// must stay within the per-NAK cap plus a logarithmic probe per entry.
    #[test]
    fn nak_with_huge_ranges_does_bounded_buffer_work() {
        const BUFFERED: u32 = 10_000;
        const RANGES: usize = 180;
        let mut s = Sender::new(PEER);
        for q in 0..BUFFERED {
            s.on_data(q, q, b"x", Duration::ZERO).unwrap();
        }
        // 180 entries each claiming [5 000, 0x3FFF_FFFF]; 180 x 8 B = 1 440 B
        // fits one datagram.
        let entries = alloc::vec![LossListEntry::Range(5_000, 0x3FFF_FFFF); RANGES];
        feed_nak(&mut s, &entries);
        assert_eq!(s.pending_retransmit_count(), (BUFFERED - 5_000) as usize);
        // log2(10 000) < 14 probes to find each range start.
        let bound = (MAX_NAK_SEQS_PER_PACKET + RANGES * 14) as u64;
        assert!(
            s.lookup_steps <= bound,
            "lookup work {} exceeds the bound {bound}",
            s.lookup_steps
        );
    }

    /// r08-SRT-W1: a single NAK entry costs a logarithmic lookup, not a scan.
    #[test]
    fn single_nak_entry_lookup_is_logarithmic() {
        let mut s = Sender::new(PEER);
        for q in 0..8192u32 {
            s.on_data(q, q, b"x", Duration::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Single(8000)]);
        assert_eq!(s.pending_retransmit_count(), 1);
        assert!(
            s.lookup_steps <= 14,
            "log2(8192) = 13 probes, got {}",
            s.lookup_steps
        );
        // And the retransmit lookup in `tick` is logarithmic too.
        let before = s.lookup_steps;
        let out = s.tick(Duration::ZERO);
        assert_eq!(out.len(), 1);
        assert!(s.lookup_steps - before <= 14);
        assert_eq!(DataPacket::parse(&out[0]).unwrap().seq_number, 8000);
    }

    /// A NAK range that straddles the 31-bit sequence wrap selects exactly
    /// the buffered packets inside it, and they are retransmitted oldest
    /// first in circular order (the pending set is ordered by raw value,
    /// which would send `0, 1` before `MAX - 1, MAX`).
    #[test]
    fn nak_range_across_the_sequence_wrap_retransmits_in_circular_order() {
        const MAX: u32 = crate::packet::SEQ_NUMBER_MASK;
        let mut s = Sender::new(PEER);
        for q in [MAX - 2, MAX - 1, MAX, 0, 1, 2] {
            s.on_data(q, 1, b"x", Duration::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(MAX - 1, 1)]);
        assert_eq!(s.pending_retransmit_count(), 4);
        let seqs: Vec<u32> = s
            .tick(Duration::ZERO)
            .iter()
            .map(|b| DataPacket::parse(b).unwrap().seq_number)
            .collect();
        assert_eq!(seqs, alloc::vec![MAX - 1, MAX, 0, 1]);
    }

    /// A range whose end precedes its start is malformed and names nothing.
    #[test]
    fn malformed_nak_range_names_nothing() {
        let mut s = Sender::new(PEER);
        for q in 0..20u32 {
            s.on_data(q, q, b"x", Duration::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(10, 5)]);
        assert_eq!(s.pending_retransmit_count(), 0);
        // Beyond half the sequence space is also "before" in circular order.
        feed_nak(&mut s, &[LossListEntry::Range(0, 0x5000_0000)]);
        assert_eq!(s.pending_retransmit_count(), 0);
    }

    /// r08-SRT-O3: an ACK drops exactly the freed sequence numbers from the
    /// retransmit queue and leaves the rest (it no longer rebuilds a set of
    /// the whole buffer to do it).
    #[test]
    fn ack_purges_only_freed_seqs_from_the_retransmit_queue() {
        let mut s = Sender::new(PEER);
        for q in 0..6u32 {
            s.on_data(q, q, b"x", Duration::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(1, 4)]);
        assert_eq!(s.pending_retransmit_count(), 4);
        let ack = AckPacket {
            ack_number: 0,
            timestamp: 0,
            dest_socket_id: PEER,
            cif: AckCif::Light { last_ack_seq: 3 },
        };
        assert!(s.on_ack(&ack, Duration::ZERO).is_none());
        // seqs 1, 2 freed; 3, 4 still pending and still buffered.
        assert_eq!(s.pending_retransmit_count(), 2);
        let seqs: Vec<u32> = s
            .tick(Duration::ZERO)
            .iter()
            .map(|b| DataPacket::parse(b).unwrap().seq_number)
            .collect();
        assert_eq!(seqs, alloc::vec![3, 4]);
    }

    /// Wire fields are 31 / 26 bits: wider inputs are an error (nothing is
    /// buffered), the widest legal values are fine.
    #[test]
    fn on_data_rejects_values_wider_than_their_wire_fields() {
        let mut s = Sender::new(PEER);
        assert!(matches!(
            s.on_data(SEQ_NUMBER_MASK + 1, 0, b"x", Duration::ZERO),
            Err(Error::FieldTooWide { bits: 31, .. })
        ));
        assert!(matches!(
            s.on_data(0, MESSAGE_NUMBER_MASK + 1, b"x", Duration::ZERO),
            Err(Error::FieldTooWide { bits: 26, .. })
        ));
        assert_eq!(s.buffered_count(), 0, "a rejected packet is not buffered");
        let bytes = s
            .on_data(SEQ_NUMBER_MASK, MESSAGE_NUMBER_MASK, b"x", Duration::ZERO)
            .unwrap();
        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, SEQ_NUMBER_MASK);
        assert_eq!(dp.message_number, MESSAGE_NUMBER_MASK);
        feed_nak(&mut s, &[LossListEntry::Single(SEQ_NUMBER_MASK)]);
        assert_eq!(s.pending_retransmit_count(), 1);
    }
}
