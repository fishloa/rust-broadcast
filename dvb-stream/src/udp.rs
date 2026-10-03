//! UDP/multicast input: `socket2` for the bind and join (`SO_REUSEADDR`,
//! `SO_RCVBUF`, multicast interface), `tokio_util::udp::UdpFramed` +
//! [`TsDecoder::datagram`] for framing.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdp};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio_util::udp::UdpFramed;

use crate::ResyncStats;
use crate::ts_codec::TsDecoder;

/// Where to listen and which multicast group to join.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MulticastConfig {
    /// Address and port to listen on (usually `0.0.0.0:PORT`).
    pub bind_addr: SocketAddrV4,
    /// Group to join.
    pub group: Ipv4Addr,
    /// Interface for the join; default = `bind_addr.ip()` (the previous behaviour).
    pub interface: Ipv4Addr,
    /// `SO_RCVBUF`; default `None` = the OS default.
    pub recv_buffer_size: Option<usize>,
    /// `SO_REUSEADDR` (plus `SO_REUSEPORT` on unix); default `false`, as with the
    /// plain `UdpSocket::bind` this replaces (a second binder gets `AddrInUse`).
    pub reuse_address: bool,
}

impl MulticastConfig {
    /// Listen on `bind_addr` and join `group` on the bind address's interface.
    #[must_use]
    pub fn new(bind_addr: SocketAddrV4, group: Ipv4Addr) -> Self {
        Self {
            bind_addr,
            group,
            interface: *bind_addr.ip(),
            recv_buffer_size: None,
            reuse_address: false,
        }
    }

    /// Join the group on `interface` instead of the bind address.
    #[must_use]
    pub fn with_interface(mut self, interface: Ipv4Addr) -> Self {
        self.interface = interface;
        self
    }

    /// Request an `SO_RCVBUF` of `bytes` (the kernel may round or clamp it).
    #[must_use]
    pub fn with_recv_buffer_size(mut self, bytes: usize) -> Self {
        self.recv_buffer_size = Some(bytes);
        self
    }

    /// Enable or disable `SO_REUSEADDR` / `SO_REUSEPORT`.
    #[must_use]
    pub fn with_reuse_address(mut self, on: bool) -> Self {
        self.reuse_address = on;
        self
    }

    /// `socket2` bind + join, returned non-blocking and ready for tokio.
    ///
    /// # Errors
    ///
    /// Any socket option, bind or multicast-join failure.
    pub fn bind(&self) -> io::Result<StdUdp> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        if self.reuse_address {
            socket.set_reuse_address(true)?;
            #[cfg(all(
                unix,
                not(any(
                    target_os = "solaris",
                    target_os = "illumos",
                    target_os = "cygwin",
                    target_os = "nuttx",
                    target_os = "wasi"
                ))
            ))]
            socket.set_reuse_port(true)?;
        }
        if let Some(n) = self.recv_buffer_size {
            socket.set_recv_buffer_size(n)?;
        }
        socket.bind(&SockAddr::from(self.bind_addr))?;
        socket.join_multicast_v4(&self.group, &self.interface)?;
        socket.set_nonblocking(true)?;
        Ok(socket.into())
    }
}

/// `UdpFramed` yields `(packet, source)`; the streams only want the packet.
pub(crate) struct UdpPackets(UdpFramed<TsDecoder, tokio::net::UdpSocket>);

impl UdpPackets {
    pub(crate) fn new(socket: tokio::net::UdpSocket) -> Self {
        Self(UdpFramed::new(socket, TsDecoder::datagram()))
    }

    pub(crate) fn resync_stats(&self) -> ResyncStats {
        self.0.codec().resync_stats()
    }
}

impl Stream for UdpPackets {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0)
            .poll_next(cx)
            .map(|o| o.map(|r| r.map(|(pkt, _src)| pkt)))
    }
}
