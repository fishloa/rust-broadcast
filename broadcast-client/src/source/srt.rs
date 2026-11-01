//! SRT caller link: drives `srt-runtime`'s caller handshake, ARQ receiver and TSBPD scheduler from
//! datagrams and the caller's `now_us`. The crypto (key wrap, AES-CTR) is `srt-runtime`'s too; this
//! module is only the glue that `srt-runtime`'s tokio adapter has for its own sockets, without
//! tokio, clocks or randomness (the entropy comes from [`crate::srt::SrtEntropySeed`]).
//!
//! SRT: draft-sharabayko-srt-01 (handshake 4.3, TSBPD 4.5, too-late drop 4.6, ARQ 4.8, encryption 6).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::time::Duration;

use sha2::{Digest, Sha256};
use srt_runtime::arq::Receiver as ArqReceiver;
use srt_runtime::arq::seq::{seq_diff, seq_in_closed_range, seq_lt, seq_next};
use srt_runtime::caller::CallerHandshake;
use srt_runtime::crypto::{SALT_LEN, aes_ctr_apply, select_sek};
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig, HandshakeOutput, NegotiatedParams};
use srt_runtime::packet::misc::ShutdownPacket;
use srt_runtime::packet::{
    ControlPacket, DropReqPacket, EncryptionField, EncryptionKeyField, SrtPacket,
};
use srt_runtime::tsbpd::{TickOutcome, TsbpdScheduler};

use crate::source::datagram::is_aligned_ts;
use crate::srt::{SrtConfig, SrtKeyLength};
use crate::udp::DatagramStats;

const MICROS_PER_MILLI: u64 = 1_000;
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
    next_tick_us: u64,
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
    config: SrtConfig,
    attempt: u64,
    state: State,
    stats: DatagramStats,
    out: Out,
}

impl SrtCaller {
    pub(crate) fn new(config: &SrtConfig) -> Result<Self, crate::Error> {
        if config.entropy_seed.is_none() {
            return Err(crate::Error::InvalidConfig {
                field: "srt.entropy_seed",
            });
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
        let mut hs_config = HandshakeConfig::default();
        hs_config.latency_ms = self.config.latency_ms;
        hs_config.initial_seq_number = isn;
        hs_config.stream_id.clone_from(&self.config.stream_id);
        if let Some(enc) = &self.config.encryption {
            hs_config.encryption_field = match enc.key_length {
                SrtKeyLength::Aes128 => EncryptionField::Aes128,
                SrtKeyLength::Aes192 => EncryptionField::Aes192,
                SrtKeyLength::Aes256 => EncryptionField::Aes256,
            };
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&derive(&seed, LABEL_SALT, attempt)[..SALT_LEN]);
            let sek = derive(&seed, LABEL_SEK, attempt)[..enc.key_length.bytes()].to_vec();
            hs_config.crypto = Some(CryptoConfig {
                passphrase: enc.passphrase().to_vec(),
                salt,
                sek,
            });
        }
        let ticks = hs_config.retransmit_after_ticks.max(1);
        let tick_period_us = HANDSHAKE_RETRANSMIT_PERIOD_US / u64::from(ticks);
        let mut hs = CallerHandshake::new(own_socket_id, hs_config);
        match hs.start() {
            Ok(induction) => {
                self.out.sends.push(induction);
                self.state = State::Handshaking(Box::new(Handshake {
                    hs,
                    started_us: now_us,
                    next_tick_us: now_us.saturating_add(tick_period_us),
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
                    self.connected(&params, hp.initial_seq_number, hp.timestamp, now_us);
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
                u64::from(params.latency_ms),
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
        let now = Duration::from_micros(now_us.saturating_sub(link.epoch_us));
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
                let budget_us = self
                    .config
                    .handshake_timeout_ms
                    .saturating_mul(MICROS_PER_MILLI);
                if now_us.saturating_sub(h.started_us) >= budget_us {
                    self.fail_handshake();
                    return;
                }
                while now_us >= h.next_tick_us {
                    h.next_tick_us = h.next_tick_us.saturating_add(h.tick_period_us.max(1));
                    for output in h.hs.tick() {
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
                let idle_us = self
                    .config
                    .peer_idle_timeout_ms
                    .saturating_mul(MICROS_PER_MILLI);
                if now_us.saturating_sub(link.last_rx_us) >= idle_us {
                    self.state = State::Dead;
                    self.out.notices.push(Notice::PeerIdle);
                    return;
                }
                let now = Duration::from_micros(now_us.saturating_sub(link.epoch_us));
                link.tick(now);
                let released = link.tsbpd.tick(now);
                link.release(&released, &mut self.stats, &mut self.out);
                self.flush_link_control();
            }
            State::Idle | State::Dead => {}
        }
    }

    pub(crate) fn next_deadline_us(&self, now_us: u64) -> Option<u64> {
        match &self.state {
            State::Handshaking(h) => {
                let budget = h.started_us.saturating_add(
                    self.config
                        .handshake_timeout_ms
                        .saturating_mul(MICROS_PER_MILLI),
                );
                Some(budget.min(h.next_tick_us))
            }
            State::Connected(link) => {
                let idle = link.last_rx_us.saturating_add(
                    self.config
                        .peer_idle_timeout_ms
                        .saturating_mul(MICROS_PER_MILLI),
                );
                let ack = link
                    .epoch_us
                    .saturating_add(micros(link.receiver.next_timeout()));
                let mut at = idle.min(ack);
                let now = Duration::from_micros(now_us.saturating_sub(link.epoch_us));
                if let Some(t) = link.tsbpd.next_release_after(now) {
                    at = at.min(link.epoch_us.saturating_add(micros(t)));
                }
                Some(at)
            }
            State::Idle | State::Dead => None,
        }
    }

    pub(crate) fn shutdown_datagram(&mut self, now_us: u64) -> Option<Vec<u8>> {
        let State::Connected(link) = &self.state else {
            return None;
        };
        let pkt = ControlPacket::Shutdown(ShutdownPacket {
            timestamp: wire_timestamp(now_us.saturating_sub(link.epoch_us)),
            dest_socket_id: link.peer_socket_id,
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
        now: Duration,
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
        self.tsbpd.feed_data(d.seq_number, d.timestamp, now)
    }

    fn drop_request(&mut self, d: &DropReqPacket, now: Duration) -> TickOutcome {
        let first = d.first_seq & SEQ_NUMBER_MASK;
        let last = d.last_seq & SEQ_NUMBER_MASK;
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

    fn tick(&mut self, now: Duration) {
        for bytes in self.receiver.tick(now) {
            self.control.push_back(bytes);
        }
    }
}

fn contiguous_runs(seqs: &[u32]) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &s in seqs {
        match runs.last_mut() {
            Some((_, last)) if seq_next(*last) == s => *last = s,
            _ => runs.push((s, s)),
        }
    }
    runs
}
