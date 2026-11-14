//! UDP/multicast input: [`bind_udp`] applies the workspace's one
//! [`UdpOptions`] with `socket2` (`SO_REUSEADDR`/`SO_REUSEPORT`, `SO_RCVBUF`,
//! the multicast join), and `tokio_util::udp::UdpFramed` +
//! [`TsDecoder::datagram`] frames the datagrams.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::pin::Pin;
use std::task::{Context, Poll};

use broadcast_common::net::{JoinMismatch, MulticastJoin, UdpOptions};
use bytes::Bytes;
use futures_core::Stream;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio_util::udp::UdpFramed;

use crate::ResyncStats;
use crate::ts_codec::TsDecoder;

/// Creates, configures, joins and binds one UDP socket for `addr`, returning
/// it non-blocking and ready to hand to a runtime.
///
/// This is the workspace's single datagram bind: every consumer that needs
/// `SO_REUSEADDR` / `SO_REUSEPORT`, a socket receive buffer and a multicast
/// join passes its [`UdpOptions`] here rather than repeating the per-OS option
/// list. `SO_REUSEPORT` is applied only where `socket2` provides it (a Unix
/// that is not solaris, illumos, cygwin, nuttx or wasi); elsewhere the request
/// is ignored rather than mis-set.
///
/// A bind requests the receive buffer but does not fail over it when only that
/// request fails (the kernel at most clamps the size); a multicast join failure
/// is returned.
///
/// # Errors
///
/// [`check_udp`]'s refusals (the bind address is not a multicast address, the
/// group is of the other family than the bind address, the interface is of the
/// other family than the group, or the source is not a usable source); any
/// socket option or bind failure; any multicast-join failure. A source-specific
/// IPv6 join is [`io::ErrorKind::Unsupported`].
pub fn bind_udp(addr: SocketAddr, options: &UdpOptions) -> io::Result<Socket> {
    check_udp(addr, options)?;
    match open(addr, options, options.recv_buffer_bytes) {
        Err(error) if options.recv_buffer_bytes.is_some() => {
            tracing::warn!(
                error = %error,
                requested = options.recv_buffer_bytes,
                "SO_RCVBUF not applied; the socket keeps the OS default"
            );
            open(addr, options, None)
        }
        other => other,
    }
}

/// Rejects options that cannot apply to `addr` before any socket is made:
/// [`io::ErrorKind::InvalidInput`] when the join's group or the bind address is
/// not a multicast address, when the group and the bind address are of
/// different families, or when [`UdpOptions::validate`] refuses the join.
///
/// # Errors
///
/// The two refusals above.
pub fn check_udp(addr: SocketAddr, options: &UdpOptions) -> io::Result<()> {
    if let Some(join) = &options.join
        && join.group.is_ipv4() != addr.is_ipv4()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "multicast group is of the other address family than the bind address",
        ));
    }
    options
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.reason()))
}

fn open(
    addr: SocketAddr,
    options: &UdpOptions,
    recv_buffer: Option<NonZeroU32>,
) -> io::Result<Socket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if options.reuse_address {
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
    if options.reuse_port {
        socket.set_reuse_port(true)?;
    }
    if let Some(n) = recv_buffer {
        socket.set_recv_buffer_size(usize::try_from(n.get()).unwrap_or(usize::MAX))?;
    }
    socket.bind(&SockAddr::from(addr))?;
    if let Some(join) = &options.join {
        join_group(&socket, join)?;
    }
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// Joins `join`'s group, any-source or source-specific. A name or index that
/// does not name an IPv4 interface joins on the OS-chosen one (the previous
/// behaviour for an unspecified interface); [`UdpOptions::validate`] has
/// already refused a source of the other family or a source that is itself a
/// group.
fn join_group(socket: &Socket, join: &MulticastJoin) -> io::Result<()> {
    let interface = &join.interface;
    match (join.group, join.source) {
        (IpAddr::V4(group), None) => socket.join_multicast_v4(&group, &interface.ipv4()),
        (IpAddr::V4(group), Some(IpAddr::V4(source))) => {
            socket.join_ssm_v4(&source, &group, &interface.ipv4())
        }
        (IpAddr::V6(_), Some(_)) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "source-specific IPv6 multicast joins are not supported; set the join's source to none",
        )),
        (IpAddr::V6(group), None) => socket.join_multicast_v6(&group, interface.ipv6_index()),
        (IpAddr::V4(_), Some(IpAddr::V6(_))) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            JoinMismatch::SourceFamily.reason(),
        )),
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
