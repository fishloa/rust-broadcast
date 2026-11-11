//! UDP/multicast input: `socket2` for the bind and join (`SO_REUSEADDR`,
//! `SO_RCVBUF`, multicast interface), `tokio_util::udp::UdpFramed` +
//! [`TsDecoder::datagram`] for framing.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, UdpSocket as StdUdp};
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
        let spec = || SocketSpec {
            bind_addr: SocketAddr::V4(self.bind_addr),
            multicast: Some(MulticastMembership::V4 {
                group: self.group,
                interface: self.interface,
            }),
            reuse_address: self.reuse_address,
            reuse_port: self.reuse_address,
            recv_buffer_size: self.recv_buffer_size,
            context: Some("multicast bind"),
        };
        let spec = spec();
        let socket = match bind_standard(spec) {
            Err(error) if self.recv_buffer_size.is_some() => {
                tracing::warn!(
                    error = %error,
                    requested = self.recv_buffer_size,
                    "SO_RCVBUF not applied; the socket keeps the OS default"
                );
                bind_standard(SocketSpec {
                    recv_buffer_size: None,
                    ..spec
                })?
            }
            other => other?,
        };
        Ok(socket.into())
    }
}

impl SocketSpec {
    /// A plain unicast bind with no options set (see each field for the
    /// default the OS then applies).
    #[must_use]
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            multicast: None,
            reuse_address: false,
            reuse_port: false,
            recv_buffer_size: None,
            context: None,
        }
    }

    /// Join `group` after the bind.
    #[must_use]
    pub fn with_multicast(mut self, group: MulticastMembership) -> Self {
        self.multicast = Some(group);
        self
    }

    /// Enable `SO_REUSEADDR` / `SO_REUSEPORT` before the bind.
    #[must_use]
    pub fn with_reuse(mut self, address: bool, port: bool) -> Self {
        self.reuse_address = address;
        self.reuse_port = port;
        self
    }

    /// Request a socket receive buffer of `bytes`.
    #[must_use]
    pub fn with_recv_buffer_size(mut self, bytes: usize) -> Self {
        self.recv_buffer_size = Some(bytes);
        self
    }

    /// Log a bind failure with `context`.
    #[must_use]
    pub fn with_context(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
    }
}

/// Address family of the IP multicast group a socket joins.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulticastMembership {
    /// An IPv4 group, joined on an IPv4 interface address.
    V4 {
        /// The group to receive.
        group: Ipv4Addr,
        /// The interface the join goes out on.
        interface: Ipv4Addr,
    },
    /// An IPv6 group, joined on an interface index (`0` = the system default).
    V6 {
        /// The group to receive.
        group: Ipv6Addr,
        /// The interface index the join goes out on.
        interface: u32,
    },
}

/// Everything [`bind_standard`] needs to build one datagram socket.
///
/// Every option is a *request*: the kernel may clamp `recv_buffer_size`
/// (Linux doubles it and floors it at `rmem_default`; macOS adds overhead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketSpec {
    /// Local address and port to bind.
    pub bind_addr: SocketAddr,
    /// Group to join after the bind; `None` leaves the socket unicast-only.
    pub multicast: Option<MulticastMembership>,
    /// Set `SO_REUSEADDR` before the bind.
    pub reuse_address: bool,
    /// Set `SO_REUSEPORT` before the bind (ignored where the option does not exist).
    pub reuse_port: bool,
    /// `SO_RCVBUF` to request; `None` keeps the OS default.
    pub recv_buffer_size: Option<usize>,
    /// Log the bind failure at debug level with this context; `None` only
    /// returns it.
    pub context: Option<&'static str>,
}

/// Creates, configures and binds one UDP socket from `spec`, returning it
/// non-blocking and ready to hand to a runtime.
///
/// This is the workspace's single datagram bind: every consumer that needs
/// `SO_REUSEADDR` / `SO_REUSEPORT`, a socket receive buffer and a multicast
/// join gets it here rather than repeating the per-OS option list.
/// `SO_REUSEPORT` is applied only where `socket2` provides it (a Unix that is
/// not solaris, illumos, cygwin, nuttx or wasi); elsewhere the request is
/// ignored rather than mis-set.
///
/// # Errors
///
/// Any socket option, bind or multicast-join failure.
pub fn bind_standard(spec: SocketSpec) -> io::Result<Socket> {
    open_standard(&spec).inspect_err(|error| {
        if let Some(context) = spec.context {
            tracing::debug!(%error, context, "UDP bind failed");
        }
    })
}

fn open_standard(spec: &SocketSpec) -> io::Result<Socket> {
    let socket = Socket::new(
        Domain::for_address(spec.bind_addr),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    if spec.reuse_address {
        socket.set_reuse_address(true)?;
    }
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
    if spec.reuse_port {
        socket.set_reuse_port(true)?;
    }
    if let Some(n) = spec.recv_buffer_size {
        socket.set_recv_buffer_size(n)?;
    }
    socket.bind(&SockAddr::from(spec.bind_addr))?;
    match spec.multicast {
        Some(MulticastMembership::V4 { group, interface }) => {
            socket.join_multicast_v4(&group, &interface)?;
        }
        Some(MulticastMembership::V6 { group, interface }) => {
            socket.join_multicast_v6(&group, interface)?;
        }
        None => {}
    }
    socket.set_nonblocking(true)?;
    Ok(socket)
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
