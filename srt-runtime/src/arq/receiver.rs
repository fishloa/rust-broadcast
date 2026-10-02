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
//! # TLPKTDROP fake ACK
//! The receiver advances its ack point past packets the delivery side gave up
//! on only when told to (the crate-internal skip the tokio adapter calls when
//! the TSBPD scheduler reports a drop, rule 13); it never skips a gap on its
//! own, so without that call it keeps NAKing and never acknowledges what it
//! has not received.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::time::Duration;

use crate::packet::nak::build_loss_list;
use crate::packet::{AckAckPacket, AckCif, AckPacket, ControlPacket, LossListEntry, NakPacket};

use super::rtt::RttEstimator;
use super::{FULL_ACK_PERIOD, LIGHT_ACK_THRESHOLD, duration_to_wire_us, nak_interval, seq};

/// The most sequence numbers ahead of the cumulative ack point this receiver
/// will ever track (a bound on `loss_list` + `out_of_order`, which together
/// can never name more sequence numbers than the window spans). The
/// effective bound is the negotiated Maximum Flow Window Size
/// (`draft-sharabayko-srt-01` §3.2.1 — a packet further ahead than the window
/// cannot be buffered by the peer's own flow control either), clamped to this
/// value so a hostile or misconfigured window cannot turn one packet into
/// unbounded work. Not a `specs/rules/srt-arq.md` rule — an implementation
/// bound (about ten times libsrt's default 25 600-packet flow window).
const MAX_TRACKED_WINDOW: u32 = 1 << 18;

/// IPv4 (20 B) + UDP (8 B) header bytes inside one MTU-sized datagram — what
/// the handshake's `MTU` field (§3.2.1, "Maximum Transmission Unit Size")
/// counts but a NAK's Control Information Field cannot use.
const IP_UDP_HEADER_LEN: usize = 28;

/// Encoded size of one loss-list entry: a single sequence number is one
/// 32-bit word, a range is two (Appendix A, Figures 21/22).
const LOSS_ENTRY_SINGLE_LEN: usize = 4;
const LOSS_ENTRY_RANGE_LEN: usize = 8;

/// Default MTU assumed for NAK sizing until [`Receiver::with_mtu`] sets the
/// negotiated value (the handshake's default `MTU`, 1500 B).
const DEFAULT_MTU: u32 = 1500;

/// How many multiples of the round-trip time a Full ACK waits for its ACKACK
/// before the entry is forgotten (the ACKACK was lost, or the peer never
/// sends one). Implementation-defined — `srt-arq.md` rules 26-28 do not say
/// when to give up.
const ACK_RETENTION_RTTS: u32 = 4;

/// Floor on the time an outstanding Full ACK is retained, so a very low RTT
/// estimate cannot expire an ACKACK that is merely queued.
const ACK_RETENTION_FLOOR: Duration = Duration::from_secs(1);

/// Hard cap on outstanding Full ACKs awaiting an ACKACK, however long the
/// retention (at one Full ACK per 10 ms this is ten seconds' worth).
const MAX_OUTSTANDING_ACKS: usize = 1024;

/// Outcome of one [`Receiver::feed_data`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FeedOutcome {
    /// Sequence numbers that became cumulatively in-order-deliverable as a
    /// result of this packet's arrival — includes `seq` itself when it was
    /// the next-expected packet, plus any previously-buffered out-of-order
    /// packets it consequently unblocked.
    pub delivered: Vec<u32>,
    /// An immediate NAK to send, if this packet's arrival revealed a new
    /// gap (`specs/rules/srt-arq.md` rules 4, 14).
    pub nak: Option<Vec<u8>>,
    /// The packet was not a 31-bit sequence number (§3.1), or was further
    /// ahead of the cumulative ack point than the flow window allows, and was
    /// ignored entirely (not recorded, not
    /// NAKed): accepting it would let one crafted datagram inflate the loss
    /// list. The caller must not stage or deliver it.
    pub out_of_window: bool,
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
    /// The Full ACK's "Available Buffer Size" (packets) — the negotiated
    /// Maximum Flow Window Size (§3.2.1), the most this receiver will ever
    /// have staged ahead of its delivery cursor (`Driver::ingress`'s flow-
    /// window overflow guard enforces that cap independently, so this is
    /// never an overstatement). A real libsrt sender's own flow control
    /// treats this field as "room for N more packets": reporting `0` here
    /// (this crate's old default, chosen to avoid fabricating a real
    /// buffer-occupancy estimate) reads as "no room at all" and a real
    /// libsrt Caller then withholds every DATA packet forever, connected but
    /// silent — a real, previously undetected interop stall found live
    /// against `srt-live-transmit`, independent of the caller-side
    /// CONCLUSION `dest_socket_id` bug `libsrt_interop.rs` was originally
    /// written to catch (libsrt interop; no tracked issue number for this
    /// one).
    avail_buf_size: u32,
    /// The MTU the periodic NAK is sized against (`IP_UDP_HEADER_LEN` and the
    /// SRT header come off it, see [`Self::with_mtu`]).
    mtu: u32,
}

impl Receiver {
    /// A fresh receiver expecting `initial_seq` first (the peer's ISN),
    /// addressing `dest_socket_id` (the peer's SRT Socket ID, §3), advertising
    /// `max_flow_window` (§3.2.1) as its Full ACK's Available Buffer Size.
    pub fn new(dest_socket_id: u32, initial_seq: u32, max_flow_window: u32) -> Self {
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
            avail_buf_size: max_flow_window,
            mtu: DEFAULT_MTU,
        }
    }

    /// Size periodic NAK datagrams against `mtu` (the handshake's negotiated
    /// Maximum Transmission Unit Size, §3.2.1) instead of the 1500-byte
    /// default: a loss list too long for one datagram is split across several
    /// NAK packets rather than emitted as one the socket cannot send.
    pub fn with_mtu(mut self, mtu: u32) -> Self {
        self.mtu = mtu;
        self
    }

    /// How far ahead of the ack point a packet may be and still be tracked.
    fn window(&self) -> u32 {
        self.avail_buf_size.min(MAX_TRACKED_WINDOW)
    }

    /// Most NAK Control Information Field bytes one datagram may carry:
    /// the MTU less the IP/UDP and SRT headers, never less than one range
    /// entry (so a pathological MTU still makes progress).
    fn max_nak_cif_len(&self) -> usize {
        usize::try_from(self.mtu)
            .unwrap_or(usize::MAX)
            .saturating_sub(IP_UDP_HEADER_LEN + crate::packet::SRT_HEADER_LEN)
            .max(LOSS_ENTRY_RANGE_LEN)
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
    ///
    /// A packet more than the flow window ahead of the ack point is ignored
    /// ([`FeedOutcome::out_of_window`]): a crafted sequence jump therefore
    /// never grows the loss list or the out-of-order set past the window.
    pub fn feed_data(&mut self, seq_number: u32, now: Duration) -> FeedOutcome {
        // Not a 31-bit sequence number at all (§3.1): never a real packet.
        if seq_number > crate::packet::SEQ_NUMBER_MASK {
            return FeedOutcome {
                out_of_window: true,
                ..FeedOutcome::default()
            };
        }
        if seq::seq_gt(seq_number, self.next_expected)
            && !u32::try_from(seq::seq_diff(seq_number, self.next_expected))
                .is_ok_and(|ahead| ahead <= self.window())
        {
            return FeedOutcome {
                out_of_window: true,
                ..FeedOutcome::default()
            };
        }

        self.packets_since_ack = self.packets_since_ack.saturating_add(1);

        let mut nak_entries = Vec::new();
        // Before any packet has arrived the "highest received" is the one
        // just below the peer's ISN (the ack point), so a first packet
        // ahead of the ISN reveals a gap like any other.
        let highest = self
            .highest_received
            .unwrap_or_else(|| seq::seq_add(self.next_expected, crate::packet::SEQ_NUMBER_MASK));
        if seq::seq_gt(seq_number, highest) {
            // The newly opened gap, `highest + 1 ..= seq_number - 1` —
            // clipped to the ack point (a DROPREQ may have moved it past
            // a `highest` that never saw those packets).
            let mut first_lost = seq::seq_next(highest);
            if seq::seq_lt(first_lost, self.next_expected) {
                first_lost = self.next_expected;
            }
            if first_lost != seq_number && seq::seq_lt(first_lost, seq_number) {
                // `+ (2^31 - 1)` is `- 1` in the 31-bit sequence space.
                let last_lost = seq::seq_add(seq_number, crate::packet::SEQ_NUMBER_MASK);
                let mut s = first_lost;
                loop {
                    self.loss_list.insert(s);
                    if s == last_lost {
                        break;
                    }
                    s = seq::seq_next(s);
                }
                nak_entries.push(if first_lost == last_lost {
                    LossListEntry::Single(first_lost)
                } else {
                    LossListEntry::Range(first_lost, last_lost)
                });
            }
            self.highest_received = Some(seq_number);
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

        let nak = if nak_entries.is_empty() {
            None
        } else {
            self.build_naks(&nak_entries, now).into_iter().next()
        };

        FeedOutcome {
            delivered,
            nak,
            out_of_window: false,
        }
    }

    /// Serialize `entries` into one or more NAK datagrams, none larger than
    /// the MTU allows (a periodic NAK of a bursty-loss list otherwise grows
    /// past one datagram, which the socket refuses with `EMSGSIZE`).
    fn build_naks(&self, entries: &[LossListEntry], now: Duration) -> Vec<Vec<u8>> {
        let budget = self.max_nak_cif_len();
        let mut out = Vec::new();
        let mut chunk: Vec<LossListEntry> = Vec::new();
        let mut used = 0usize;
        for &entry in entries {
            let len = match entry {
                LossListEntry::Range(..) => LOSS_ENTRY_RANGE_LEN,
                _ => LOSS_ENTRY_SINGLE_LEN,
            };
            if used + len > budget && !chunk.is_empty() {
                out.extend(self.nak_datagram(&chunk, now));
                chunk.clear();
                used = 0;
            }
            chunk.push(entry);
            used += len;
        }
        if !chunk.is_empty() {
            out.extend(self.nak_datagram(&chunk, now));
        }
        out
    }

    /// One NAK datagram for `entries`, or `None` if the entries cannot be
    /// encoded (a sequence number wider than 31 bits — unreachable, every
    /// entry derives from a masked sequence number — is dropped rather than
    /// panicking the connection).
    fn nak_datagram(&self, entries: &[LossListEntry], now: Duration) -> Option<Vec<u8>> {
        let raw = build_loss_list(entries).ok()?;
        let pkt = ControlPacket::Nak(NakPacket {
            timestamp: duration_to_wire_us(now),
            dest_socket_id: self.dest_socket_id,
            raw_loss_list: &raw,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).ok()?;
        Some(buf)
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
            out.extend(self.build_full_ack(now));
            self.last_full_ack_at = now;
            self.packets_since_ack = 0;
        } else if self.packets_since_ack >= LIGHT_ACK_THRESHOLD {
            out.extend(self.build_light_ack());
            self.packets_since_ack = 0;
        }

        let interval = nak_interval(self.rtt.rtt(), self.rtt.rtt_var());
        if !self.loss_list.is_empty() && elapsed(now, self.last_nak_at) >= interval {
            // Circular order from the ack point, so a run that straddles the
            // 31-bit wrap coalesces into one range.
            let base = self.next_expected;
            let mut seqs: Vec<u32> = self.loss_list.iter().copied().collect();
            seqs.sort_unstable_by_key(|&s| seq::seq_diff(s, base));
            out.extend(self.build_naks(&coalesce(&seqs), now));
            self.last_nak_at = now;
        }

        self.expire_outstanding_acks(now);

        out
    }

    /// Forget Full ACKs whose ACKACK never came (W3): a lost ACKACK, or a
    /// peer that never sends one, would otherwise leave one entry per Full
    /// ACK (100/s) forever.
    fn expire_outstanding_acks(&mut self, now: Duration) {
        let retention = (self.rtt.rtt() * ACK_RETENTION_RTTS).max(ACK_RETENTION_FLOOR);
        self.outstanding_acks
            .retain(|_, sent_at| elapsed(now, *sent_at) <= retention);
    }

    fn build_full_ack(&mut self, now: Duration) -> Option<Vec<u8>> {
        let ack_number = self.next_ack_number;
        self.next_ack_number = self.next_ack_number.wrapping_add(1);
        if self.outstanding_acks.len() >= MAX_OUTSTANDING_ACKS {
            // Oldest first: Acknowledgement Numbers only ever increase.
            self.outstanding_acks.pop_first();
        }
        self.outstanding_acks.insert(ack_number, now);
        let pkt = ControlPacket::Ack(AckPacket {
            ack_number,
            timestamp: duration_to_wire_us(now),
            dest_socket_id: self.dest_socket_id,
            cif: AckCif::Full {
                last_ack_seq: self.next_expected,
                rtt_us: self.rtt.rtt_us(),
                rtt_var_us: self.rtt.rtt_var_us(),
                avail_buf_size: self.avail_buf_size,
                // Bandwidth/rate estimation (§4.7) is out of ARQ scope — not
                // curated in srt-arq.md, so left at 0 rather than fabricated.
                pkt_recv_rate: 0,
                est_link_capacity: 0,
                recv_rate_bps: 0,
            },
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).ok()?;
        Some(buf)
    }

    fn build_light_ack(&self) -> Option<Vec<u8>> {
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
        pkt.serialize_into(&mut buf).ok()?;
        Some(buf)
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

    /// Handle a peer's DROPREQ (`draft-sharabayko-srt-01` §3.2.9): the sender
    /// has given up on delivering the inclusive sequence range
    /// `first..=last`, so they will never arrive.
    ///
    /// Without this, loss detection would NAK those numbers forever and the
    /// cumulative ack point (`next_expected`) would stall below the gap for
    /// the life of the connection. The range is removed from the loss list
    /// (stop asking) and the out-of-order set (the sender has given up on
    /// them, so anything they unblocked can no longer be delivered in order),
    /// and when the range covers `next_expected` the ack point jumps to the
    /// sequence after `last`. A `last` preceding `first` is a malformed
    /// (empty) range and ignored.
    //
    // The DROPREQ consumer lives in the tokio adapter (`crate::io`); the
    // engine core itself never receives packets, so it is gated accordingly.
    #[cfg(feature = "tokio")]
    pub(crate) fn skip_range(&mut self, first: u32, last: u32) {
        if seq::seq_diff(last, first) < 0 {
            return;
        }
        self.loss_list
            .retain(|s| !seq::seq_in_closed_range(*s, first, last));
        self.out_of_order
            .retain(|s| !seq::seq_in_closed_range(*s, first, last));
        if seq::seq_in_closed_range(self.next_expected, first, last) {
            self.next_expected = seq::seq_next(last);
        }
        // Anything at or beyond the new ack point that was already received
        // is now cumulatively deliverable — drain it exactly like `feed_data`
        // does when a gap is filled.
        while self.out_of_order.remove(&self.next_expected) {
            self.next_expected = seq::seq_next(self.next_expected);
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
    const MAX_FLOW_WINDOW: u32 = 8192;

    #[test]
    fn in_order_arrivals_deliver_immediately_without_nak() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
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
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
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
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
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
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
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
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        let out = r.tick(FULL_ACK_PERIOD);
        let ControlPacket::Ack(ack) = ControlPacket::parse(&out[0]).unwrap() else {
            panic!("expected ACK");
        };
        let ackack = AckAckPacket {
            ack_number: ack.ack_number,
            timestamp: 0,
            dest_socket_id: PEER,
            libsrt_pad: false,
        };
        let sample = Duration::from_millis(20);
        r.on_ackack(&ackack, FULL_ACK_PERIOD + sample);
        // moved from the 100ms initial value toward the 20ms sample.
        assert!(r.rtt() < Duration::from_millis(100));
        assert!(r.rtt() > sample);
    }

    fn nak_entries(bytes: &[u8]) -> Vec<LossListEntry> {
        let ControlPacket::Nak(n) = ControlPacket::parse(bytes).unwrap() else {
            panic!("expected NAK");
        };
        n.entries().map(|e| e.unwrap()).collect()
    }

    /// r08-SRT-W2: a sequence number past the flow window is ignored, so one
    /// crafted datagram cannot inflate the loss list. 2^30 - 1 is the farthest
    /// "ahead" the 31-bit circular order allows (the old code enumerated
    /// 65 536 losses for it and then jumped `highest_received`, so the next
    /// spoofed packet added another 65 536).
    #[test]
    fn a_sequence_number_wider_than_31_bits_is_refused() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        for seq in [0x8000_0000u32, 0x8000_0001, u32::MAX] {
            let out = r.feed_data(seq, Duration::ZERO);
            assert!(out.out_of_window, "{seq:#x}");
            assert!(out.delivered.is_empty() && out.nak.is_none());
        }
        assert_eq!(r.loss_list_len(), 0);
        assert_eq!(r.highest_received, None);
        assert_eq!(r.ack_point(), 0);
    }

    #[test]
    fn a_hostile_sequence_jump_is_ignored_not_enumerated() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        r.feed_data(0, Duration::ZERO);
        for jump in [(1u32 << 30) - 1, 1 << 20, 1 << 18, 5 << 16] {
            let out = r.feed_data(jump, Duration::ZERO);
            assert!(out.out_of_window, "jump to {jump} must be refused");
            assert!(out.nak.is_none());
            assert!(out.delivered.is_empty());
        }
        assert_eq!(r.loss_list_len(), 0);
        assert_eq!(r.out_of_order.len(), 0);
        assert_eq!(r.ack_point(), 1);
        // The ack point is where the window is measured from: repeating the
        // attack does not move `highest_received` either.
        assert_eq!(r.highest_received, Some(0));
    }

    /// The window boundary is exact: `next_expected + window` is accepted
    /// (and its whole gap NAKed as one range), one more is refused.
    #[test]
    fn the_flow_window_boundary_is_exact() {
        const WINDOW: u32 = 1000;
        let mut r = Receiver::new(PEER, 0, WINDOW);
        let beyond = r.feed_data(WINDOW + 1, Duration::ZERO);
        assert!(beyond.out_of_window);
        assert_eq!(r.loss_list_len(), 0);

        let edge = r.feed_data(WINDOW, Duration::ZERO);
        assert!(!edge.out_of_window);
        assert_eq!(r.loss_list_len(), (WINDOW - 1) as usize + 1); // 0..WINDOW-1
        let nak = edge.nak.expect("the gap is NAKed immediately");
        // One Range entry, not 1000 singles.
        assert_eq!(
            nak_entries(&nak),
            alloc::vec![LossListEntry::Range(0, WINDOW - 1)]
        );
        assert!(r.out_of_order.len() <= WINDOW as usize);
    }

    /// A window larger than the receiver is willing to track is clamped, so
    /// a hostile `max_flow_window_size` cannot re-open the flood.
    #[test]
    fn an_oversized_negotiated_window_is_clamped() {
        let mut r = Receiver::new(PEER, 0, u32::MAX);
        let out = r.feed_data((1 << 30) - 1, Duration::ZERO);
        assert!(out.out_of_window);
        assert_eq!(r.loss_list_len(), 0);
    }

    /// r08-SRT-W3: Full ACKs whose ACKACK never arrives are forgotten. 5 000
    /// ticks (50 s) with no ACKACK at all used to leave 5 000 entries.
    #[test]
    fn unanswered_full_acks_age_out() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        for tick in 1..=5_000u64 {
            r.tick(Duration::from_millis(10 * tick));
        }
        // Retention is max(4 x RTT, 1 s) = 1 s of 10 ms ACKs.
        let retention_acks =
            (ACK_RETENTION_FLOOR.as_millis() / FULL_ACK_PERIOD.as_millis()) as usize;
        assert!(
            r.outstanding_acks.len() <= retention_acks + 1,
            "{} outstanding ACKs retained",
            r.outstanding_acks.len()
        );
        assert!(!r.outstanding_acks.is_empty(), "recent ACKs are kept");
    }

    /// An ACKACK arriving inside the retention window still measures RTT; one
    /// that arrives after its entry aged out is ignored.
    #[test]
    fn ackack_after_expiry_is_ignored() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        let out = r.tick(FULL_ACK_PERIOD);
        let ControlPacket::Ack(ack) = ControlPacket::parse(&out[0]).unwrap() else {
            panic!("expected ACK");
        };
        let ackack = AckAckPacket {
            ack_number: ack.ack_number,
            timestamp: 0,
            dest_socket_id: PEER,
            libsrt_pad: false,
        };
        let rtt_before = r.rtt();
        // Age the entry out (no ACKACK for 10 s), then deliver the ACKACK.
        for tick in 2..=1_000u64 {
            r.tick(Duration::from_millis(10 * tick));
        }
        r.on_ackack(&ackack, Duration::from_secs(10));
        assert_eq!(r.rtt(), rtt_before, "an expired ACK must not move the RTT");
    }

    /// The hard cap: more Full ACKs than `MAX_OUTSTANDING_ACKS` inside one
    /// retention window evict the oldest rather than growing.
    #[test]
    fn outstanding_acks_are_capped() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        for _ in 0..(MAX_OUTSTANDING_ACKS * 5) {
            r.build_full_ack(Duration::ZERO);
        }
        assert_eq!(r.outstanding_acks.len(), MAX_OUTSTANDING_ACKS);
        // The survivors are the newest.
        let oldest = *r.outstanding_acks.keys().next().unwrap();
        assert_eq!(oldest as usize, MAX_OUTSTANDING_ACKS * 4 + 1);
    }

    /// r08-SRT-W4: a bursty (non-contiguous) loss list no longer becomes one
    /// datagram larger than the MTU — it is split, every datagram fits, and
    /// together they name every lost packet exactly once.
    #[test]
    fn periodic_nak_is_split_to_fit_the_mtu() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        r.feed_data(0, Duration::ZERO);
        // Receive every other packet: 2, 4, ..., 2000 → 1000 isolated losses.
        let mut lost = Vec::new();
        for q in (2..=2000u32).step_by(2) {
            r.feed_data(q, Duration::ZERO);
            lost.push(q - 1);
        }
        assert_eq!(r.loss_list_len(), 1000);

        let naks: Vec<Vec<u8>> = r
            .tick(Duration::from_secs(1))
            .into_iter()
            .filter(|b| matches!(ControlPacket::parse(b), Ok(ControlPacket::Nak(_))))
            .collect();
        assert!(naks.len() > 1, "1000 singles cannot fit one datagram");
        let mut named = Vec::new();
        for nak in &naks {
            assert!(
                nak.len() + IP_UDP_HEADER_LEN <= DEFAULT_MTU as usize,
                "NAK of {} B exceeds the {} B MTU",
                nak.len() + IP_UDP_HEADER_LEN,
                DEFAULT_MTU
            );
            for e in nak_entries(nak) {
                match e {
                    LossListEntry::Single(s) => named.push(s),
                    other => panic!("unexpected entry {other:?}"),
                }
            }
        }
        named.sort_unstable();
        assert_eq!(named, lost);
    }

    /// The MTU is configurable: 576 gives smaller datagrams.
    #[test]
    fn nak_chunking_follows_the_configured_mtu() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW).with_mtu(576);
        r.feed_data(0, Duration::ZERO);
        for q in (2..=400u32).step_by(2) {
            r.feed_data(q, Duration::ZERO);
        }
        let naks: Vec<Vec<u8>> = r
            .tick(Duration::from_secs(1))
            .into_iter()
            .filter(|b| matches!(ControlPacket::parse(b), Ok(ControlPacket::Nak(_))))
            .collect();
        assert!(naks.len() >= 2);
        for nak in &naks {
            assert!(nak.len() + IP_UDP_HEADER_LEN <= 576);
        }
    }

    /// A loss run that straddles the 31-bit wrap is one range in the periodic
    /// NAK (the loss list is ordered by raw value, which would split it at
    /// the wrap).
    #[test]
    fn periodic_nak_coalesces_a_loss_run_across_the_wrap() {
        const MAX: u32 = crate::packet::SEQ_NUMBER_MASK;
        let mut r = Receiver::new(PEER, MAX - 2, MAX_FLOW_WINDOW);
        r.feed_data(MAX - 2, Duration::ZERO);
        let imm = r.feed_data(2, Duration::ZERO); // MAX-1, MAX, 0, 1 lost
        assert_eq!(
            nak_entries(&imm.nak.expect("gap NAK")),
            alloc::vec![LossListEntry::Range(MAX - 1, 1)]
        );
        let naks: Vec<Vec<u8>> = r
            .tick(Duration::from_secs(1))
            .into_iter()
            .filter(|b| matches!(ControlPacket::parse(b), Ok(ControlPacket::Nak(_))))
            .collect();
        assert_eq!(naks.len(), 1);
        assert_eq!(
            nak_entries(&naks[0]),
            alloc::vec![LossListEntry::Range(MAX - 1, 1)]
        );
    }

    /// After a DROPREQ moves the ack point past packets this receiver never
    /// saw, the next gap is measured from the ack point, not from a stale
    /// `highest_received` (which would NAK sequence numbers the sender gave
    /// up on).
    #[cfg(feature = "tokio")]
    #[test]
    fn gap_after_a_dropreq_starts_at_the_ack_point() {
        let mut r = Receiver::new(PEER, 0, MAX_FLOW_WINDOW);
        r.feed_data(0, Duration::ZERO);
        r.skip_range(1, 99); // sender gave up on 1..=99
        assert_eq!(r.ack_point(), 100);
        let out = r.feed_data(105, Duration::ZERO);
        assert_eq!(
            nak_entries(&out.nak.expect("gap NAK")),
            alloc::vec![LossListEntry::Range(100, 104)]
        );
        assert_eq!(r.loss_list_len(), 5);
    }
}
