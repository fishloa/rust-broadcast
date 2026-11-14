//! UDP/multicast bind for `media-doctor watch`: uses the workspace's one
//! datagram bind, [`dvb_stream::udp::bind_udp`], over
//! [`broadcast_common::net::UdpOptions`] (`SO_RCVBUF`, `SO_REUSEADDR`,
//! `SO_REUSEPORT` where sharing a multicast port needs it, the multicast
//! interface).

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::num::NonZeroU32;

use broadcast_common::net::{MulticastInterface, MulticastJoin, UdpOptions};
use socket2::SockRef;

/// Sharing a multicast port on macOS/BSD needs `SO_REUSEPORT` as well as
/// `SO_REUSEADDR`; on Linux `SO_REUSEPORT` load-balances datagrams across the
/// sockets, which a monitor sharing a port with a receiver must not do.
const SHARE_PORT_NEEDS_REUSE_PORT: bool = cfg!(target_vendor = "apple");

/// The [`UdpOptions`] a `media-doctor watch` bind asks for: `reuse_addr` sets
/// `SO_REUSEADDR` (and `SO_REUSEPORT` on Apple targets, where sharing a
/// multicast port needs it), `recv_buffer` is the `SO_RCVBUF` request, and
/// `interface` is the interface the multicast group is joined on. Defaults
/// match the plain `UdpSocket::bind` media-doctor used before these options
/// existed. Note: on Windows `SO_REUSEADDR` lets another process take over the
/// port, so only enable it deliberately.
#[must_use]
pub fn udp_options(
    recv_buffer: Option<NonZeroU32>,
    reuse_addr: bool,
    interface: Option<MulticastInterface>,
) -> UdpOptions {
    let mut options = UdpOptions::new()
        .with_reuse_address(reuse_addr)
        .with_reuse_port(reuse_addr && SHARE_PORT_NEEDS_REUSE_PORT);
    if let Some(bytes) = recv_buffer {
        options = options.with_recv_buffer_bytes(bytes);
    }
    if let Some(interface) = interface {
        options = options.with_join(join_on(interface));
    }
    options
}

/// The multicast join for an interface: the group itself is the bind
/// address, which [`bind_udp`] joins.
#[must_use]
pub fn join_on(interface: MulticastInterface) -> MulticastJoin {
    MulticastJoin::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED)).with_interface(interface)
}

/// Bind a UDP socket for `addr`; a multicast `addr` binds the wildcard on its
/// port and joins the group on the interface `options` names.
///
/// # Errors
///
/// [`dvb_stream::udp::bind_udp`]'s refusals ([`io::ErrorKind::InvalidInput`]
/// when [`UdpOptions`]'s multicast group is not a multicast address or its
/// interface is of the other family), and any socket, bind or join error;
/// `bind_udp` returns a non-blocking socket (for tokio consumers) and this
/// restores blocking mode for the read-timeout ingest loop.
pub fn bind_udp(addr: SocketAddr, options: &UdpOptions) -> io::Result<UdpSocket> {
    let socket = dvb_stream::udp::bind_udp(addr, options)?;
    socket.set_nonblocking(false)?;
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
    use std::net::{Ipv4Addr, UdpSocket};

    fn loopback0() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    fn group(interface: MulticastInterface) -> UdpOptions {
        udp_options(None, false, Some(interface))
    }

    #[test]
    fn unicast_binds_the_requested_address_on_port_zero() {
        let s = bind_udp(loopback0(), &UdpOptions::default()).unwrap();
        let a = s.local_addr().unwrap();
        assert_eq!(a.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(a.port(), 0, "the kernel-assigned port is reported");
    }

    #[test]
    fn the_defaults_match_a_plain_bind() {
        assert_eq!(udp_options(None, false, None), UdpOptions::default());
        let options = udp_options(NonZeroU32::new(1 << 20), true, None);
        assert_eq!(options.recv_buffer_bytes, NonZeroU32::new(1 << 20));
        assert!(options.reuse_address);
        assert_eq!(
            options.reuse_port, SHARE_PORT_NEEDS_REUSE_PORT,
            "SO_REUSEPORT only where sharing a multicast port needs it"
        );
        assert_eq!(options.join, None);
        assert_eq!(
            group(MulticastInterface::V6Index(1)).join,
            Some(join_on(MulticastInterface::V6Index(1)))
        );
    }

    #[test]
    fn second_bind_to_the_same_port_fails_without_reuse_addr() {
        let first = bind_udp(loopback0(), &UdpOptions::default()).unwrap();
        let addr = first.local_addr().unwrap();
        let err = bind_udp(addr, &UdpOptions::default()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
    }

    #[test]
    fn reuse_addr_allows_a_second_bind_to_the_same_port() {
        // The first bind carries the configured options too: on macOS a
        // REUSEPORT holder refuses a second binder for the live lifetime
        // unless that binder holds the option as well (#2663), which is how
        // a shared multicast port is actually used.
        let cfg = udp_options(None, true, None);
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
            recv_buffer_size(&bind_udp(loopback0(), &UdpOptions::default()).unwrap()).unwrap();
        let want = NonZeroU32::new(4 * 1024 * 1024).unwrap();
        let cfg = UdpOptions::new().with_recv_buffer_bytes(want);
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
        let rx = bind_udp(loopback0(), &UdpOptions::default()).unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(b"\x47ts", rx.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 16];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"\x47ts");
    }

    #[test]
    fn reuse_addr_allows_a_second_bind_on_the_same_multicast_port() {
        let cfg = udp_options(None, true, None);
        let first = bind_udp("239.255.42.42:0".parse().unwrap(), &cfg).unwrap();
        let addr = first.local_addr().unwrap();
        bind_udp(addr, &cfg).unwrap();
    }

    #[test]
    fn interface_kind_must_match_the_group_family() {
        let err = bind_udp(
            "239.255.42.42:0".parse().unwrap(),
            &group(MulticastInterface::V6Index(1)),
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let err = bind_udp(
            "[ff02::1234]:0".parse().unwrap(),
            &group(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// `--udp-interface` text goes through `MulticastInterface::parse`: an
    /// address, an index and a name each reach the join as themselves, and a
    /// malformed address is refused rather than becoming a name.
    #[test]
    fn an_interface_option_reaches_the_join_unchanged() {
        let joined = |text: &str| {
            udp_options(None, false, Some(MulticastInterface::parse(text).unwrap()))
                .join
                .unwrap()
                .interface
        };
        assert_eq!(
            joined("192.168.1.10"),
            MulticastInterface::V4(Ipv4Addr::new(192, 168, 1, 10))
        );
        assert_eq!(joined("3"), MulticastInterface::V6Index(3));
        assert_eq!(joined("eth0"), MulticastInterface::Name("eth0".into()));
        assert_eq!(
            group(MulticastInterface::V6Index(1))
                .join
                .unwrap()
                .interface,
            MulticastInterface::V6Index(1),
            "an IPv6 address is not an interface address"
        );
    }

    #[test]
    fn a_join_without_a_multicast_group_is_refused() {
        let err = bind_udp(loopback0(), &group(MulticastInterface::Any)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// Joining a group on the loopback interface needs a multicast-capable
    /// route that CI containers sometimes lack; skip loudly (the repo's
    /// oracle-test convention) rather than pass silently or flake.
    #[test]
    fn ipv4_multicast_group_is_joined_on_the_requested_interface() {
        let cfg = udp_options(
            None,
            true,
            Some(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
        );
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
