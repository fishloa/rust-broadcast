//! ARQ sender-side reliability state — `draft-sharabayko-srt-01` §4.8
//! (Acknowledgement and Lost Packet Handling), §4.8.1 (ACKs/ACKACKs), §4.8.2
//! (NAKs), §4.10 (RTT). Curated rules: `private/specs-verified/rules/srt-arq.md`.
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
use broadcast_common::stage::Timestamp;
use core::time::Duration;

use crate::error::Result;
use crate::packet::number::{MESSAGE_NUMBER_BITS, SocketId};
use crate::packet::{
    AckAckPacket, AckCif, AckPacket, ControlPacket, DataPacket, EncryptionKeyField, LossListEntry,
    NakPacket, PacketPosition,
};

use super::rtt::RttEstimator;
use super::{seq, timestamp_to_wire_us};

/// A NAK loss-list range is a compact wire encoding (Appendix A), but
/// nothing stops a malformed/adversarial range from declaring billions of
/// entries. [`Sender::on_nak`] never expands a range: it intersects it with
/// the (small) send buffer, so the work per range is bounded by what is
/// actually buffered. This is not a `private/specs-verified/rules/srt-arq.md` rule — it is a
/// further safety cap on how many buffered packets one NAK datagram may mark
/// for retransmission, bounding the work a single (possibly spoofed) NAK can
/// cause.
const MAX_NAK_SEQS_PER_PACKET: usize = 1 << 16;

/// Shortest interval between two retransmissions of the same sequence number
/// (#2819), the same implementation-policy value `rist-runtime` uses for its
/// own sender. `draft-sharabayko-srt-01` does not state one; without it a peer
/// re-sending one NAK back to back could keep every buffered packet in the
/// driver's outbound queue forever.
const MIN_RETRANSMIT_INTERVAL: Duration = Duration::from_millis(20);

/// Divisor taking the midpoint of a bisection range.
const BISECTION_DIVISOR: usize = 2;

/// Bit width of the Packet Sequence Number the NAK/Drop Request CIFs carry
/// (§3.2.5, §3.2.7).
const SEQ_NUMBER_BITS: u32 = 31;

/// One buffered, sent-but-not-yet-acknowledged data packet (rules 1, 16,
/// 18).
#[derive(Debug, Clone)]
struct SentPacket {
    seq: u32,
    message_number: u32,
    payload: Vec<u8>,
    /// `KK` flag for this packet (NotEncrypted for plaintext connections).
    key_flag: EncryptionKeyField,
    /// Resend counter (rule 18): incremented on every retransmission.
    resend_count: u32,
    /// This packet's retransmission is already in the driver's outbound
    /// queue, not yet sent. A repeated NAK must not queue a second copy
    /// (r17-SRT-I6/#2819); the flag clears when the datagram leaves that
    /// queue.
    queued_retransmit: bool,
    /// When this packet was last handed out for retransmission, so one
    /// sequence number cannot be retransmitted faster than
    /// [`MIN_RETRANSMIT_INTERVAL`].
    last_retransmit_at: Option<Timestamp>,
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
            let mid = lo + (hi - lo) / BISECTION_DIVISOR;
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
            let mid = lo + (hi - lo) / BISECTION_DIVISOR;
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
    /// [`crate::error::Error::FieldOverflow`] if `seq` does not fit its 31-bit field or
    /// `message_number` its 26-bit one; nothing is buffered then.
    pub fn on_data(
        &mut self,
        seq: u32,
        message_number: u32,
        payload: &[u8],
        now: Timestamp,
    ) -> Result<Vec<u8>> {
        // The wire fields are 31 and 26 bits wide (§3.1): a wider value is a
        // caller bug, reported rather than silently wrapped into another
        // packet's number.
        broadcast_core::len::fit_bits(u64::from(seq), SEQ_NUMBER_BITS, "Packet Sequence Number")?;
        broadcast_core::len::fit_bits(
            u64::from(message_number),
            MESSAGE_NUMBER_BITS,
            "Message Number",
        )?;
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
            timestamp: timestamp_to_wire_us(now),
            dest_socket_id: SocketId::from_wire(self.dest_socket_id),
            data: payload,
        };
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)?;
        self.buffer.push_back(SentPacket {
            seq,
            message_number,
            payload: payload.to_vec(),
            key_flag: EncryptionKeyField::NotEncrypted,
            resend_count: 0,
            queued_retransmit: false,
            last_retransmit_at: None,
        });
        Ok(buf)
    }
    /// Like [`Self::on_data`] but records a `KK` flag for the packet (used by
    /// the tokio adapter's encrypted data path). The payload is already
    /// encrypted by the caller.
    pub fn on_data_keyed(
        &mut self,
        seq: u32,
        message_number: u32,
        payload: &[u8],
        key_flag: EncryptionKeyField,
        now: Timestamp,
    ) -> Result<Vec<u8>> {
        broadcast_core::len::fit_bits(u64::from(seq), SEQ_NUMBER_BITS, "Packet Sequence Number")?;
        broadcast_core::len::fit_bits(
            u64::from(message_number),
            MESSAGE_NUMBER_BITS,
            "Message Number",
        )?;
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
            key_flag,
            retransmitted: false,
            message_number,
            timestamp: timestamp_to_wire_us(now),
            dest_socket_id: SocketId::from_wire(self.dest_socket_id),
            data: payload,
        };
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)?;
        self.buffer.push_back(SentPacket {
            seq,
            message_number,
            payload: payload.to_vec(),
            key_flag,
            resend_count: 0,
            queued_retransmit: false,
            last_retransmit_at: None,
        });
        Ok(buf)
    }

    /// Record a NAK's loss-list entries for prioritized retransmission
    /// (`private/specs-verified/rules/srt-arq.md` rules 5, 15, 16, 18). Entries no longer in
    /// the send buffer (already freed by a since-received ACK) are silently
    /// ignored (rule 17).
    ///
    /// Each entry is resolved against the send buffer by binary search, and a
    /// range is intersected with the buffered window rather than expanded, so
    /// the work is bounded by the buffer and by `MAX_NAK_SEQS_PER_PACKET`
    /// no matter what range a (possibly spoofed) NAK declares. A range whose
    /// end precedes its start is malformed and ignored.
    pub fn on_nak(&mut self, nak: &NakPacket) {
        let mut budget = MAX_NAK_SEQS_PER_PACKET;
        for &entry in &nak.entries {
            if budget == 0 {
                break;
            }
            match entry {
                LossListEntry::Single(s) => {
                    let s = s.value();
                    if self.index_of(s).is_some() {
                        self.pending_retransmit.insert(s);
                    }
                    budget -= 1;
                }
                LossListEntry::Range(first, last) => {
                    let (first, last) = (first.value(), last.value());
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

    /// Drain the pending retransmit queue, returning each retransmission's
    /// sequence number and wire bytes (rules 16, 18 — the `R` flag is set,
    /// the resend counter incremented). A queued sequence number no longer
    /// in the buffer (freed by a since-received ACK) is dropped without
    /// emitting anything (rule 17).
    ///
    /// A sequence number already queued in the caller's outbound queue
    /// ([`Self::on_retransmit_sent`] clears it once the datagram leaves) or
    /// retransmitted less than `MIN_RETRANSMIT_INTERVAL` ago stays pending:
    /// a peer re-sending the same NAK cannot queue a second copy (#2819).
    pub fn tick(&mut self, now: Timestamp) -> Vec<(u32, Vec<u8>)> {
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
        let mut still_pending = BTreeSet::new();
        for seq in seqs {
            let Some(idx) = self.index_of(seq) else {
                continue; // rule 17: already dropped from the buffer.
            };
            let sent = &mut self.buffer[idx];
            let rate_limited = sent
                .last_retransmit_at
                .is_some_and(|last| now.saturating_sub(last) < MIN_RETRANSMIT_INTERVAL);
            if sent.queued_retransmit || rate_limited {
                still_pending.insert(seq);
                continue;
            }
            sent.queued_retransmit = true;
            sent.last_retransmit_at = Some(now);
            sent.resend_count = sent.resend_count.saturating_add(1);
            let pkt = DataPacket {
                seq_number: sent.seq,
                position: PacketPosition::Solo,
                in_order: true,
                key_flag: sent.key_flag,
                retransmitted: true,
                message_number: sent.message_number,
                timestamp: timestamp_to_wire_us(now),
                dest_socket_id: SocketId::from_wire(self.dest_socket_id),
                data: &sent.payload,
            };
            let mut buf = alloc::vec![0u8; pkt.serialized_len()];
            // Built from an already-buffered (so in-range) packet: cannot fail,
            // but a failure drops this retransmit rather than panicking.
            if pkt.serialize_into(&mut buf).is_ok() {
                out.push((seq, buf));
            } else {
                sent.queued_retransmit = false;
            }
        }
        self.pending_retransmit = still_pending;
        out
    }

    /// The retransmission of `seq` left the caller's outbound queue: it may
    /// be queued again by a later [`Self::tick`] (a NAK asking for it again,
    /// or the rate limit having passed).
    pub fn on_retransmit_sent(&mut self, seq: u32) {
        if let Some(idx) = self.index_of(seq) {
            self.buffer[idx].queued_retransmit = false;
        }
    }

    /// The retransmission of `seq` was dropped instead of queued (the
    /// caller's outbound queue was full): clear the queued mark and put it
    /// back in the pending set, so a later tick offers it again.
    pub fn on_retransmit_dropped(&mut self, seq: u32) {
        self.on_retransmit_sent(seq);
        self.pending_retransmit.insert(seq);
    }

    /// Process an incoming ACK: free every acknowledged packet (rules 7, 8,
    /// 16, 17), and — for a Full ACK only — update RTT/RTTVar (rule 33) and
    /// return the ACKACK reply (rules 3, 9).
    pub fn on_ack(&mut self, ack: &AckPacket, now: Timestamp) -> Option<Vec<u8>> {
        let last_ack_seq = match ack.cif {
            AckCif::Full { last_ack_seq, .. }
            | AckCif::Small { last_ack_seq, .. }
            | AckCif::Light { last_ack_seq } => last_ack_seq,
        };
        if let Some(back) = self.buffer.back()
            && seq::seq_gt(last_ack_seq.value(), seq::seq_next(back.seq))
        {
            return None;
        }
        // rule 8: every seq strictly before `last_ack_seq` is acknowledged.
        while self
            .buffer
            .front()
            .is_some_and(|front| seq::seq_lt(front.seq, last_ack_seq.value()))
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
                timestamp: timestamp_to_wire_us(now),
                dest_socket_id: SocketId::from_wire(self.dest_socket_id),
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
    use crate::error::Error;
    use crate::packet::SEQ_NUMBER_MASK;
    use crate::packet::data::MESSAGE_NUMBER_MASK;
    use crate::packet::number::{AckNumber, SeqNo, SocketId, SrtTimestamp};

    const PEER: u32 = 0xAAAA;

    fn sn(v: u32) -> SeqNo {
        assert!(v <= SEQ_NUMBER_MASK, "{v} is not a 31-bit sequence number");
        SeqNo::from_wire_masked(v)
    }

    fn nak_bytes(entries: &[LossListEntry]) -> Vec<u8> {
        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(PEER),
            entries: entries.to_vec(),
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn on_data_buffers_and_returns_wire_bytes() {
        let mut s = Sender::new(PEER);
        let bytes = s
            .on_data(5, 5, b"hello", Timestamp::from_millis(1))
            .unwrap();
        assert_eq!(s.buffered_count(), 1);
        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, 5);
        assert!(!dp.retransmitted);
        assert_eq!(dp.data, b"hello");
    }

    #[test]
    fn nak_then_tick_retransmits_with_r_flag_set() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Timestamp::ZERO).unwrap();
        s.on_data(1, 1, b"b", Timestamp::ZERO).unwrap();

        let raw = nak_bytes(&[LossListEntry::Single(sn(1))]);
        let ControlPacket::Nak(nak) = ControlPacket::parse(&raw).unwrap() else {
            panic!("expected NAK");
        };
        s.on_nak(&nak);
        assert_eq!(s.pending_retransmit_count(), 1);

        let out = s.tick(Timestamp::from_millis(5));
        assert_eq!(out.len(), 1);
        let dp = DataPacket::parse(&out[0].1).unwrap();
        assert_eq!(dp.seq_number, 1);
        assert!(dp.retransmitted);
        assert_eq!(s.pending_retransmit_count(), 0);
    }

    /// #2819: a sequence number whose retransmission is already queued is
    /// never queued twice, and one already retransmitted is rate-limited to
    /// one emission per `MIN_RETRANSMIT_INTERVAL`; both stay pending rather
    /// than disappearing, so a later tick still sends them.
    #[test]
    fn a_queued_retransmit_is_not_queued_twice_and_is_rate_limited() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 1, b"a", Timestamp::ZERO).unwrap();

        feed_nak(&mut s, &[LossListEntry::Single(sn(0))]);
        assert_eq!(s.tick(Timestamp::from_millis(1)).len(), 1);

        // A second NAK while the first copy is still queued in the driver.
        feed_nak(&mut s, &[LossListEntry::Single(sn(0))]);
        assert_eq!(
            s.tick(Timestamp::from_millis(2)).len(),
            0,
            "a queued retransmission must not be queued again"
        );
        assert_eq!(s.pending_retransmit_count(), 1, "but it stays pending");

        // The datagram left the queue; the interval has not passed yet.
        s.on_retransmit_sent(0);
        assert_eq!(s.tick(Timestamp::from_millis(3)).len(), 0);
        assert_eq!(s.pending_retransmit_count(), 1);

        let at_interval = Timestamp::from_millis(1)
            .checked_add(MIN_RETRANSMIT_INTERVAL)
            .expect("deadline in range");
        let again = s.tick(at_interval);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].0, 0);
        assert_eq!(s.pending_retransmit_count(), 0);
    }

    #[test]
    fn nak_for_unbuffered_seq_is_ignored() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Timestamp::ZERO).unwrap();
        let raw = nak_bytes(&[LossListEntry::Single(sn(99))]);
        let ControlPacket::Nak(nak) = ControlPacket::parse(&raw).unwrap() else {
            panic!("expected NAK");
        };
        s.on_nak(&nak);
        assert_eq!(s.pending_retransmit_count(), 0);
        assert!(s.tick(Timestamp::ZERO).is_empty());
    }

    #[test]
    fn full_ack_frees_buffer_and_updates_rtt_and_replies_ackack() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Timestamp::ZERO).unwrap();
        s.on_data(1, 1, b"b", Timestamp::ZERO).unwrap();
        s.on_data(2, 2, b"c", Timestamp::ZERO).unwrap();

        let ack = AckPacket {
            ack_number: AckNumber::from_wire(1),
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(PEER),
            cif: AckCif::Full {
                last_ack_seq: sn(2),
                rtt_us: 20_000,
                rtt_var_us: 5_000,
                avail_buf_size: 0,
                pkt_recv_rate: 0,
                est_link_capacity: 0,
                recv_rate_bps: 0,
            },
        };
        let reply = s.on_ack(&ack, Timestamp::from_millis(1)).unwrap();
        assert_eq!(s.buffered_count(), 1); // seq 2 remains (not < last_ack_seq)
        // rule 29: (7*100 000 + 20 000) / 8 = 90 000 us.
        assert_eq!(s.rtt(), Duration::from_micros(90_000));
        // rule 30: (3*50 000 + |100 000 - 20 000|) / 4 = 57 500 us.
        assert_eq!(s.rtt_var(), Duration::from_micros(57_500));

        let ControlPacket::AckAck(ackack) = ControlPacket::parse(&reply).unwrap() else {
            panic!("expected ACKACK");
        };
        assert_eq!(ackack.ack_number.value(), 1);
    }

    #[test]
    fn light_ack_does_not_trigger_ackack() {
        let mut s = Sender::new(PEER);
        s.on_data(0, 0, b"a", Timestamp::ZERO).unwrap();
        let ack = AckPacket {
            ack_number: AckNumber::ZERO,
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(PEER),
            cif: AckCif::Light {
                last_ack_seq: sn(1),
            },
        };
        assert!(s.on_ack(&ack, Timestamp::ZERO).is_none());
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
            s.on_data(q, q, b"x", Timestamp::ZERO).unwrap();
        }
        // 180 entries each claiming [5 000, 0x3FFF_FFFF]; 180 x 8 B = 1 440 B
        // fits one datagram.
        let entries = alloc::vec![LossListEntry::Range(sn(5_000), sn(0x3FFF_FFFF)); RANGES];
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
            s.on_data(q, q, b"x", Timestamp::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Single(sn(8000))]);
        assert_eq!(s.pending_retransmit_count(), 1);
        assert!(
            s.lookup_steps <= 14,
            "log2(8192) = 13 probes, got {}",
            s.lookup_steps
        );
        // And the retransmit lookup in `tick` is logarithmic too.
        let before = s.lookup_steps;
        let out = s.tick(Timestamp::ZERO);
        assert_eq!(out.len(), 1);
        assert!(s.lookup_steps - before <= 14);
        assert_eq!(DataPacket::parse(&out[0].1).unwrap().seq_number, 8000);
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
            s.on_data(q, 1, b"x", Timestamp::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(sn(MAX - 1), sn(1))]);
        assert_eq!(s.pending_retransmit_count(), 4);
        let seqs: Vec<u32> = s
            .tick(Timestamp::ZERO)
            .iter()
            .map(|(seq, _)| *seq)
            .collect();
        assert_eq!(seqs, alloc::vec![MAX - 1, MAX, 0, 1]);
    }

    /// A range whose end precedes its start is malformed and names nothing.
    #[test]
    fn malformed_nak_range_names_nothing() {
        let mut s = Sender::new(PEER);
        for q in 0..20u32 {
            s.on_data(q, q, b"x", Timestamp::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(sn(10), sn(5))]);
        assert_eq!(s.pending_retransmit_count(), 0);
        // Beyond half the sequence space is also "before" in circular order.
        feed_nak(&mut s, &[LossListEntry::Range(sn(0), sn(0x5000_0000))]);
        assert_eq!(s.pending_retransmit_count(), 0);
    }

    /// r08-SRT-O3: an ACK drops exactly the freed sequence numbers from the
    /// retransmit queue and leaves the rest (it no longer rebuilds a set of
    /// the whole buffer to do it).
    #[test]
    fn ack_purges_only_freed_seqs_from_the_retransmit_queue() {
        let mut s = Sender::new(PEER);
        for q in 0..6u32 {
            s.on_data(q, q, b"x", Timestamp::ZERO).unwrap();
        }
        feed_nak(&mut s, &[LossListEntry::Range(sn(1), sn(4))]);
        assert_eq!(s.pending_retransmit_count(), 4);
        let ack = AckPacket {
            ack_number: AckNumber::ZERO,
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(PEER),
            cif: AckCif::Light {
                last_ack_seq: sn(3),
            },
        };
        assert!(s.on_ack(&ack, Timestamp::ZERO).is_none());
        // seqs 1, 2 freed; 3, 4 still pending and still buffered.
        assert_eq!(s.pending_retransmit_count(), 2);
        let seqs: Vec<u32> = s
            .tick(Timestamp::ZERO)
            .iter()
            .map(|(seq, _)| *seq)
            .collect();
        assert_eq!(seqs, alloc::vec![3, 4]);
    }

    /// One NAK may name at most `MAX_NAK_SEQS_PER_PACKET` buffered packets:
    /// the per-NAK work budget, decremented per single entry, runs out before
    /// the (unbounded) loss list does.
    #[test]
    fn on_nak_stops_at_the_per_packet_work_budget() {
        let mut s = Sender::new(PEER);
        let total = MAX_NAK_SEQS_PER_PACKET + 1000;
        for q in 0..u32::try_from(total).expect("fits") {
            s.on_data(q, q, b"x", Timestamp::ZERO).unwrap();
        }
        let entries: Vec<LossListEntry> = (0..total)
            .map(|q| LossListEntry::Single(sn(u32::try_from(q).expect("fits"))))
            .collect();
        s.on_nak(&NakPacket {
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(PEER),
            entries,
        });
        assert_eq!(s.pending_retransmit_count(), MAX_NAK_SEQS_PER_PACKET);
    }

    /// Wire fields are 31 / 26 bits: wider inputs are an error (nothing is
    /// buffered), the widest legal values are fine.
    #[test]
    fn on_data_rejects_values_wider_than_their_wire_fields() {
        let mut s = Sender::new(PEER);
        assert!(matches!(
            s.on_data(SEQ_NUMBER_MASK + 1, 0, b"x", Timestamp::ZERO),
            Err(Error::FieldOverflow(e)) if e.max == u64::from(SEQ_NUMBER_MASK)
        ));
        assert!(matches!(
            s.on_data(0, MESSAGE_NUMBER_MASK + 1, b"x", Timestamp::ZERO),
            Err(Error::FieldOverflow(e)) if e.max == u64::from(MESSAGE_NUMBER_MASK)
        ));
        assert_eq!(s.buffered_count(), 0, "a rejected packet is not buffered");
        let bytes = s
            .on_data(SEQ_NUMBER_MASK, MESSAGE_NUMBER_MASK, b"x", Timestamp::ZERO)
            .unwrap();
        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, SEQ_NUMBER_MASK);
        assert_eq!(dp.message_number, MESSAGE_NUMBER_MASK);
        feed_nak(&mut s, &[LossListEntry::Single(sn(SEQ_NUMBER_MASK))]);
        assert_eq!(s.pending_retransmit_count(), 1);
    }
}
