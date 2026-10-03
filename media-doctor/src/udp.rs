//! UDP/multicast bind for `media-doctor watch` (de-hand-roll W1-P, SP1.6):
//! `socket2` called directly, with configurable `SO_RCVBUF`, `SO_REUSEADDR`
//! and the multicast interface. No shared wrapper crate (spec §4 SP1.6).

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

use socket2::{Domain, Protocol, SockRef, Socket, Type};

/// Socket options for [`bind_udp`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UdpConfig {
    /// `SO_RCVBUF` request in bytes; `None` leaves the OS default. The kernel
    /// may cap the request (`net.core.rmem_max` on Linux); read the result
    /// back with [`recv_buffer_size`].
    pub recv_buffer: Option<usize>,
    /// `SO_REUSEADDR` (and `SO_REUSEPORT` on Apple targets, where sharing a
    /// multicast port needs it). Default `false`, as the old bind behaved.
    /// Note: on Windows `SO_REUSEADDR` lets another process take over the
    /// port, so only enable it deliberately.
    pub reuse_addr: bool,
    /// Interface for the multicast join (`None` = OS choice).
    pub interface: Option<MulticastInterface>,
}

/// Which interface a multicast group is joined on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MulticastInterface {
    /// An IPv4 interface address, for an IPv4 group.
    V4(Ipv4Addr),
    /// An IPv6 interface index, for an IPv6 group.
    V6Index(u32),
}

/// Bind a UDP socket for `addr`; a multicast `addr` binds the wildcard on its
/// port and joins the group (on [`UdpConfig::interface`] when given).
///
/// # Errors
///
/// Any socket error, and [`io::ErrorKind::InvalidInput`] when
/// [`UdpConfig::interface`] is of the wrong family for the multicast group.
pub fn bind_udp(addr: SocketAddr, config: &UdpConfig) -> io::Result<UdpSocket> {
    let (bind_ip, group) = match addr.ip() {
        IpAddr::V4(ip) if ip.is_multicast() => {
            (IpAddr::V4(Ipv4Addr::UNSPECIFIED), Some(IpAddr::V4(ip)))
        }
        IpAddr::V6(ip) if ip.is_multicast() => {
            (IpAddr::V6(Ipv6Addr::UNSPECIFIED), Some(IpAddr::V6(ip)))
        }
        ip => (ip, None),
    };
    // Reject a mismatched interface before any socket exists.
    match (group, config.interface) {
        (Some(IpAddr::V4(_)), Some(MulticastInterface::V6Index(_)))
        | (Some(IpAddr::V6(_)), Some(MulticastInterface::V4(_))) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multicast interface kind does not match the group's address family",
            ));
        }
        _ => {}
    }
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if config.reuse_addr {
        socket.set_reuse_address(true)?;
        // Sharing a multicast port on macOS/BSD needs SO_REUSEPORT too; on
        // Linux it has load-balancing semantics we do not want.
        #[cfg(target_vendor = "apple")]
        socket.set_reuse_port(true)?;
    }
    if let Some(bytes) = config.recv_buffer {
        socket.set_recv_buffer_size(bytes)?;
    }
    socket.bind(&SocketAddr::new(bind_ip, addr.port()).into())?;
    match group {
        Some(IpAddr::V4(ip)) => {
            let iface = match config.interface {
                Some(MulticastInterface::V4(i)) => i,
                _ => Ipv4Addr::UNSPECIFIED,
            };
            socket.join_multicast_v4(&ip, &iface)?;
        }
        Some(IpAddr::V6(ip)) => {
            let index = match config.interface {
                Some(MulticastInterface::V6Index(i)) => i,
                _ => 0,
            };
            socket.join_multicast_v6(&ip, index)?;
        }
        None => {}
    }
    Ok(socket.into())
}

/// The kernel's actual `SO_RCVBUF` (Linux reports twice the request and caps
/// it at `rmem_max`).
///
/// # Errors
///
/// The `getsockopt` error, if any.
pub fn recv_buffer_size(socket: &UdpSocket) -> io::Result<usize> {
    SockRef::from(socket).recv_buffer_size()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

    fn loopback0() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    #[test]
    fn unicast_binds_the_requested_address_on_port_zero() {
        let s = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        let a = s.local_addr().unwrap();
        assert_eq!(a.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(a.port(), 0, "the kernel-assigned port is reported");
    }

    #[test]
    fn second_bind_to_the_same_port_fails_without_reuse_addr() {
        let first = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        let addr = first.local_addr().unwrap();
        let err = bind_udp(addr, &UdpConfig::default()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn reuse_addr_allows_a_second_bind_to_the_same_port() {
        let cfg = UdpConfig {
            reuse_addr: true,
            ..UdpConfig::default()
        };
        let first = bind_udp(loopback0(), &cfg).unwrap();
        let addr = first.local_addr().unwrap();
        bind_udp(addr, &cfg).expect("SO_REUSEADDR must allow the second bind");
    }

    /// The kernel may cap the request (`rmem_max`) and Linux doubles it, so
    /// assert only what is guaranteed: a large request grows the buffer over
    /// the default (a request that was silently ignored would leave it equal),
    /// and the readback API works.
    #[test]
    fn recv_buffer_request_is_applied_or_capped_but_never_ignored() {
        let default =
            recv_buffer_size(&bind_udp(loopback0(), &UdpConfig::default()).unwrap()).unwrap();
        let want = 4 * 1024 * 1024;
        let cfg = UdpConfig {
            recv_buffer: Some(want),
            ..UdpConfig::default()
        };
        let got = recv_buffer_size(&bind_udp(loopback0(), &cfg).unwrap()).unwrap();
        eprintln!("SO_RCVBUF: default {default}, requested {want}, granted {got}");
        // Strictly greater: with `>=` an implementation that ignored the
        // request (got == default) would pass. Linux caps the request at
        // `rmem_max` but then doubles it, so even a capped grant exceeds the
        // default there; macOS grants the full 4 MiB.
        assert!(
            got > default,
            "the request was ignored: requested {want}, got {got}, default {default}"
        );
    }

    #[test]
    fn datagrams_arrive_on_the_bound_socket() {
        let rx = bind_udp(loopback0(), &UdpConfig::default()).unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(b"\x47ts", rx.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 16];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"\x47ts");
    }

    #[test]
    fn interface_kind_must_match_the_group_family() {
        let cfg = UdpConfig {
            interface: Some(MulticastInterface::V6Index(1)),
            ..UdpConfig::default()
        };
        let err = bind_udp("239.255.42.42:0".parse().unwrap(), &cfg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let cfg = UdpConfig {
            interface: Some(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
            ..UdpConfig::default()
        };
        let err = bind_udp("[ff02::1234]:0".parse().unwrap(), &cfg).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// Joining a group on the loopback interface needs a multicast-capable
    /// route that CI containers sometimes lack; skip loudly (the repo's
    /// oracle-test convention) rather than pass silently or flake.
    #[test]
    fn ipv4_multicast_group_is_joined_on_the_requested_interface() {
        let cfg = UdpConfig {
            reuse_addr: true,
            interface: Some(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
            ..UdpConfig::default()
        };
        let rx = match bind_udp("239.255.42.42:0".parse().unwrap(), &cfg) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIP multicast join: {e} (no multicast route on lo)");
                return;
            }
        };
        let port = rx.local_addr().unwrap().port();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        SockRef::from(&tx)
            .set_multicast_if_v4(&Ipv4Addr::LOCALHOST)
            .unwrap();
        tx.set_multicast_loop_v4(true).unwrap();
        if tx.send_to(b"mc", ("239.255.42.42", port)).is_err() {
            eprintln!("SKIP multicast send");
            return;
        }
        let mut buf = [0u8; 8];
        match rx.recv_from(&mut buf) {
            Ok((n, _)) => assert_eq!(&buf[..n], b"mc"),
            Err(e) => eprintln!("SKIP multicast receive: {e}"),
        }
    }
}
