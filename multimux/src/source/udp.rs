//! Shared UDP transport helper for the UDP-family ingest sources
//! ([`crate::source::rtp_udp`], [`crate::source::ts_udp`]): bind a socket and
//! optionally join a multicast group. Pure socket setup — no protocol
//! parsing — kept out of both sources so there is exactly one bind/join
//! implementation between them (issue #663 P3a).

use std::net::{IpAddr, SocketAddr};

use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};

/// Binds a UDP socket to `addr` (`host:port`), joining `multicast_group` (if
/// given) on the unspecified (any) local interface.
///
/// `addr` is the local bind address: for multicast reception this is
/// typically `0.0.0.0:<port>` (or `[::]:<port>` for IPv6) with `port`
/// matching the group's advertised port; for unicast it is the specific
/// local address/port the sender targets. `multicast_group`, if present,
/// must be a multicast address of the same IP family as `addr`'s host part.
pub(crate) async fn bind_udp(addr: &str, multicast_group: Option<&str>) -> Result<UdpSocket> {
    let bind_addr: SocketAddr = addr.parse().map_err(|e| MultimuxError::Connect {
        reason: format!("bad UDP bind address {addr:?}: {e}"),
    })?;
    let socket = UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("udp bind {addr}: {e}"),
        })?;
    if let Some(group) = multicast_group {
        let group_ip: IpAddr = group.parse().map_err(|e| MultimuxError::Connect {
            reason: format!("bad multicast group {group:?}: {e}"),
        })?;
        match group_ip {
            IpAddr::V4(v4) => socket
                .join_multicast_v4(v4, std::net::Ipv4Addr::UNSPECIFIED)
                .map_err(|e| MultimuxError::Connect {
                    reason: format!("join multicast group {group}: {e}"),
                })?,
            IpAddr::V6(v6) => {
                socket
                    .join_multicast_v6(&v6, 0)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("join multicast group {group}: {e}"),
                    })?
            }
        }
    }
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binds_ephemeral_loopback_port() {
        let socket = bind_udp("127.0.0.1:0", None).await.unwrap();
        assert!(socket.local_addr().unwrap().port() > 0);
    }

    #[tokio::test]
    async fn rejects_unparsable_addr() {
        assert!(bind_udp("not-an-addr", None).await.is_err());
    }

    #[tokio::test]
    async fn rejects_unparsable_multicast_group() {
        assert!(bind_udp("0.0.0.0:0", Some("not-an-ip")).await.is_err());
    }
}
