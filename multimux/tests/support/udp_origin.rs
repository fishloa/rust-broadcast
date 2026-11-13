//! Boots a multimux origin whose single route ingests a TS fixture sent over loopback UDP.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multimux::config::{Config, InputSpec};
use multimux::registry::SchemeRegistry;
use multimux::{MultimuxError, serve_with_registry_on};

use super::listener::bind_tcp;

const TS_PACKET_SIZE: usize = 188;
const DATAGRAM_TS_PACKETS: usize = 7;

pub struct UdpTsOrigin {
    pub bind_addr: SocketAddr,
    pub server: tokio::task::JoinHandle<Result<(), MultimuxError>>,
    pub stop: Arc<AtomicBool>,
    pub sender: tokio::task::JoinHandle<()>,
}

/// Bind `127.0.0.1:0` for UDP, returning the concrete address and the
/// still-bound socket — hand the socket to a route via `Config::prebound`
/// (`test-hooks`) so no test reserves a port and races to re-bind it.
pub async fn bind_udp() -> (SocketAddr, tokio::net::UdpSocket) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0 udp");
    let addr = socket.local_addr().expect("local addr");
    (addr, socket)
}

/// Serves `make_config(bind, TsUdp input)` and loops `fixture` into its UDP socket, one datagram
/// every `pacing`, until `stop` is set.
pub async fn spawn_udp_ts_origin(
    make_config: impl FnOnce(SocketAddr, InputSpec) -> Config,
    fixture: &Path,
    pacing: Duration,
) -> UdpTsOrigin {
    let (bind_addr, bind_listener) = bind_tcp();
    let (udp_addr, udp_socket) = bind_udp().await;
    let mut config = make_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
            socket: Default::default(),
        },
    );
    config.prebound.with_udp(udp_addr.to_string(), udp_socket);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));

    let ts_bytes = std::fs::read(fixture).expect("TS fixture must exist");
    let stop = Arc::new(AtomicBool::new(false));
    let sender_stop = Arc::clone(&stop);
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind sender");
    let sender = tokio::spawn(async move {
        while !sender_stop.load(Ordering::Relaxed) {
            for chunk in ts_bytes.chunks(DATAGRAM_TS_PACKETS * TS_PACKET_SIZE) {
                let _ = socket.send_to(chunk, udp_addr).await;
                tokio::time::sleep(pacing).await;
            }
        }
    });
    UdpTsOrigin {
        bind_addr,
        server,
        stop,
        sender,
    }
}
