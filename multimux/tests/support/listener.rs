//! SP7.1 test harness: bind an ephemeral loopback listener and hand the LIVE
//! listener to the code under test, so no test reserves a port and then races
//! to re-bind it (`reserve_then_drop` + `TcpListener::bind(addr)`).

use std::net::SocketAddr;

/// Bind `127.0.0.1:0`, returning the concrete address and the still-bound
/// tokio listener — pass the listener to `serve_*_on`.
pub fn bind_tcp() -> (SocketAddr, tokio::net::TcpListener) {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    std_listener.set_nonblocking(true).expect("set nonblocking");
    let addr = std_listener.local_addr().expect("local addr");
    let listener = tokio::net::TcpListener::from_std(std_listener).expect("tokio listener");
    (addr, listener)
}

/// Bind `127.0.0.1:0` for UDP, returning the concrete address and the
/// still-bound socket — hand the socket to a route via `Config::prebound`
/// (`test-hooks`) so no test reserves a port and races to re-bind it.
#[allow(dead_code)]
pub async fn bind_udp() -> (SocketAddr, tokio::net::UdpSocket) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0 udp");
    let addr = socket.local_addr().expect("local addr");
    (addr, socket)
}
