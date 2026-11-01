//! Datagram-socket vocabulary shared by the UDP, RTP and SRT sources: how the host binds a
//! socket, the multicast join it performs, the reorder settings and the counters a session keeps.

use core::net::IpAddr;

/// UDP and RTP source settings ([`crate::Config::udp`]).
///
/// The host applies the socket-level settings (`SO_RCVBUF`, the multicast interface, the
/// source-specific join) when it executes [`crate::Action::UdpBind`]; the session only decides
/// them.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpConfig {
    /// Local interface address a multicast group is joined on; `None` lets the host pick.
    pub interface: Option<IpAddr>,
    /// Source address for a source-specific multicast join (RFC 4607); `None` joins any-source.
    /// Must be of the same address family as the group.
    pub source: Option<IpAddr>,
    /// Requested `SO_RCVBUF` in bytes; `None` keeps the host default. A live multicast feed of a
    /// few Mbit/s overruns the usual 200 KB default during a decoder hiccup.
    pub recv_buffer_bytes: Option<u32>,
    /// RTP only: most packets the reorder buffer holds while it waits for a missing one.
    pub reorder_packets: usize,
    /// RTP only: longest a held packet waits for the missing one before the gap is counted lost.
    pub reorder_wait_us: u64,
}

/// Default [`UdpConfig::reorder_packets`]: about 190 ms of a 10 Mbit/s stream (7 TS packets per
/// RTP packet), more than the reordering ordinary IP paths produce.
pub const DEFAULT_REORDER_PACKETS: usize = 128;
/// Default [`UdpConfig::reorder_wait_us`]: 100 ms.
pub const DEFAULT_REORDER_WAIT_US: u64 = 100_000;
/// Largest [`UdpConfig::reorder_packets`]: the window must stay well inside the signed half of
/// the 16-bit RTP sequence space so "ahead" and "behind" are never ambiguous.
pub const MAX_REORDER_PACKETS: usize = 8_192;

impl Default for UdpConfig {
    fn default() -> Self {
        Self {
            interface: None,
            source: None,
            recv_buffer_bytes: None,
            reorder_packets: DEFAULT_REORDER_PACKETS,
            reorder_wait_us: DEFAULT_REORDER_WAIT_US,
        }
    }
}

impl UdpConfig {
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let invalid = |field| Err(crate::Error::InvalidConfig { field });
        if self.reorder_packets == 0 || self.reorder_packets > MAX_REORDER_PACKETS {
            return invalid("udp.reorder_packets");
        }
        if self.reorder_wait_us == 0 {
            return invalid("udp.reorder_wait_us");
        }
        if self.recv_buffer_bytes == Some(0) {
            return invalid("udp.recv_buffer_bytes");
        }
        Ok(())
    }
}

/// What the host does with the datagram socket named by [`crate::Action::UdpBind`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpMode {
    /// Bind `host:port` locally and receive datagrams sent to it; join the group when
    /// [`crate::Action::UdpBind::multicast`] is set. The datagram's sender is of no interest.
    Receive,
    /// Open a socket connected to `host:port` (resolve the name first), so that
    /// [`crate::Action::Send`] writes one datagram to that peer and only that peer's datagrams are
    /// received.
    Connect,
}

impl UdpMode {
    /// Short label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Receive => "receive",
            Self::Connect => "connect",
        }
    }
}

broadcast_common::impl_spec_display!(UdpMode);

/// Multicast membership the host establishes for a [`UdpMode::Receive`] socket whose address is
/// a multicast group.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MulticastJoin {
    /// Interface address to join on; for an IPv6 group the host maps it to its interface index.
    /// `None` lets the host pick.
    pub interface: Option<IpAddr>,
    /// Source address of a source-specific join (`IP_ADD_SOURCE_MEMBERSHIP`,
    /// `MCAST_JOIN_SOURCE_GROUP`); `None` is an any-source join.
    pub source: Option<IpAddr>,
}

/// Datagram counters of a UDP, RTP or SRT session ([`crate::Session::datagram_stats`]). They
/// accumulate over the life of the session, reconnects included.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DatagramStats {
    /// Datagrams handed to the session by `on_data`.
    pub datagrams: u64,
    /// Datagrams dropped whole because they were not TS (a length that is not a multiple of 188,
    /// a missing sync byte), not RTP, or not RTP payload type 33.
    pub malformed: u64,
    /// RTP packets, or SRT packets, given up on as lost.
    pub lost: u64,
    /// RTP packets that arrived out of order and were put back in place.
    pub reordered: u64,
    /// Duplicate packets dropped.
    pub duplicates: u64,
    /// Packets that arrived after their slot had been given up on, or behind the delivery point.
    pub late: u64,
    /// SRT packets dropped because they were encrypted under a key the session does not hold.
    pub undecryptable: u64,
    /// Times the sequence space was restarted by a new SSRC or a backward jump beyond the window.
    pub resyncs: u64,
}
