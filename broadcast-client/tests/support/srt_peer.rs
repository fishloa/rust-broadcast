//! A sans-IO SRT listener peer on virtual time, built from `srt-runtime`'s own listener handshake
//! and packet codecs. It sends TS data (clear or AES-CTR encrypted), answers ACK with ACKACK and
//! retransmits on NAK, so the session's caller can be driven through loss, reorder, duplication,
//! silence and shutdown with a deterministic clock.

use std::collections::BTreeMap;

use srt_runtime::crypto::aes_ctr_apply;
use srt_runtime::handshake_sm::{CryptoConfig, HandshakeConfig, HandshakeOutput};
use srt_runtime::listener::ListenerHandshake;
use srt_runtime::packet::misc::{AckAckPacket, KeepAlivePacket, ShutdownPacket};
use srt_runtime::packet::{
    ControlPacket, DataPacket, DropReqPacket, EncryptionField, EncryptionKeyField, LossListEntry,
    PacketPosition, SrtPacket,
};

pub const PEER_SOCKET_ID: u32 = 0x1234_5678;
pub const PEER_COOKIE: u32 = 0xC00C_1E00;
pub const PEER_ISN: u32 = 1_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cipher {
    Clear,
    Aes128,
    Aes192,
    Aes256,
}

pub struct SrtPeer {
    hs: ListenerHandshake,
    pub connected: bool,
    pub caller_socket_id: u32,
    epoch_us: Option<u64>,
    next_seq: u32,
    sent: BTreeMap<u32, Vec<u8>>,
    key: Option<([u8; 16], Vec<u8>)>,
    pub handshakes_seen: u32,
    pub rejected: Option<String>,
    pub naks_received: u32,
    pub keepalives_received: u32,
    pub acks_received: u32,
    pub retransmits: u32,
    pub isn: u32,
    pub max_acked: Option<u32>,
}

impl SrtPeer {
    pub fn new(cipher: Cipher, passphrase: Option<&str>, latency_ms: u16) -> Self {
        Self::with_isn(cipher, passphrase, latency_ms, PEER_ISN)
    }

    pub fn with_isn(cipher: Cipher, passphrase: Option<&str>, latency_ms: u16, isn: u32) -> Self {
        let mut config = HandshakeConfig::default();
        config.latency_ms = latency_ms;
        config.initial_seq_number = isn;
        config.encryption_field = match cipher {
            Cipher::Clear => EncryptionField::NoEncryption,
            Cipher::Aes128 => EncryptionField::Aes128,
            Cipher::Aes192 => EncryptionField::Aes192,
            Cipher::Aes256 => EncryptionField::Aes256,
        };
        if let Some(p) = passphrase {
            config.crypto = Some(CryptoConfig {
                passphrase: p.as_bytes().to_vec(),
                salt: [0; 16],
                sek: Vec::new(),
            });
        }
        Self {
            hs: ListenerHandshake::new(PEER_SOCKET_ID, PEER_COOKIE, config),
            connected: false,
            caller_socket_id: 0,
            epoch_us: None,
            next_seq: isn,
            sent: BTreeMap::new(),
            key: None,
            handshakes_seen: 0,
            rejected: None,
            naks_received: 0,
            keepalives_received: 0,
            acks_received: 0,
            retransmits: 0,
            isn,
            max_acked: None,
        }
    }

    fn ts(&self, now_us: u64) -> u32 {
        let epoch = self.epoch_us.unwrap_or(now_us);
        u32::try_from(now_us.saturating_sub(epoch) & u64::from(u32::MAX)).unwrap()
    }

    pub fn on_datagram(&mut self, bytes: &[u8], now_us: u64) -> Vec<Vec<u8>> {
        let mut replies = Vec::new();
        match SrtPacket::parse(bytes) {
            Ok(SrtPacket::Control(ControlPacket::Handshake(_))) => {
                self.handshakes_seen += 1;
                let Ok(outputs) = self.hs.feed_bytes(bytes) else {
                    return replies;
                };
                for output in outputs {
                    match output {
                        HandshakeOutput::Send(b) => replies.push(b),
                        HandshakeOutput::Connected(p) => {
                            self.connected = true;
                            self.caller_socket_id = p.peer_socket_id;
                            self.epoch_us.get_or_insert(now_us);
                            if let (Some(sek), Some(salt)) = (&p.sek, p.salt) {
                                let mut s = [0u8; 16];
                                s.copy_from_slice(&salt);
                                self.key = Some((s, sek.to_vec()));
                            }
                        }
                        HandshakeOutput::Rejected(r) => self.rejected = Some(r.name().to_string()),
                        _ => {}
                    }
                }
            }
            Ok(SrtPacket::Control(ControlPacket::Ack(ack))) => {
                self.acks_received += 1;
                let acked = match ack.cif {
                    srt_runtime::packet::AckCif::Full { last_ack_seq, .. }
                    | srt_runtime::packet::AckCif::Small { last_ack_seq, .. }
                    | srt_runtime::packet::AckCif::Light { last_ack_seq } => last_ack_seq,
                    _ => 0,
                };
                if self
                    .max_acked
                    .is_none_or(|m| srt_runtime::arq::seq::seq_lt(m, acked))
                {
                    self.max_acked = Some(acked);
                }
                let pkt = ControlPacket::AckAck(AckAckPacket {
                    ack_number: ack.ack_number,
                    timestamp: self.ts(now_us),
                    dest_socket_id: self.caller_socket_id,
                    libsrt_pad: true,
                });
                replies.push(serialize(&pkt));
            }
            Ok(SrtPacket::Control(ControlPacket::Nak(nak))) => {
                self.naks_received += 1;
                for entry in nak.entries().flatten() {
                    let (first, last) = match entry {
                        LossListEntry::Single(s) => (s, s),
                        LossListEntry::Range(a, b) => (a, b),
                        _ => continue,
                    };
                    let mut s = first;
                    loop {
                        if let Some(d) = self.sent.get(&s) {
                            self.retransmits += 1;
                            replies.push(d.clone());
                        }
                        if s == last {
                            break;
                        }
                        s = (s + 1) & 0x7FFF_FFFF;
                    }
                }
            }
            Ok(SrtPacket::Control(ControlPacket::KeepAlive(_))) => self.keepalives_received += 1,
            _ => {}
        }
        replies
    }

    pub fn connected_peer_socket_id(&self) -> u32 {
        PEER_SOCKET_ID
    }

    pub fn sent_datagram(&self, seq: u32) -> Vec<u8> {
        self.sent[&seq].clone()
    }

    pub fn far_ahead_datagram(&mut self, payload: &[u8], ahead: u32, now_us: u64) -> Vec<u8> {
        let saved = self.next_seq;
        self.next_seq = (saved + ahead) & 0x7FFF_FFFF;
        let d = self.data(payload, now_us);
        self.next_seq = saved;
        self.sent.remove(&((saved + ahead) & 0x7FFF_FFFF));
        d
    }

    pub fn next_seq(&self) -> u32 {
        self.next_seq
    }

    pub fn data(&mut self, payload: &[u8], now_us: u64) -> Vec<u8> {
        self.data_with(payload, now_us, None)
    }

    pub fn data_with(
        &mut self,
        payload: &[u8],
        now_us: u64,
        force_flag: Option<EncryptionKeyField>,
    ) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = (seq + 1) & 0x7FFF_FFFF;
        let mut body = payload.to_vec();
        let mut flag = EncryptionKeyField::NotEncrypted;
        if let Some((salt, sek)) = &self.key {
            aes_ctr_apply(sek, salt, seq, &mut body).unwrap();
            flag = EncryptionKeyField::Even;
        }
        if let Some(f) = force_flag {
            flag = f;
        }
        let pkt = DataPacket {
            seq_number: seq,
            position: PacketPosition::Solo,
            in_order: true,
            key_flag: flag,
            retransmitted: false,
            message_number: seq & 0x03FF_FFFF,
            timestamp: self.ts(now_us),
            dest_socket_id: self.caller_socket_id,
            data: &body,
        };
        let mut buf = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut buf).unwrap();
        self.sent.insert(seq, buf.clone());
        buf
    }

    pub fn drop_request(&self, first: u32, last: u32, now_us: u64) -> Vec<u8> {
        serialize(&ControlPacket::DropReq(DropReqPacket {
            message_number: 0,
            timestamp: self.ts(now_us),
            dest_socket_id: self.caller_socket_id,
            first_seq: first,
            last_seq: last,
        }))
    }

    pub fn keepalive(&self, now_us: u64) -> Vec<u8> {
        serialize(&ControlPacket::KeepAlive(KeepAlivePacket {
            timestamp: self.ts(now_us),
            dest_socket_id: self.caller_socket_id,
            libsrt_pad: true,
        }))
    }

    pub fn shutdown(&self, now_us: u64) -> Vec<u8> {
        serialize(&ControlPacket::Shutdown(ShutdownPacket {
            timestamp: self.ts(now_us),
            dest_socket_id: self.caller_socket_id,
            libsrt_pad: true,
        }))
    }
}

fn serialize(pkt: &ControlPacket<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf).unwrap();
    buf
}
