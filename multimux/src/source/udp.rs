//! Shared UDP transport helper for the UDP-family ingest sources
//! ([`crate::source::rtp_udp`], [`crate::source::ts_udp`]): bind a socket
//! with configurable socket options and optionally join a multicast group.
//! Pure socket setup — no protocol parsing — kept out of both sources so
//! there is exactly one bind/join implementation between them (issue #663
//! P3a; SP1.6 moved the bind onto `socket2` so `SO_RCVBUF`/`SO_REUSEADDR`
//! and the multicast interface are real options rather than defaults).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};

/// The socket options a UDP bind applies. Every field is a *request*: the
/// kernel may clamp `SO_RCVBUF` (Linux doubles it and floors at
/// `rmem_default`; macOS adds overhead), so a caller that needs to know what
/// the kernel accepted must read it back (`get_recv_buffer_size`).
#[derive(Debug, Clone, Default)]
pub struct UdpBindOptions {
    /// Requested `SO_RCVBUF` in bytes — a larger receive buffer absorbs
    /// bursts that would otherwise drop datagrams under a scheduling hiccup.
    pub recv_buffer_bytes: Option<usize>,
    /// Set `SO_REUSEADDR` before bind (two listeners on one port, or a
    /// restart while old datagrams are still in flight).
    pub reuse_address: bool,
    /// Interface to join a multicast group on: an IPv4 dotted literal, or an
    /// IPv6 interface *index* (as a decimal string). `None` joins on the
    /// unspecified interface (any).
    pub multicast_interface: Option<String>,
}

/// Binds a UDP socket to `addr` (`host:port`), applying `opts`, and joining
/// `multicast_group` (if given).
///
/// `addr` is the local bind address: for multicast reception this is
/// typically `0.0.0.0:<port>` (or `[::]:<port>` for IPv6) with `port`
/// matching the group's advertised port; for unicast it is the specific
/// local address/port the sender targets. `multicast_group`, if present,
/// must be a multicast address of the same IP family as `addr`'s host part.
pub async fn bind_udp(
    addr: &str,
    multicast_group: Option<&str>,
    opts: UdpBindOptions,
) -> Result<UdpSocket> {
    let bind_addr: SocketAddr = addr.parse().map_err(|e| MultimuxError::Connect {
        reason: format!("bad UDP bind address {addr:?}: {e}"),
    })?;
    let domain = socket2::Domain::for_address(bind_addr);
    let socket = socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
        .map_err(|e| MultimuxError::Connect {
        reason: format!("udp socket: {e}"),
    })?;
    // SO_REUSEADDR before bind (two-listener tests rely on it being a
    // deliberate option, not a default).
    if opts.reuse_address {
        socket
            .set_reuse_address(true)
            .map_err(|e| MultimuxError::Connect {
                reason: format!("SO_REUSEADDR: {e}"),
            })?;
    }
    if let Some(bytes) = opts.recv_buffer_bytes {
        // The OS clamps (Linux doubles, floors at rmem_default); the getter
        // proves application, the value is a request not a guarantee.
        socket
            .set_recv_buffer_size(bytes)
            .map_err(|e| MultimuxError::Connect {
                reason: format!("SO_RCVBUF: {e}"),
            })?;
    }
    let sin = socket2::SockAddr::from(bind_addr);
    socket.bind(&sin).map_err(|e| MultimuxError::Connect {
        reason: format!("udp bind {addr}: {e}"),
    })?;
    socket
        .set_nonblocking(true)
        .map_err(|e| MultimuxError::Connect {
            reason: format!("udp nonblocking: {e}"),
        })?;
    if let Some(group) = multicast_group {
        let group_ip: IpAddr = group.parse().map_err(|e| MultimuxError::Connect {
            reason: format!("bad multicast group {group:?}: {e}"),
        })?;
        match group_ip {
            IpAddr::V4(v4) => {
                let iface: Ipv4Addr = opts
                    .multicast_interface
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("bad multicast interface: {e}"),
                    })?
                    .unwrap_or(Ipv4Addr::UNSPECIFIED);
                let target = socket2::InterfaceIndexOrAddress::Address(iface);
                socket
                    .join_multicast_v4_n(&v4, &target)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("join multicast group {group}: {e}"),
                    })?;
            }
            IpAddr::V6(v6) => {
                let interface: u32 = opts
                    .multicast_interface
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("bad multicast interface index: {e}"),
                    })?
                    .unwrap_or(0);
                socket
                    .join_multicast_v6(&v6, interface)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("join multicast group {group}: {e}"),
                    })?;
            }
        }
    }
    let std_socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_socket).map_err(|e| MultimuxError::Connect {
        reason: format!("udp async: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binds_ephemeral_loopback_port() {
        let socket = bind_udp("127.0.0.1:0", None, UdpBindOptions::default())
            .await
            .unwrap();
        assert!(socket.local_addr().unwrap().port() > 0);
    }

    #[tokio::test]
    async fn rejects_unparsable_addr() {
        assert!(
            bind_udp("not-an-addr", None, UdpBindOptions::default())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn rejects_unparsable_multicast_group() {
        assert!(
            bind_udp("0.0.0.0:0", Some("not-an-ip"), UdpBindOptions::default())
                .await
                .is_err()
        );
    }
}
