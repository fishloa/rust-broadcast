//! Shared UDP transport helper for the UDP-family ingest sources
//! ([`crate::source::rtp_udp`], [`crate::source::ts_udp`]): bind a socket
//! with configurable socket options and optionally join a multicast group.
//! This module only translates multimux's route config into a
//! [`dvb_stream::udp::SocketSpec`]: the socket option list itself lives in
//! `dvb-stream`'s `bind_standard`, the workspace's single UDP bind (issue
//! #2374), so multimux's UDP, RTP-UDP, TS-UDP and broadcast-client's hosts
//! cannot drift apart (issue #663 P3a; SP1.6 made `SO_RCVBUF`/`SO_REUSEADDR`
//! and the multicast interface real options rather than defaults).
//!
//! # Multicast port sharing is per-OS (be honest about it)
//!
//! Two receivers on one multicast group/port need different options per OS:
//! - **Linux**: `SO_REUSEADDR` (set here) suffices for two binds to the same
//!   multicast address+port.
//! - **BSD / macOS**: `SO_REUSEADDR` on a UDP socket behaves like
//!   `SO_REUSEPORT` for multicast, so `reuse_address` is usually enough — but
//!   `reuse_port` is available (and is what the platform documents) when a
//!   caller wants it explicit.
//! - **Windows**: `SO_REUSEADDR` lets a second socket *hijack* the port
//!   (undefined which receives); `SO_REUSEPORT` does not exist there, so
//!   multi-receiver sharing on Windows is genuinely unsupported by this
//!   helper — do not rely on it.
//!
//! `reuse_port` is applied only where `socket2` has the option (Unix, minus
//! solaris/illumos/cygwin/nuttx/wasi); elsewhere it is ignored rather than
//! silently mis-set.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};

/// The receive-buffer size requested when a caller does not set one (4 MiB):
/// comfortably above the observed OS defaults (Linux ~208 KiB, macOS ~786 KiB)
/// so a burst between scheduler wakeups is not dropped. Best-effort — the OS
/// may clamp it and the bind still succeeds.
pub const DEFAULT_RECV_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// The `SO_RCVBUF` size [`bind_udp`] will request: the caller's explicit value,
/// or [`DEFAULT_RECV_BUFFER_BYTES`] when none was given. A pure function so the
/// choice of default is unit-testable without a socket (I-B).
pub fn requested_recv_buffer_bytes(recv_buffer_bytes: Option<usize>) -> usize {
    recv_buffer_bytes.unwrap_or(DEFAULT_RECV_BUFFER_BYTES)
}

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
    /// Set `SO_REUSEPORT` before bind (where the option exists; ignored
    /// elsewhere). See the module doc's per-OS multicast-sharing note.
    /// Defaults to `false`.
    pub reuse_port: bool,
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
    let spec = socket_spec(addr, multicast_group, opts)?;
    let wanted_recv = spec.recv_buffer_size;
    let std_socket =
        dvb_stream::udp::bind_standard(spec).map_err(|e| MultimuxError::Connect {
            reason: format!("udp bind {addr}: {e}"),
        })?;
    if let Some(requested) = wanted_recv {
        match std_socket.recv_buffer_size() {
            Ok(actual) if actual < requested => tracing::debug!(
                requested,
                actual,
                "SO_RCVBUF clamped by the kernel"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(
                requested,
                error = %e,
                "SO_RCVBUF not applied; the socket keeps the OS default"
            ),
        }
    }
    UdpSocket::from_std(std_socket.into()).map_err(|e| MultimuxError::Connect {
        reason: format!("udp async: {e}"),
    })
}

/// The socket options `bind_udp` asks for: the caller's, with the crate's
/// default `SO_RCVBUF` filled in when none was given.
fn socket_spec(
    addr: &str,
    multicast_group: Option<&str>,
    opts: UdpBindOptions,
) -> Result<dvb_stream::udp::SocketSpec> {
    let connect_error = |reason: String| MultimuxError::Connect { reason };
    let bind_addr: SocketAddr = addr
        .parse()
        .map_err(|e| connect_error(format!("bad UDP bind address {addr:?}: {e}")))?;
    let multicast = multicast_group
        .map(|group| membership(group, opts.multicast_interface.as_deref()))
        .transpose()?;
    let mut spec = dvb_stream::udp::SocketSpec::new(bind_addr)
        .with_reuse(opts.reuse_address, opts.reuse_port)
        .with_recv_buffer_size(requested_recv_buffer_bytes(opts.recv_buffer_bytes))
        .with_context("multimux udp bind");
    if let Some(membership) = multicast {
        spec = spec.with_multicast(membership);
    }
    Ok(spec)
}

/// The group membership `bind_udp` joins, parsed out of the configured group
/// and interface strings.
fn membership(
    group: &str,
    interface: Option<&str>,
) -> Result<dvb_stream::udp::MulticastMembership> {
    let connect_error = |reason: String| MultimuxError::Connect { reason };
    let group_ip: IpAddr = group
        .parse()
        .map_err(|e| connect_error(format!("bad multicast group {group:?}: {e}")))?;
    Ok(match group_ip {
        IpAddr::V4(v4) => dvb_stream::udp::MulticastMembership::V4 {
            group: v4,
            interface: parse_interface(interface)?,
        },
        IpAddr::V6(v6) => dvb_stream::udp::MulticastMembership::V6 {
            group: v6,
            interface: interface
                .map(str::parse)
                .transpose()
                .map_err(|e| connect_error(format!("bad multicast interface index: {e}")))?
                .unwrap_or(0),
        },
    })
}

/// The IPv4 interface a join goes out on: the configured dotted literal, or
/// the unspecified address (any interface) when none is configured.
fn parse_interface(interface: Option<&str>) -> Result<Ipv4Addr> {
    interface
        .map(str::parse)
        .transpose()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("bad multicast interface: {e}"),
        })
        .map(|iface| iface.unwrap_or(Ipv4Addr::UNSPECIFIED))
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
        assert!(matches!(
            bind_udp("not-an-addr", None, UdpBindOptions::default()).await,
            Err(crate::error::MultimuxError::Connect { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_unparsable_multicast_group() {
        assert!(
            bind_udp("0.0.0.0:0", Some("not-an-ip"), UdpBindOptions::default())
                .await
                .is_err()
        );
    }

    #[test]
    fn spec_carries_the_caller_options() {
        let spec = socket_spec(
            "127.0.0.1:5004",
            Some("239.255.42.1"),
            UdpBindOptions {
                recv_buffer_bytes: Some(1024),
                reuse_address: true,
                multicast_interface: Some("127.0.0.1".to_string()),
                reuse_port: false,
            },
        )
        .unwrap();
        assert_eq!(
            spec.bind_addr,
            SocketAddr::from(([127, 0, 0, 1], 5004))
        );
        assert_eq!(spec.recv_buffer_size, Some(1024));
        assert!(spec.reuse_address);
        assert!(!spec.reuse_port);
        assert_eq!(
            spec.multicast,
            Some(dvb_stream::udp::MulticastMembership::V4 {
                group: Ipv4Addr::new(239, 255, 42, 1),
                interface: Ipv4Addr::LOCALHOST,
            })
        );
    }

    #[test]
    fn spec_defaults_the_receive_buffer_and_the_interface() {
        let spec = socket_spec("0.0.0.0:5004", Some("239.255.42.1"), UdpBindOptions::default())
            .unwrap();
        assert_eq!(spec.recv_buffer_size, Some(DEFAULT_RECV_BUFFER_BYTES));
        assert_eq!(
            spec.multicast,
            Some(dvb_stream::udp::MulticastMembership::V4 {
                group: Ipv4Addr::new(239, 255, 42, 1),
                interface: Ipv4Addr::UNSPECIFIED,
            })
        );
        assert!(socket_spec("0.0.0.0:5004", None, UdpBindOptions::default())
            .unwrap()
            .multicast
            .is_none());
    }

    #[test]
    fn spec_rejects_a_bad_interface() {
        assert!(socket_spec(
            "0.0.0.0:5004",
            Some("239.255.42.1"),
            UdpBindOptions {
                multicast_interface: Some("eth0".to_string()),
                ..UdpBindOptions::default()
            },
        )
        .is_err());
        assert!(socket_spec(
            "0.0.0.0:5004",
            Some("ff3e::1"),
            UdpBindOptions {
                multicast_interface: Some("eth0".to_string()),
                ..UdpBindOptions::default()
            },
        )
        .is_err());
    }
}
