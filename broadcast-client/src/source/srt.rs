//! SRT caller link: drives `srt-runtime`'s caller handshake, ARQ receiver and TSBPD scheduler from
//! datagrams and the caller's `now_us`. The crypto (key wrap, AES-CTR) is `srt-runtime`'s too; this
//! module is only the glue that `srt-runtime`'s tokio adapter has for its own sockets, without
//! tokio, clocks or randomness (the entropy comes from [`crate::srt::SrtEntropySeed`]).
//!
//! SRT: draft-sharabayko-srt-01 (handshake 4.3, TSBPD 4.5, too-late drop 4.6, ARQ 4.8, encryption 6).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::time::Duration;

use broadcast_common::stage::Timestamp;

use sha2::{Digest, Sha256};
use srt_runtime::arq::Receiver as ArqReceiver;
use srt_runtime::arq::seq::{contiguous_runs, seq_diff, seq_in_closed_range, seq_lt};
use srt_runtime::caller::CallerHandshake;
use srt_runtime::crypto::{SALT_LEN, aes_ctr_apply, select_sek};
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig, HandshakeOutput, NegotiatedParams};
use srt_runtime::packet::misc::ShutdownPacket;
use srt_runtime::packet::{
    ControlPacket, DropReqPacket, EncryptionField, EncryptionKeyField, SocketId, SrtPacket,
    SrtTimestamp,
};
use srt_runtime::tsbpd::{TickOutcome, TsbpdScheduler};

use crate::source::datagram::is_aligned_ts;
use crate::srt::{SrtCallerConfig, SrtKeyLength};
use crate::udp::DatagramStats;

const HANDSHAKE_RETRANSMIT_PERIOD_US: u64 = 250_000;
const MAX_SOCKET_ID: u32 = 0x3FFF_FFFF;
const SEQ_NUMBER_MASK: u32 = 0x7FFF_FFFF;
const DRIFT_US: i64 = 0;
const LABEL_SOCKET_ID: &[u8] = b"srt-socket-id";
const LABEL_ISN: &[u8] = b"srt-isn";
const LABEL_SALT: &[u8] = b"srt-salt";
const LABEL_SEK: &[u8] = b"srt-sek";

fn derive(seed: &[u8; 32], label: &[u8], attempt: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(seed);
    h.update(label);
    h.update(attempt.to_be_bytes());
    h.finalize().into()
}

fn first_u32(bytes: &[u8; 32]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub(crate) enum Notice {
    Rejected(&'static str),
    HandshakeTimedOut,
    PeerIdle,
    PeerShutdown,
}

#[derive(Default)]
pub(crate) struct Out {
    pub sends: Vec<Vec<u8>>,
    pub payloads: Vec<Vec<u8>>,
    pub notices: Vec<Notice>,
}

struct Handshake {
    hs: CallerHandshake,
    started_us: u64,
    next_tick_us: Option<Timestamp>,
    last_tick_us: u64,
    tick_period_us: u64,
}

struct Link {
    peer_socket_id: u32,
    epoch_us: u64,
    receiver: ArqReceiver,
    tsbpd: TsbpdScheduler,
    staged: BTreeMap<u32, Vec<u8>>,
    key: Option<([u8; SALT_LEN], Vec<u8>)>,
    last_rx_us: u64,
    control: VecDeque<Vec<u8>>,
}

enum State {
    Idle,
    Handshaking(Box<Handshake>),
    Connected(Box<Link>),
    Dead,
}

pub(crate) struct SrtCaller {
    config: SrtCallerConfig,
    attempt: u64,
    state: State,
    stats: DatagramStats,
    out: Out,
}

impl SrtCaller {
    pub(crate) fn new(config: &SrtCallerConfig) -> Result<Self, crate::Error> {
        if config.entropy_seed.is_none() {
            return Err(crate::Error::InvalidConfig(
                crate::error::ConfigField::SrtEntropySeed,
            ));
        }
        Ok(Self {
            config: config.clone(),
            attempt: 0,
            state: State::Idle,
            stats: DatagramStats::default(),
            out: Out::default(),
        })
    }

    pub(crate) fn stats(&self) -> DatagramStats {
        self.stats
    }

    pub(crate) fn reset(&mut self) {
        self.attempt = self.attempt.saturating_add(1);
        self.state = State::Idle;
    }

    pub(crate) fn take_out(&mut self) -> Out {
        core::mem::take(&mut self.out)
    }

    fn seed(&self) -> [u8; 32] {
        self.config
            .entropy_seed
            .as_ref()
            .map_or([0; 32], |s| *s.bytes())
    }

    pub(crate) fn on_connected(&mut self, now_us: u64) {
        if !matches!(self.state, State::Idle) {
            return;
        }
        let seed = self.seed();
        let attempt = self.attempt;
        let own_socket_id = first_u32(&derive(&seed, LABEL_SOCKET_ID, attempt)) % MAX_SOCKET_ID + 1;
        let isn = first_u32(&derive(&seed, LABEL_ISN, attempt)) & SEQ_NUMBER_MASK;
        let mut hs_config = HandshakeConfig::default()
            .with_latency(self.config.latency)
            .with_initial_seq_number(isn)
            .with_stream_id(self.config.stream_id.clone());
        if let Some(enc) = &self.config.encryption {
            hs_config.encryption_field = match enc.key_length {
                SrtKeyLength::Aes128 => EncryptionField::Aes128,
                SrtKeyLength::Aes192 => EncryptionField::Aes192,
                SrtKeyLength::Aes256 => EncryptionField::Aes256,
            };
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&derive(&seed, LABEL_SALT, attempt)[..SALT_LEN]);
            let sek = derive(&seed, LABEL_SEK, attempt)[..enc.key_length.bytes()].to_vec();
            hs_config.crypto = Some(CryptoConfig::new(enc.passphrase().to_vec(), salt, sek));
        }
        let tick_period_us = HANDSHAKE_RETRANSMIT_PERIOD_US;
        let mut hs = CallerHandshake::new(own_socket_id, hs_config);
        match hs.start() {
            Ok(induction) => {
                self.out.sends.push(induction);
                self.state = State::Handshaking(Box::new(Handshake {
                    hs,
                    started_us: now_us,
                    next_tick_us: Timestamp::from_micros(now_us)
                        .checked_add(Duration::from_micros(tick_period_us)),
                    last_tick_us: now_us,
                    tick_period_us,
                }));
            }
            Err(_) => {
                self.state = State::Dead;
                self.out.notices.push(Notice::HandshakeTimedOut);
            }
        }
    }

    pub(crate) fn on_datagram(&mut self, bytes: &[u8], now_us: u64) {
        self.stats.datagrams = self.stats.datagrams.saturating_add(1);
        match &mut self.state {
            State::Handshaking(_) => self.handshake_datagram(bytes, now_us),
            State::Connected(_) => self.link_datagram(bytes, now_us),
            State::Idle | State::Dead => {}
        }
    }

    fn handshake_datagram(&mut self, bytes: &[u8], now_us: u64) {
        let State::Handshaking(h) = &mut self.state else {
            return;
        };
        let Ok(outputs) = h.hs.feed_bytes(bytes) else {
            return;
        };
        for output in outputs {
            match output {
                HandshakeOutput::Send(b) => self.out.sends.push(b),
                HandshakeOutput::Connected(params) => {
                    let Ok(SrtPacket::Control(ControlPacket::Handshake(hp))) =
                        SrtPacket::parse(bytes)
                    else {
                        self.fail_handshake();
                        return;
                    };
                    self.connected(&params, hp.initial_seq_number, hp.timestamp.value(), now_us);
                    return;
                }
                HandshakeOutput::Rejected(reason) => {
                    self.state = State::Dead;
                    self.out.notices.push(Notice::Rejected(reason.name()));
                    return;
                }
                HandshakeOutput::TimedOut => {
                    self.fail_handshake();
                    return;
                }
                _ => {}
            }
        }
    }

    fn fail_handshake(&mut self) {
        self.state = State::Dead;
        self.out.notices.push(Notice::HandshakeTimedOut);
    }

    fn connected(
        &mut self,
        params: &NegotiatedParams,
        peer_isn: u32,
        peer_ts_us: u32,
        now_us: u64,
    ) {
        let key = match (&params.sek, params.salt) {
            (Some(sek), Some(salt)) => Some((salt, sek.to_vec())),
            _ => None,
        };
        let link = Link {
            peer_socket_id: params.peer_socket_id,
            epoch_us: now_us,
            receiver: ArqReceiver::new(
                params.peer_socket_id,
                peer_isn,
                params.max_flow_window_size,
            )
            .with_mtu(params.mtu),
            tsbpd: TsbpdScheduler::new(
                peer_isn,
                -i64::from(peer_ts_us),
                params.latency,
                DRIFT_US,
                params.flags.tlpktdrop(),
                None,
            ),
            staged: BTreeMap::new(),
            key,
            last_rx_us: now_us,
            control: VecDeque::new(),
        };
        self.state = State::Connected(Box::new(link));
    }

    fn link_datagram(&mut self, bytes: &[u8], now_us: u64) {
        let State::Connected(link) = &mut self.state else {
            return;
        };
        link.last_rx_us = now_us;
        let now = Timestamp::from_micros(now_us.saturating_sub(link.epoch_us));
        let Ok(packet) = SrtPacket::parse(bytes) else {
            self.stats.malformed = self.stats.malformed.saturating_add(1);
            return;
        };
        match packet {
            SrtPacket::Data(d) => {
                let released = link.ingest_data(&d, now, &mut self.stats);
                link.release(&released, &mut self.stats, &mut self.out);
            }
            SrtPacket::Control(ControlPacket::AckAck(a)) => link.receiver.on_ackack(&a, now),
            SrtPacket::Control(ControlPacket::DropReq(d)) => {
                let released = link.drop_request(&d, now);
                link.release(&released, &mut self.stats, &mut self.out);
            }
            SrtPacket::Control(ControlPacket::Shutdown(_)) => {
                self.state = State::Dead;
                self.out.notices.push(Notice::PeerShutdown);
                return;
            }
            _ => {}
        }
        self.flush_link_control();
    }

    fn flush_link_control(&mut self) {
        if let State::Connected(link) = &mut self.state {
            while let Some(c) = link.control.pop_front() {
                self.out.sends.push(c);
            }
        }
    }

    pub(crate) fn poll(&mut self, now_us: u64) {
        match &mut self.state {
            State::Handshaking(h) => {
                let budget_us = micros(self.config.timeouts.handshake);
                if now_us.saturating_sub(h.started_us) >= budget_us {
                    self.fail_handshake();
                    return;
                }
                if h.next_tick_us
                    .is_some_and(|next| Timestamp::from_micros(now_us) >= next)
                {
                    // A late wake is one retransmit, not one per missed period (#3847).
                    let elapsed_us = now_us.saturating_sub(h.last_tick_us).max(1);
                    h.last_tick_us = now_us;
                    h.next_tick_us = Timestamp::from_micros(now_us)
                        .checked_add(Duration::from_micros(h.tick_period_us.max(1)));
                    for output in h.hs.tick(Duration::from_micros(elapsed_us)) {
                        match output {
                            HandshakeOutput::Send(b) => self.out.sends.push(b),
                            HandshakeOutput::TimedOut => {
                                self.fail_handshake();
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
            State::Connected(link) => {
                let idle_us = micros(self.config.timeouts.read_idle);
                if now_us.saturating_sub(link.last_rx_us) >= idle_us {
                    self.state = State::Dead;
                    self.out.notices.push(Notice::PeerIdle);
                    return;
                }
                let now = Timestamp::from_micros(now_us.saturating_sub(link.epoch_us));
                link.tick(now);
                let released = link.tsbpd.tick(now);
                link.release(&released, &mut self.stats, &mut self.out);
                self.flush_link_control();
            }
            State::Idle | State::Dead => {}
        }
    }

    pub(crate) fn next_deadline(&self, now_us: u64) -> Option<Timestamp> {
        match &self.state {
            State::Handshaking(h) => {
                // A budget past the end of the clock can never be reached: only the next
                // handshake tick is a deadline then.
                let budget = Timestamp::from_micros(h.started_us)
                    .checked_add(self.config.timeouts.handshake);
                match (budget, h.next_tick_us) {
                    (Some(budget), Some(tick)) => Some(budget.min(tick)),
                    (budget, tick) => budget.or(tick),
                }
            }
            State::Connected(link) => {
                let idle = Timestamp::from_micros(link.last_rx_us)
                    .checked_add(self.config.timeouts.read_idle);
                let ack = link.receiver.next_timeout().and_then(|t| {
                    Timestamp::from_micros(link.epoch_us)
                        .checked_add(Duration::from_micros(t.as_micros()))
                });
                // A deadline past the end of the clock is no deadline; the others still bind.
                let mut at = match (idle, ack) {
                    (Some(idle), Some(ack)) => Some(idle.min(ack)),
                    (Some(one), None) | (None, Some(one)) => Some(one),
                    (None, None) => None,
                };
                let now = Timestamp::from_micros(now_us.saturating_sub(link.epoch_us));
                if let Some(t) = link.tsbpd.next_release_after(now) {
                    // A scheduled release past the end of the clock is no release: no deadline.
                    let release = Timestamp::from_micros(link.epoch_us).checked_add(t);
                    at = match (at, release) {
                        (Some(at), Some(release)) => Some(at.min(release)),
                        (None, Some(release)) => Some(release),
                        (None, None) => None,
                        (Some(_), None) => None,
                    };
                }
                at
            }
            State::Idle | State::Dead => None,
        }
    }

    pub(crate) fn shutdown_datagram(&mut self, now_us: u64) -> Option<Vec<u8>> {
        let State::Connected(link) = &self.state else {
            return None;
        };
        let pkt = ControlPacket::Shutdown(ShutdownPacket {
            timestamp: SrtTimestamp::from_wire(wire_timestamp(
                now_us.saturating_sub(link.epoch_us),
            )),
            dest_socket_id: SocketId::from_wire(link.peer_socket_id),
            libsrt_pad: true,
        });
        let mut buf = alloc::vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).ok()?;
        Some(buf)
    }
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

fn wire_timestamp(us: u64) -> u32 {
    u32::try_from(us & u64::from(u32::MAX)).unwrap_or(0)
}

impl Link {
    fn ingest_data(
        &mut self,
        d: &srt_runtime::packet::DataPacket<'_>,
        now: Timestamp,
        stats: &mut DatagramStats,
    ) -> TickOutcome {
        let mut payload = d.data.to_vec();
        match (&self.key, d.key_flag != EncryptionKeyField::NotEncrypted) {
            (None, false) => {}
            (Some((salt, sek)), true) => {
                let Ok(key) = select_sek(d.key_flag, Some(sek.as_slice()), None) else {
                    stats.undecryptable = stats.undecryptable.saturating_add(1);
                    return TickOutcome::default();
                };
                if aes_ctr_apply(key, salt, d.seq_number, &mut payload).is_err() {
                    stats.undecryptable = stats.undecryptable.saturating_add(1);
                    return TickOutcome::default();
                }
            }
            _ => {
                stats.undecryptable = stats.undecryptable.saturating_add(1);
                return TickOutcome::default();
            }
        }
        if seq_lt(d.seq_number, self.tsbpd.next_release()) {
            stats.late = stats.late.saturating_add(1);
            return TickOutcome::default();
        }
        let outcome = self.receiver.feed_data(d.seq_number, now);
        if outcome.out_of_window {
            return TickOutcome::default();
        }
        if let Some(nak) = outcome.nak {
            self.control.push_back(nak);
        }
        if self.staged.contains_key(&d.seq_number) {
            stats.duplicates = stats.duplicates.saturating_add(1);
        }
        self.staged.entry(d.seq_number).or_insert(payload);
        self.tsbpd.feed_data(d.seq_number, d.timestamp.value(), now)
    }

    fn drop_request(&mut self, d: &DropReqPacket, now: Timestamp) -> TickOutcome {
        let first = d.first_seq.value();
        let last = d.last_seq.value();
        if seq_diff(last, first) < 0 {
            return TickOutcome::default();
        }
        self.receiver.skip_range(first, last);
        self.tsbpd.skip_range(first, last);
        self.staged
            .retain(|&s, _| !seq_in_closed_range(s, first, last));
        self.tsbpd.tick(now)
    }

    fn release(&mut self, outcome: &TickOutcome, stats: &mut DatagramStats, out: &mut Out) {
        for &seq in &outcome.delivered {
            let Some(payload) = self.staged.remove(&seq) else {
                continue;
            };
            if is_aligned_ts(&payload) {
                out.payloads.push(payload);
            } else {
                stats.malformed = stats.malformed.saturating_add(1);
            }
        }
        for &seq in &outcome.dropped {
            self.staged.remove(&seq);
            stats.lost = stats.lost.saturating_add(1);
        }
        for (first, last) in contiguous_runs(&outcome.dropped) {
            self.receiver.skip_range(first, last);
        }
    }

    fn tick(&mut self, now: Timestamp) {
        for bytes in self.receiver.tick(now) {
            self.control.push_back(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srt::SrtEntropySeed;
    use srt_runtime::listener::ListenerHandshake;

    fn first_handshake(seed: [u8; 32]) -> (u32, u32) {
        let config = SrtCallerConfig {
            entropy_seed: Some(SrtEntropySeed::new(seed)),
            ..SrtCallerConfig::default()
        };
        let mut caller = SrtCaller::new(&config).expect("caller");
        caller.on_connected(0);
        let out = caller.take_out();
        let first = out.sends.first().expect("the induction handshake");
        match SrtPacket::parse(first) {
            Ok(SrtPacket::Control(ControlPacket::Handshake(hs))) => {
                (hs.srt_socket_id, hs.initial_seq_number)
            }
            _ => panic!("the first datagram is an induction handshake"),
        }
    }

    #[test]
    fn the_caller_socket_id_and_initial_sequence_number_are_in_range() {
        for byte in 0u16..=255 {
            let seed = [u8::try_from(byte).unwrap(); 32];
            let (socket_id, isn) = first_handshake(seed);
            assert!(
                (1..=MAX_SOCKET_ID).contains(&socket_id),
                "socket id {socket_id} for seed byte {byte}"
            );
            assert!(
                isn <= SEQ_NUMBER_MASK,
                "initial sequence number {isn} for seed byte {byte}"
            );
        }
    }

    #[test]
    fn the_socket_id_and_initial_sequence_number_derive_from_the_seed() {
        for (seed, socket_id, isn) in [
            ([0u8; 32], 99_515_047u32, 1_953_798_393u32),
            ([0xFF; 32], 354_121_992, 1_124_863_867),
            (
                [
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ],
                371_226_212,
                688_821_898,
            ),
        ] {
            assert_eq!(first_handshake(seed), (socket_id, isn), "seed {seed:?}");
        }
    }

    #[test]
    fn a_control_timestamp_is_the_low_32_bits_of_the_clock() {
        assert_eq!(wire_timestamp(0x1_0000_0005), 5);
        assert_eq!(wire_timestamp(0x00FF_FFFF_FFFF), 0xFFFF_FFFF);
    }

    fn handshake_send(outputs: &[HandshakeOutput]) -> Vec<u8> {
        outputs
            .iter()
            .find_map(|o| match o {
                HandshakeOutput::Send(bytes) => Some(bytes.clone()),
                _ => None,
            })
            .expect("a handshake datagram")
    }

    /// A starved runtime can wake the session once, several retransmit
    /// periods late. That single late poll must count as one retransmit, not
    /// synthesize one tick per nominal period: doing the latter spends the
    /// whole 12-retry budget at once and times the handshake out while the
    /// wall-clock budget is nowhere near spent. The dropped first INDUCTION is
    /// still resent and the connection still completes (issue #3847).
    #[test]
    fn a_late_poll_resends_the_lost_induction_instead_of_timing_out() {
        const STALL_US: u64 = 3_600_000;
        let config = SrtCallerConfig {
            entropy_seed: Some(SrtEntropySeed::new([7u8; 32])),
            timeouts: broadcast_common::timeouts::Timeouts::default()
                .with_handshake(Duration::from_secs(10)),
            ..SrtCallerConfig::default()
        };
        let mut caller = SrtCaller::new(&config).expect("caller");
        caller.on_connected(0);
        // The very first INDUCTION is lost: the listener never receives it.
        let _lost = caller.take_out();

        // One wake, 3.6 s late — fourteen nominal 250 ms periods in one step.
        caller.poll(STALL_US);
        let late = caller.take_out();
        assert!(
            late.notices.is_empty(),
            "one late poll must not exhaust the handshake retry budget"
        );
        assert_eq!(late.sends.len(), 1, "exactly one retransmit, not a burst");
        let induction = late
            .sends
            .into_iter()
            .next()
            .expect("retransmitted induction");

        let mut listener =
            ListenerHandshake::new(0x1234_5678, 0xC0FF_EE00, HandshakeConfig::default());
        listener.set_elapsed(Duration::from_micros(STALL_US));
        let response = handshake_send(
            &listener
                .feed_bytes(&induction)
                .expect("listener induction response"),
        );
        caller.on_datagram(&response, STALL_US);
        let out = caller.take_out();
        assert!(out.notices.is_empty());
        let conclusion = out.sends.into_iter().next().expect("conclusion");

        listener.set_elapsed(Duration::from_micros(STALL_US));
        let outputs = listener
            .feed_bytes(&conclusion)
            .expect("listener conclusion response");
        assert!(
            outputs
                .iter()
                .any(|o| matches!(o, HandshakeOutput::Connected(_))),
            "the listener completes the handshake"
        );
        caller.on_datagram(&handshake_send(&outputs), STALL_US);
        assert!(
            matches!(caller.state, State::Connected(_)),
            "the lost-and-resent induction still connects"
        );
    }

    fn link() -> Link {
        Link {
            peer_socket_id: 1,
            epoch_us: 0,
            receiver: ArqReceiver::new(1, 100, 8192),
            tsbpd: TsbpdScheduler::new(100, 0, Duration::from_millis(120), 0, false, None),
            staged: BTreeMap::new(),
            key: None,
            last_rx_us: 0,
            control: VecDeque::new(),
        }
    }

    #[test]
    fn a_drop_request_removes_the_staged_packets_inside_its_range() {
        let mut link = link();
        for seq in [101u32, 102, 103, 104, 105] {
            link.staged
                .insert(seq, alloc::vec![u8::try_from(seq).unwrap()]);
        }
        let req = DropReqPacket {
            message_number: srt_runtime::packet::MessageNumber::try_new(1).expect("message"),
            timestamp: SrtTimestamp::ZERO,
            dest_socket_id: SocketId::from_wire(1),
            first_seq: srt_runtime::packet::SeqNo::try_new(102).expect("first"),
            last_seq: srt_runtime::packet::SeqNo::try_new(104).expect("last"),
        };
        link.drop_request(&req, Timestamp::from_micros(0));
        let left: Vec<u32> = link.staged.keys().copied().collect();
        assert_eq!(left, vec![101, 105], "the dropped range is removed");
    }
}
