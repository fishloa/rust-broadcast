//! Shared UDP transport helper for the UDP-family ingest sources
//! ([`crate::source::rtp_udp`], [`crate::source::ts_udp`]): bind a socket
//! with the workspace's one socket-option type,
//! [`broadcast_common::net::UdpOptions`], and optionally join a multicast
//! group. The option list itself is applied by `dvb-stream`'s `bind_udp`, the
//! workspace's single UDP bind (issue #2374), so multimux's UDP, RTP-UDP,
//! TS-UDP and broadcast-client's hosts cannot drift apart (issue #663 P3a;
//! SP1.6 made `SO_RCVBUF`/`SO_REUSEADDR` and the multicast interface real
//! options rather than defaults; #2663 made the option type shared).
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

use std::net::{IpAddr, SocketAddr};

use broadcast_common::net::{MulticastJoin, UdpOptions};
use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};

/// The session URL (`scheme://group-or-bind:port`) of a UDP-family route: the multicast group the
/// route's options join when they name one, else the bind address.
pub(crate) fn session_url(
    scheme: &str,
    addr: &str,
    join: Option<&MulticastJoin>,
) -> Result<String> {
    let bind: SocketAddr = addr.parse().map_err(|e| MultimuxError::Connect {
        reason: format!("bad UDP bind address {addr:?}: {e}"),
    })?;
    let host = match join.map(|join| join.group) {
        Some(IpAddr::V6(group)) => format!("[{group}]"),
        Some(group) => group.to_string(),
        None if bind.is_ipv6() => format!("[{}]", bind.ip()),
        None => bind.ip().to_string(),
    };
    Ok(format!("{scheme}://{host}:{}", bind.port()))
}

/// Carries the route's socket options ([`UdpOptions`]) onto the session's UDP configuration, so
/// the session's own validation covers them.
pub(crate) fn apply_socket_options(
    config: &mut broadcast_client::Config,
    options: &UdpOptions,
) -> Result<()> {
    config.udp.options.recv_buffer_bytes = options.recv_buffer_bytes;
    config.udp.options.reuse_address = options.reuse_address;
    config.udp.options.reuse_port = options.reuse_port;
    if let Some(join) = &options.join {
        config.udp.options.join = Some(join.clone());
    }
    Ok(())
}

/// Takes the caller-supplied pre-bound socket out of its slot, tolerating a poisoned lock.
pub(crate) fn take_prebound(slot: &std::sync::Mutex<Option<UdpSocket>>) -> Option<UdpSocket> {
    match slot.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

/// The receive-buffer size requested when a caller does not set one (4 MiB):
/// comfortably above the observed OS defaults (Linux ~208 KiB, macOS ~786 KiB)
/// so a burst between scheduler wakeups is not dropped. Best-effort — the OS
/// may clamp it and the bind still succeeds.
pub const DEFAULT_RECV_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// The `SO_RCVBUF` size the bind will request: the caller's explicit value,
/// or [`DEFAULT_RECV_BUFFER_BYTES`] when none was given. A pure function so the
/// choice of default is unit-testable without a socket (I-B).
pub fn requested_recv_buffer_bytes(recv_buffer_bytes: Option<usize>) -> usize {
    recv_buffer_bytes.unwrap_or(DEFAULT_RECV_BUFFER_BYTES)
}

/// The [`UdpOptions`] a route's bind asks for: the caller's socket options,
/// with the crate's default `SO_RCVBUF` filled in when none was given and the
/// route's `multicast_group` as the join's group (when there is one).
///
/// `addr` is the local bind address: for multicast reception this is
/// typically `0.0.0.0:<port>` (or `[::]:<port>` for IPv6) with `port`
/// matching the group's advertised port; for unicast it is the specific
/// local address/port the sender targets. `multicast_group`, if present, must
/// be a multicast address.
///
/// # Errors
///
/// [`MultimuxError::Connect`] when `addr` or `multicast_group` do not parse, or
/// when [`UdpOptions::validate`] refuses the join.
pub fn bind_options(
    addr: &str,
    multicast_group: Option<&str>,
    options: &UdpOptions,
) -> Result<UdpOptions> {
    let connect_error = |reason: String| MultimuxError::Connect { reason };
    addr.parse::<SocketAddr>()
        .map_err(|e| connect_error(format!("bad UDP bind address {addr:?}: {e}")))?;
    let mut bind = options
        .clone()
        .with_recv_buffer_bytes(requested_recv_buffer_bytes(options.recv_buffer_bytes));
    if let Some(group) = multicast_group {
        let group: IpAddr = group
            .parse()
            .map_err(|e| connect_error(format!("bad multicast group {group:?}: {e}")))?;
        let join = bind
            .join
            .take()
            .unwrap_or_else(|| MulticastJoin::new(group));
        bind = bind.with_join(join.with_group(group));
    }
    bind.validate()
        .map_err(|mismatch| MultimuxError::ConfigInvalid {
            field: "routes.input.multicast_interface",
            reason: mismatch.reason().to_string(),
        })?;
    Ok(bind)
}

/// Binds a UDP socket to `addr` (`host:port`), applying `options` and joining
/// `multicast_group` (if given), through the workspace's one datagram bind
/// (`dvb_stream::udp::bind_udp`).
///
/// See [`bind_options`] for what `addr` and `multicast_group` are.
///
/// # Errors
///
/// [`bind_options`]'s refusals, and any bind or join failure, as
/// [`MultimuxError::Connect`].
pub async fn bind_udp(
    addr: &str,
    multicast_group: Option<&str>,
    options: UdpOptions,
) -> Result<UdpSocket> {
    let bind_addr: SocketAddr = addr.parse().map_err(|e| MultimuxError::Connect {
        reason: format!("bad UDP bind address {addr:?}: {e}"),
    })?;
    let options = bind_options(addr, multicast_group, &options)?;
    let wanted_recv = options.recv_buffer_bytes;
    let std_socket =
        dvb_stream::udp::bind_udp(bind_addr, &options).map_err(|e| MultimuxError::Connect {
            reason: format!("udp bind {addr}: {e}"),
        })?;
    if let Some(requested) = wanted_recv {
        match std_socket.recv_buffer_size() {
            Ok(actual) if actual < requested => {
                tracing::debug!(requested, actual, "SO_RCVBUF clamped by the kernel")
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::net::MulticastInterface;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn binds_ephemeral_loopback_port() {
        let socket = bind_udp("127.0.0.1:0", None, UdpOptions::default())
            .await
            .unwrap();
        assert!(socket.local_addr().unwrap().port() > 0);
    }

    #[tokio::test]
    async fn rejects_unparsable_addr() {
        assert!(matches!(
            bind_udp("not-an-addr", None, UdpOptions::default()).await,
            Err(crate::error::MultimuxError::Connect { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_unparsable_multicast_group() {
        assert!(
            bind_udp("0.0.0.0:0", Some("not-an-ip"), UdpOptions::default())
                .await
                .is_err()
        );
    }

    #[test]
    fn the_bind_options_carry_the_caller_options() {
        let options = UdpOptions::new()
            .with_recv_buffer_bytes(1024)
            .with_reuse_address(true)
            .with_join(
                MulticastJoin::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
                    .with_interface(MulticastInterface::V4(Ipv4Addr::LOCALHOST)),
            );
        let bind = bind_options("127.0.0.1:5004", Some("239.255.42.1"), &options).unwrap();
        assert_eq!(bind.recv_buffer_bytes, Some(1024));
        assert!(bind.reuse_address);
        assert!(!bind.reuse_port);
        assert_eq!(
            bind.join,
            Some(
                MulticastJoin::new(IpAddr::V4(Ipv4Addr::new(239, 255, 42, 1)))
                    .with_interface(MulticastInterface::V4(Ipv4Addr::LOCALHOST))
            )
        );
    }

    #[test]
    fn the_bind_options_default_the_receive_buffer_and_the_interface() {
        let bind =
            bind_options("0.0.0.0:5004", Some("239.255.42.1"), &UdpOptions::default()).unwrap();
        assert_eq!(bind.recv_buffer_bytes, Some(DEFAULT_RECV_BUFFER_BYTES));
        assert_eq!(
            bind.join,
            Some(MulticastJoin::new(IpAddr::V4(Ipv4Addr::new(
                239, 255, 42, 1
            ))))
        );
        assert!(
            bind_options("0.0.0.0:5004", None, &UdpOptions::default())
                .unwrap()
                .join
                .is_none()
        );
    }

    /// A name reaches the host as a name (it is the host that resolves it); an
    /// interface of the group's other family is refused before any socket is
    /// made.
    #[test]
    fn the_bind_options_keep_a_named_interface_and_refuse_a_wrong_family() {
        let named = UdpOptions::new().with_join(
            MulticastJoin::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
                .with_interface(MulticastInterface::Name("eth0".into())),
        );
        let bind = bind_options("0.0.0.0:5004", Some("239.255.42.1"), &named).unwrap();
        assert_eq!(
            bind.join.unwrap().interface,
            MulticastInterface::Name("eth0".into())
        );
        let indexed = UdpOptions::new().with_join(
            MulticastJoin::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
                .with_interface(MulticastInterface::V6Index(3)),
        );
        assert!(matches!(
            bind_options("0.0.0.0:5004", Some("239.255.42.1"), &indexed),
            Err(crate::error::MultimuxError::ConfigInvalid {
                field: "routes.input.multicast_interface",
                ..
            })
        ));
        assert!(
            bind_options("0.0.0.0:5004", Some("ff3e::1"), &named).is_ok(),
            "a name does not pin a family"
        );
    }
}
