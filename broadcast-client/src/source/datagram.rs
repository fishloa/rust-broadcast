//! Datagram ingest for the UDP and RTP sources: TS alignment checks, the RTP reorder buffer
//! (RFC 3550 5.1 sequence numbers, RFC 3551 payload type 33 for MP2T) and the loss counters.
//!
//! Everything here is a pure function of the datagrams and the caller's `now_us`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::IpAddr;

use broadcast_common::Parse;
use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};
use rtp_packet::RtpPacket;

use url::{Host, Url};

use crate::udp::{DatagramStats, MulticastJoin, UdpConfig, UdpMode};
use crate::{Action, Error};

/// RFC 3551 Table 5: static payload type 33 is MP2T, the only RTP payload this source plays.
pub(crate) const RTP_PAYLOAD_TYPE_MP2T: u8 = 33;

/// `true` when `payload` is a non-empty whole number of TS packets that each start with the sync
/// byte.
pub(crate) fn is_aligned_ts(payload: &[u8]) -> bool {
    !payload.is_empty()
        && payload.len().is_multiple_of(TS_PACKET_SIZE)
        && payload
            .chunks_exact(TS_PACKET_SIZE)
            .all(|p| p[0] == TS_SYNC_BYTE)
}

struct Held {
    arrived_us: u64,
    payload: Vec<u8>,
}

pub(crate) struct Ingest {
    rtp: bool,
    window: usize,
    wait_us: u64,
    ssrc: Option<u32>,
    next_seq: u16,
    next_ext: u64,
    held: BTreeMap<u64, Held>,
    stats: DatagramStats,
    discontinuity: bool,
}

pub(crate) struct Released {
    pub payloads: Vec<Vec<u8>>,
    pub discontinuity: bool,
}

impl Ingest {
    pub(crate) fn new(rtp: bool, config: &UdpConfig) -> Self {
        Self {
            rtp,
            window: config.reorder_packets,
            wait_us: config.reorder_wait_us,
            ssrc: None,
            next_seq: 0,
            next_ext: 0,
            held: BTreeMap::new(),
            stats: DatagramStats::default(),
            discontinuity: false,
        }
    }

    pub(crate) fn stats(&self) -> DatagramStats {
        self.stats
    }

    pub(crate) fn reset(&mut self) {
        self.ssrc = None;
        self.held.clear();
        self.discontinuity = false;
    }

    pub(crate) fn push(&mut self, datagram: &[u8], now_us: u64) -> Released {
        self.stats.datagrams = self.stats.datagrams.saturating_add(1);
        let mut out = Vec::new();
        if self.rtp {
            self.push_rtp(datagram, now_us, &mut out);
        } else if is_aligned_ts(datagram) {
            out.push(datagram.to_vec());
        } else {
            self.stats.malformed = self.stats.malformed.saturating_add(1);
        }
        self.finish(out)
    }

    pub(crate) fn poll(&mut self, now_us: u64) -> Released {
        let mut out = Vec::new();
        while let Some((&ext, oldest)) = self.held.first_key_value() {
            if now_us.saturating_sub(oldest.arrived_us) < self.wait_us {
                break;
            }
            self.skip_to(ext);
            self.drain(&mut out);
        }
        self.finish(out)
    }

    pub(crate) fn next_deadline_us(&self) -> Option<u64> {
        self.held
            .values()
            .map(|h| h.arrived_us)
            .min()
            .map(|t| t.saturating_add(self.wait_us))
    }

    fn finish(&mut self, payloads: Vec<Vec<u8>>) -> Released {
        Released {
            payloads,
            discontinuity: core::mem::take(&mut self.discontinuity),
        }
    }

    fn push_rtp(&mut self, datagram: &[u8], now_us: u64, out: &mut Vec<Vec<u8>>) {
        let Ok(packet) = RtpPacket::parse(datagram) else {
            self.stats.malformed = self.stats.malformed.saturating_add(1);
            return;
        };
        if packet.payload_type != RTP_PAYLOAD_TYPE_MP2T || !is_aligned_ts(packet.payload) {
            self.stats.malformed = self.stats.malformed.saturating_add(1);
            return;
        }
        let seq = packet.sequence_number;
        if self.ssrc != Some(packet.ssrc) {
            if self.ssrc.is_some() {
                self.resync();
            }
            self.ssrc = Some(packet.ssrc);
            self.next_seq = seq;
        }
        let delta = i64::from(seq.wrapping_sub(self.next_seq) as i16);
        let window = i64::try_from(self.window).unwrap_or(i64::MAX);
        if delta < 0 {
            if -delta > window {
                self.resync();
                self.next_seq = seq;
                self.deliver(packet.payload, out);
            } else {
                self.stats.late = self.stats.late.saturating_add(1);
            }
            return;
        }
        if delta == 0 {
            let filled_gap = !self.held.is_empty();
            self.deliver(packet.payload, out);
            if filled_gap {
                self.stats.reordered = self.stats.reordered.saturating_add(1);
            }
            self.drain(out);
            return;
        }
        if delta > window {
            let target = self
                .next_ext
                .saturating_add(u64::try_from(delta).unwrap_or(0));
            self.give_up_all(out);
            self.skip_to(target);
            self.next_seq = seq;
            self.deliver(packet.payload, out);
            return;
        }
        let ext = self
            .next_ext
            .saturating_add(u64::try_from(delta).unwrap_or(0));
        if self.held.contains_key(&ext) {
            self.stats.duplicates = self.stats.duplicates.saturating_add(1);
            return;
        }
        self.held.insert(
            ext,
            Held {
                arrived_us: now_us,
                payload: packet.payload.to_vec(),
            },
        );
    }

    fn deliver(&mut self, payload: &[u8], out: &mut Vec<Vec<u8>>) {
        out.push(payload.to_vec());
        self.next_ext = self.next_ext.saturating_add(1);
        self.next_seq = self.next_seq.wrapping_add(1);
    }

    fn drain(&mut self, out: &mut Vec<Vec<u8>>) {
        while let Some(entry) = self.held.first_entry() {
            if *entry.key() != self.next_ext {
                break;
            }
            let held = entry.remove();
            out.push(held.payload);
            self.next_ext = self.next_ext.saturating_add(1);
            self.next_seq = self.next_seq.wrapping_add(1);
        }
    }

    fn skip_to(&mut self, ext: u64) {
        let gap = ext.saturating_sub(self.next_ext);
        self.stats.lost = self.stats.lost.saturating_add(gap);
        self.next_ext = ext;
        self.next_seq = self
            .next_seq
            .wrapping_add(u16::try_from(gap & u64::from(u16::MAX)).unwrap_or(0));
    }

    fn give_up_all(&mut self, out: &mut Vec<Vec<u8>>) {
        while let Some((&first, _)) = self.held.first_key_value() {
            self.skip_to(first);
            self.drain(out);
        }
    }

    fn resync(&mut self) {
        self.held.clear();
        self.next_ext = 0;
        self.stats.resyncs = self.stats.resyncs.saturating_add(1);
        self.discontinuity = true;
    }
}

pub(crate) struct Datagram {
    host: String,
    port: u16,
    mode: UdpMode,
    multicast: Option<MulticastJoin>,
    recv_buffer_bytes: Option<u32>,
    ingest: Ingest,
}

impl Datagram {
    pub(crate) fn udp(url: &Url, config: &UdpConfig, rtp: bool) -> Result<Self, Error> {
        let ip = match url.host() {
            Some(Host::Ipv4(a)) => IpAddr::V4(a),
            Some(Host::Ipv6(a)) => IpAddr::V6(a),
            Some(Host::Domain(name)) => name
                .parse::<IpAddr>()
                .map_err(|_| Error::InvalidUrl("udp needs an IP address".to_string()))?,
            None => return Err(Error::InvalidUrl("missing host".to_string())),
        };
        let port = match url.port() {
            Some(p) if p != 0 => p,
            _ => return Err(Error::InvalidUrl("missing port".to_string())),
        };
        let multicast = if ip.is_multicast() {
            if let Some(source) = config.source
                && (source.is_ipv4() != ip.is_ipv4() || source.is_multicast())
            {
                return Err(Error::InvalidConfig {
                    field: "udp.source",
                });
            }
            Some(MulticastJoin {
                interface: config.interface,
                source: config.source,
            })
        } else {
            if config.source.is_some() {
                return Err(Error::InvalidConfig {
                    field: "udp.source",
                });
            }
            if config.interface.is_some() {
                return Err(Error::InvalidConfig {
                    field: "udp.interface",
                });
            }
            None
        };
        Ok(Self {
            host: ip.to_string(),
            port,
            mode: UdpMode::Receive,
            multicast,
            recv_buffer_bytes: config.recv_buffer_bytes,
            ingest: Ingest::new(rtp, config),
        })
    }

    pub(crate) fn bind_action(&mut self, conn: u32) -> Action {
        self.ingest.reset();
        Action::UdpBind {
            conn,
            host: self.host.clone(),
            port: self.port,
            mode: self.mode,
            multicast: self.multicast,
            recv_buffer_bytes: self.recv_buffer_bytes,
        }
    }

    pub(crate) fn on_datagram(&mut self, bytes: &[u8], now_us: u64) -> Released {
        self.ingest.push(bytes, now_us)
    }

    pub(crate) fn poll(&mut self, now_us: u64) -> Released {
        self.ingest.poll(now_us)
    }

    pub(crate) fn next_deadline_us(&self) -> Option<u64> {
        self.ingest.next_deadline_us()
    }

    pub(crate) fn stats(&self) -> DatagramStats {
        self.ingest.stats()
    }
}
