//! ARQ receiver-side reliability state — `draft-sharabayko-srt-01` §4.8.1
//! (Full/Light ACK generation), §4.8.2 (loss detection + NAK), §4.10 (RTT
//! measurement via the ACK/ACKACK round trip). Curated rules:
//! `specs/rules/srt-arq.md`.
//!
//! Sans-IO: [`Receiver`] never reads a wall clock — [`Receiver::feed_data`]
//! and [`Receiver::tick`] take a caller-supplied `now: core::time::Duration`
//! (elapsed time since a fixed epoch the caller owns).
//!
//! # Delivery model
//! [`FeedOutcome::delivered`] lists the sequence numbers that became
//! cumulatively in-order-deliverable as a result of one `feed_data` call —
//! i.e. the ARQ engine's view of "no longer missing" (rule 8's ack point),
//! not a TSBPD-timed playout (that delay is `srt-tsbpd.md` scope, a
//! separate follow-up).
//!
//! # Non-goals
//! TLPKTDROP fake-ACK skip handling (rule 13) is not modeled — see the
//! `arq` module doc.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::time::Duration;

use crate::packet::nak::build_loss_list;
use crate::packet::{AckAckPacket, AckCif, AckPacket, ControlPacket, LossListEntry, NakPacket};

use super::rtt::RttEstimator;
use super::{FULL_ACK_PERIOD, LIGHT_ACK_THRESHOLD, duration_to_wire_us, nak_interval, seq};

/// A bound on how many individual sequence numbers one [`Receiver::feed_data`]
/// call will enumerate into the loss list for a single newly-detected gap.
/// Not a `specs/rules/srt-arq.md` rule — a safety cap against a corrupt or
/// adversarial sequence-number jump causing unbounded work (mirrors the
/// NAK-side cap in `arq::sender::expand_loss_entry`).
const MAX_GAP_EXPANSION: u32 = 1 << 16;

/// Outcome of one [`Receiver::feed_data`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FeedOutcome {
    /// Sequence numbers that became cumulatively in-order-deliverable as a
    /// result of this packet's arrival — includes `seq` itself when it was
    /// the next-expected packet, plus any previously-buffered out-of-order
    /// packets it consequently unblocked.
    pub delivered: Vec<u32>,
    /// An immediate NAK to send, if this packet's arrival revealed a new
    /// gap (`specs/rules/srt-arq.md` rules 4, 14).
    pub nak: Option<Vec<u8>>,
}

/// ARQ receiver-side state (`draft-sharabayko-srt-01` §4.8.1/§4.8.2/§4.10).
#[derive(Debug)]
pub struct Receiver {
    dest_socket_id: u32,
    /// Cumulative ack point: every seq strictly before this has been
    /// delivered in order (rule 8).
    next_expected: u32,
    /// Received but not yet in-order-deliverable (a gap remains below it).
    out_of_order: BTreeSet<u32>,
    /// Sequence numbers currently believed lost (rules 4, 14, 21).
    loss_list: BTreeSet<u32>,
    /// The highest sequence number ever received, to detect newly-opened
    /// gaps (bookkeeping, not itself a spec-named field).
    highest_received: Option<u32>,
    /// Packets received since the last ACK of either kind (rule 12).
    packets_since_ack: u32,
    /// Absolute time the last Full ACK was sent (rule 11).
    last_full_ack_at: Duration,
    /// Absolute time the last periodic NAK was sent (rule 22).
    last_nak_at: Duration,
    /// Next Full ACK's Acknowledgement Number ("starting from 1", §3.2.4).
    next_ack_number: u32,
    /// Outstanding Full ACKs awaiting their ACKACK, send-time keyed by
    /// Acknowledgement Number (rules 24, 26-28).
    outstanding_acks: BTreeMap<u32, Duration>,
    rtt: RttEstimator,
}

impl Receiver {
    /// A fresh receiver expecting `initial_seq` first (the peer's ISN),
    /// addressing `dest_socket_id` (the peer's SRT Socket ID, §3).
    pub fn new(dest_socket_id: u32, initial_seq: u32) -> Self {
        Receiver {
            dest_socket_id,
            next_expected: initial_seq,
            out_of_order: BTreeSet::new(),
            loss_list: BTreeSet::new(),
            highest_received: None,
            packets_since_ack: 0,
            last_full_ack_at: Duration::ZERO,
            last_nak_at: Duration::ZERO,
            next_ack_number: 1,
            outstanding_acks: BTreeMap::new(),
            rtt: RttEstimator::new(),
        }
    }

    /// The cumulative ack point — every seq strictly before this has been
    /// delivered in order (rule 8's ACK `n + 1` semantics).
    pub fn ack_point(&self) -> u32 {
        self.next_expected
    }

    /// The current RTT estimate (rules 26-31).
    pub fn rtt(&self) -> Duration {
        self.rtt.rtt()
    }

    /// The current RTTVar estimate.
    pub fn rtt_var(&self) -> Duration {
        self.rtt.rtt_var()
    }

    /// Number of sequence numbers currently believed lost.
    pub fn loss_list_len(&self) -> usize {
        self.loss_list.len()
    }

    /// Process one arriving data packet's sequence number.
    pub fn feed_data(&mut self, seq_number: u32, now: Duration) -> FeedOutcome {
        self.packets_since_ack = self.packets_since_ack.saturating_add(1);

        let mut newly_lost = Vec::new();
        match self.highest_received {
            None => self.highest_received = Some(seq_number),
            Some(highest) if seq::seq_gt(seq_number, highest) => {
                let mut s = seq::seq_next(highest);
                let mut n = 0u32;
                while s != seq_number && n < MAX_GAP_EXPANSION {
                    newly_lost.push(s);
                    self.loss_list.insert(s);
                    s = seq::seq_next(s);
                    n += 1;
                }
                self.highest_received = Some(seq_number);
            }
            _ => {}
        }

        self.loss_list.remove(&seq_number);

        let mut delivered = Vec::new();
        if seq_number == self.next_expected {
            delivered.push(seq_number);
            self.next_expected = seq::seq_next(seq_number);
            while self.out_of_order.remove(&self.next_expected) {
                delivered.push(self.next_expected);
                self.next_expected = seq::seq_next(self.next_expected);
            }
        } else if seq::seq_gt(seq_number, self.next_expected) {
            self.out_of_order.insert(seq_number);
        }
        // seq_number before next_expected: a duplicate of an
        // already-delivered packet (e.g. a redundant retransmission) —
        // nothing to do.

        let nak = if newly_lost.is_empty() {
            None
        } else {
            Some(self.build_nak(&newly_lost, now))
        };

        FeedOutcome { delivered, nak }
    }

    fn build_nak(&self, seqs: &[u32], now: Duration) -> Vec<u8> {
        let entries = coalesce(seqs);
        let raw = build_loss_list(&entries).expect("seq numbers are 31-bit by construction");
        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: duration_to_wire_us(now),
            dest_socket_id: self.dest_socket_id,
            raw_loss_list: &raw,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)
            .expect("buffer sized from serialized_len");
        buf
    }

    /// Advance to absolute time `now` and emit any periodic control packets
    /// now due: a Full ACK every [`super::FULL_ACK_PERIOD`] (rule 11), a
    /// Light ACK once [`super::LIGHT_ACK_THRESHOLD`] packets have arrived
    /// since the last ACK (rule 12), and a periodic NAK once `NAKInterval`
    /// has elapsed *and* the loss list is non-empty (rules 21, 22) — never a
    /// NAK when nothing is believed lost.
    pub fn tick(&mut self, now: Duration) -> Vec<Vec<u8>> {
        let mut out = Vec::new();

        if elapsed(now, self.last_full_ack_at) >= FULL_ACK_PERIOD {
            out.push(self.build_full_ack(now));
            self.last_full_ack_at = now;
            self.packets_since_ack = 0;
        } else if self.packets_since_ack >= LIGHT_ACK_THRESHOLD {
            out.push(self.build_light_ack());
            self.packets_since_ack = 0;
        }

        let interval = nak_interval(self.rtt.rtt(), self.rtt.rtt_var());
        if !self.loss_list.is_empty() && elapsed(now, self.last_nak_at) >= interval {
            let seqs: Vec<u32> = self.loss_list.iter().copied().collect();
            out.push(self.build_nak(&seqs, now));
            self.last_nak_at = now;
        }

        out
    }

    fn build_full_ack(&mut self, now: Duration) -> Vec<u8> {
        let ack_number = self.next_ack_number;
        self.next_ack_number = self.next_ack_number.wrapping_add(1);
        self.outstanding_acks.insert(ack_number, now);
        let pkt = ControlPacket::Ack(AckPacket {
            ack_number,
            timestamp: duration_to_wire_us(now),
            dest_socket_id: self.dest_socket_id,
            cif: AckCif::Full {
                last_ack_seq: self.next_expected,
                rtt_us: self.rtt.rtt_us(),
                rtt_var_us: self.rtt.rtt_var_us(),
                // Bandwidth/rate estimation (§4.7) is out of ARQ scope —
                // not curated in srt-arq.md, so left at 0 rather than
                // fabricated.
                avail_buf_size: 0,
                pkt_recv_rate: 0,
                est_link_capacity: 0,
                recv_rate_bps: 0,
            },
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)
            .expect("buffer sized from serialized_len");
        buf
    }

    fn build_light_ack(&self) -> Vec<u8> {
        // §3.2.4: a Light ACK's Acknowledgement Number "should be set to
        // 0"; it carries no RTT/CIF payload beyond the sequence number
        // (rule 24).
        let pkt = ControlPacket::Ack(AckPacket {
            ack_number: 0,
            timestamp: 0,
            dest_socket_id: self.dest_socket_id,
            cif: AckCif::Light {
                last_ack_seq: self.next_expected,
            },
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf)
            .expect("buffer sized from serialized_len");
        buf
    }

    /// Process an incoming ACKACK: match it against the outstanding Full ACK
    /// it acknowledges and update RTT/RTTVar from the round-trip sample
    /// (rules 26-30). An ACKACK for an unknown/already-matched
    /// Acknowledgement Number is ignored.
    pub fn on_ackack(&mut self, ackack: &AckAckPacket, now: Duration) {
        if let Some(sent_at) = self.outstanding_acks.remove(&ackack.ack_number) {
            self.rtt.update(elapsed(now, sent_at));
        }
    }
}

/// `now - since`, clamped to zero rather than panicking on a non-monotonic
/// `now` (a caller bug, not a protocol condition this module needs to
/// reject).
fn elapsed(now: Duration, since: Duration) -> Duration {
    now.checked_sub(since).unwrap_or(Duration::ZERO)
}

/// Coalesce a run of sequence numbers (already circularly increasing) into
/// [`LossListEntry`] Single/Range entries (Appendix A) — a compact NAK
/// encoding; the coalescing itself is not a spec rule.
fn coalesce(seqs: &[u32]) -> Vec<LossListEntry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < seqs.len() {
        let start = seqs[i];
        let mut end = start;
        let mut j = i + 1;
        while j < seqs.len() && seqs[j] == seq::seq_next(end) {
            end = seqs[j];
            j += 1;
        }
        if start == end {
            out.push(LossListEntry::Single(start));
        } else {
            out.push(LossListEntry::Range(start, end));
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: u32 = 0xBBBB;

    #[test]
    fn in_order_arrivals_deliver_immediately_without_nak() {
        let mut r = Receiver::new(PEER, 0);
        for seq_number in 0..5u32 {
            let outcome = r.feed_data(seq_number, Duration::ZERO);
            assert_eq!(outcome.delivered, alloc::vec![seq_number]);
            assert!(outcome.nak.is_none());
        }
        assert_eq!(r.ack_point(), 5);
        assert_eq!(r.loss_list_len(), 0);
    }

    #[test]
    fn a_gap_triggers_an_immediate_nak_and_stalls_delivery() {
        let mut r = Receiver::new(PEER, 0);
        r.feed_data(0, Duration::ZERO);
        r.feed_data(1, Duration::ZERO);
        let outcome = r.feed_data(3, Duration::ZERO); // seq 2 missing
        assert!(outcome.delivered.is_empty());
        let nak = outcome.nak.expect("gap must trigger an immediate NAK");
        let ControlPacket::Nak(n) = ControlPacket::parse(&nak).unwrap() else {
            panic!("expected NAK");
        };
        let entries: Vec<LossListEntry> = n.entries().map(|e| e.unwrap()).collect();
        assert_eq!(entries, alloc::vec![LossListEntry::Single(2)]);
        assert_eq!(r.ack_point(), 2); // stalled at the gap
        assert_eq!(r.loss_list_len(), 1);

        // Filling the gap unblocks the buffered seq 3 too.
        let fill = r.feed_data(2, Duration::ZERO);
        assert_eq!(fill.delivered, alloc::vec![2, 3]);
        assert!(fill.nak.is_none());
        assert_eq!(r.ack_point(), 4);
        assert_eq!(r.loss_list_len(), 0);
    }

    #[test]
    fn zero_loss_tick_never_emits_a_nak() {
        let mut r = Receiver::new(PEER, 0);
        for seq_number in 0..5u32 {
            r.feed_data(seq_number, Duration::ZERO);
        }
        for ms in 1..200u64 {
            let out = r.tick(Duration::from_millis(ms));
            for bytes in &out {
                assert!(!matches!(
                    ControlPacket::parse(bytes).unwrap(),
                    ControlPacket::Nak(_)
                ));
            }
        }
    }

    #[test]
    fn full_ack_fires_on_the_10ms_timer_and_light_ack_on_the_64_packet_threshold() {
        let mut r = Receiver::new(PEER, 0);
        let out = r.tick(FULL_ACK_PERIOD);
        assert_eq!(out.len(), 1);
        let ControlPacket::Ack(ack) = ControlPacket::parse(&out[0]).unwrap() else {
            panic!("expected ACK");
        };
        assert!(matches!(ack.cif, AckCif::Full { .. }));
        assert_eq!(ack.ack_number, 1);

        // Under the Full ACK period, but past the light-ack packet count.
        for seq_number in 0..LIGHT_ACK_THRESHOLD {
            r.feed_data(seq_number, Duration::ZERO);
        }
        let out = r.tick(FULL_ACK_PERIOD + Duration::from_millis(1));
        // Immediately after a Full ACK, the next tick 1ms later is still
        // under the 10ms period, so a Light ACK fires instead.
        let out2 = r.tick(FULL_ACK_PERIOD + Duration::from_millis(2));
        let light =
            out.into_iter()
                .chain(out2)
                .find_map(|b| match ControlPacket::parse(&b).unwrap() {
                    ControlPacket::Ack(a) if matches!(a.cif, AckCif::Light { .. }) => Some(a),
                    _ => None,
                });
        assert!(
            light.is_some(),
            "expected a Light ACK from the 64-packet threshold"
        );
    }

    #[test]
    fn ackack_updates_rtt_from_the_measured_round_trip() {
        let mut r = Receiver::new(PEER, 0);
        let out = r.tick(FULL_ACK_PERIOD);
        let ControlPacket::Ack(ack) = ControlPacket::parse(&out[0]).unwrap() else {
            panic!("expected ACK");
        };
        let ackack = AckAckPacket {
            ack_number: ack.ack_number,
            timestamp: 0,
            dest_socket_id: PEER,
        };
        let sample = Duration::from_millis(20);
        r.on_ackack(&ackack, FULL_ACK_PERIOD + sample);
        // moved from the 100ms initial value toward the 20ms sample.
        assert!(r.rtt() < Duration::from_millis(100));
        assert!(r.rtt() > sample);
    }
}
