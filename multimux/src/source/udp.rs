//! Shared UDP transport helper for the UDP-family ingest sources
//! ([`crate::source::rtp_udp`], [`crate::source::ts_udp`]): bind a socket
//! with configurable socket options and optionally join a multicast group.
//! Pure socket setup — no protocol parsing — kept out of both sources so
//! there is exactly one bind/join implementation between them (issue #663
//! P3a; SP1.6 moved the bind onto `socket2` so `SO_RCVBUF`/`SO_REUSEADDR`
//! and the multicast interface are real options rather than defaults).
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
//! `reuse_port` is applied only on Unix (`cfg(unix)`); on other platforms it
//! is ignored rather than silently mis-set.

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
    /// Set `SO_REUSEPORT` before bind (Unix only; ignored elsewhere). See the
    /// module doc's per-OS multicast-sharing note. Defaults to `false`.
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
    // SO_REUSEPORT: use socket2's OWN target gate so this can never fail to
    // build where socket2's `set_reuse_port` is absent (solaris/illumos/
    // cygwin/nuttx/wasi, or a build without socket2's `all` feature) — see
    // socket2 `src/sys/unix.rs`'s cfg on `set_reuse_port`. Elsewhere the
    // option is ignored, not mis-set.
    #[cfg(not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi",
        not(target_family = "unix")
    )))]
    if opts.reuse_port {
        socket
            .set_reuse_port(true)
            .map_err(|e| MultimuxError::Connect {
                reason: format!("SO_REUSEPORT: {e}"),
            })?;
    }
    #[cfg(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi",
        not(target_family = "unix")
    ))]
    let _ = opts.reuse_port;
    // A requested buffer is a request; a default applies when none is given,
    // so a high-bitrate input is not left at the OS default. Best-effort: the
    // OS may clamp it (Linux doubles and floors at `rmem_default`; macOS adds
    // overhead), and a failed/clamped `set` must NOT fail the bind — log and
    // carry on.
    let wanted_recv = requested_recv_buffer_bytes(opts.recv_buffer_bytes);
    if let Err(e) = socket.set_recv_buffer_size(wanted_recv) {
        tracing::warn!(
            requested = wanted_recv,
            error = %e,
            "SO_RCVBUF not applied; the socket keeps the OS default"
        );
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
