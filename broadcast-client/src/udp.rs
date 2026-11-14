//! Datagram-socket vocabulary shared by the UDP, RTP and SRT sources: how the host binds a
//! socket, the multicast join it performs, the reorder settings and the counters a session keeps.

use alloc::string::String;

use broadcast_common::net::UdpOptions;

/// UDP and RTP source settings ([`crate::Config::udp`]).
///
/// The host applies the socket-level settings ([`UdpOptions`]: `SO_RCVBUF`, the multicast
/// interface, the source-specific join) when it executes [`crate::Action::UdpBind`]; the session
/// only decides them. A live multicast feed of a few Mbit/s overruns the usual 200 KB receive
/// buffer during a decoder hiccup, so `options.recv_buffer_bytes` is worth setting.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpConfig {
    /// The socket options for the bind, the multicast join included. The session fills in the
    /// join's group from the URL; a join set here is only valid for a multicast URL.
    pub options: UdpOptions,
    /// RTP only: an out-of-band SDP (RFC 4566) describing the streams, which switches an `rtp://`
    /// session from MPEG-2 transport stream (payload type 33) to the H.264 (RFC 6184) and AAC
    /// (RFC 3640) streams the SDP declares, played by RTP payload type. The tracks are announced
    /// before the first packet. Not valid for a `udp://` URL; the reorder settings do not apply to an
    /// SDP session.
    pub sdp: Option<String>,
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
            options: UdpOptions::default(),
            sdp: None,
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
        Ok(())
    }
}

/// What the host does with the datagram socket named by [`crate::Action::UdpBind`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpMode {
    /// Bind `host:port` locally and receive datagrams sent to it; join the group when
    /// the bind's [`broadcast_common::net::UdpOptions::join`] is set. The datagram's sender is of no
    /// interest.
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
