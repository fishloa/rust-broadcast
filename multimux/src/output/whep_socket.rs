//! The media sockets of a WHEP route: one UDP socket per advertised IP, shared
//! by every viewer, instead of one socket per viewer.
//!
//! Which viewer a datagram belongs to is `webrtc_runtime::media::SocketDemux`'s
//! decision (ICE ufrag, then mapped source address); this module owns only the
//! IO around it: the receive loop, a bounded queue per viewer, and the socket
//! the viewers send through.
//!
//! A viewer that cannot keep up loses its newest datagrams (`try_send` into a
//! full queue) instead of making the receive loop, and so every other viewer,
//! wait.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use webrtc_runtime::media::{MAX_REMOTE_CANDIDATES, SocketDemux};

/// Largest datagram read: one Ethernet MTU. Inbound WHEP traffic is STUN,
/// DTLS and RTCP, all of which fit.
const RECV_BUFFER_BYTES: usize = 1500;

/// Datagrams one viewer may have queued before further ones are dropped.
const SESSION_INBOUND_QUEUE: usize = 64;

type SessionKey = u64;

/// One inbound datagram and where it came from.
pub(crate) struct Inbound {
    pub peer: SocketAddr,
    pub bytes: Bytes,
}

struct Router {
    demux: SocketDemux<SessionKey>,
    queues: HashMap<SessionKey, mpsc::Sender<Inbound>>,
}

/// One bound socket and the receive loop feeding its viewers.
pub(crate) struct SharedSocket {
    socket: Arc<UdpSocket>,
    local_addr: SocketAddr,
    router: Arc<Mutex<Router>>,
    next_key: AtomicU64,
    #[cfg(test)]
    dropped: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl Drop for SharedSocket {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SharedSocket {
    fn bind(ip: IpAddr) -> io::Result<Self> {
        let std_socket = std::net::UdpSocket::bind((ip, 0))?;
        std_socket.set_nonblocking(true)?;
        let socket = Arc::new(UdpSocket::from_std(std_socket)?);
        let local_addr = socket.local_addr()?;
        let router = Arc::new(Mutex::new(Router {
            demux: SocketDemux::new(MAX_REMOTE_CANDIDATES),
            queues: HashMap::new(),
        }));
        let dropped = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn(receive_loop(
            Arc::clone(&socket),
            Arc::clone(&router),
            Arc::clone(&dropped),
        ));
        Ok(Self {
            socket,
            local_addr,
            router,
            next_key: AtomicU64::new(0),
            #[cfg(test)]
            dropped,
            task,
        })
    }

    /// The address every viewer on this socket advertises as its candidate.
    pub(crate) fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The socket viewers send through.
    pub(crate) fn socket(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Datagrams dropped because a viewer's queue was full.
    #[cfg(test)]
    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Route datagrams naming `local_ufrag` to a new viewer. `None` when
    /// another viewer already owns that ufrag.
    pub(crate) fn attach(
        self: &Arc<Self>,
        local_ufrag: &str,
    ) -> Option<(Attachment, mpsc::Receiver<Inbound>)> {
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(SESSION_INBOUND_QUEUE);
        {
            let mut router = self.router.lock();
            if !router.demux.register(local_ufrag, key) {
                return None;
            }
            router.queues.insert(key, tx);
        }
        Some((
            Attachment {
                shared: Arc::clone(self),
                key,
            },
            rx,
        ))
    }
}

async fn receive_loop(socket: Arc<UdpSocket>, router: Arc<Mutex<Router>>, dropped: Arc<AtomicU64>) {
    let mut buf = [0u8; RECV_BUFFER_BYTES];
    loop {
        let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let data = &buf[..len];
        let queue = {
            let mut router = router.lock();
            router
                .demux
                .classify(data, peer)
                .and_then(|key| router.queues.get(&key).cloned())
        };
        let Some(queue) = queue else {
            continue;
        };
        let datagram = Inbound {
            peer,
            bytes: Bytes::copy_from_slice(data),
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = queue.try_send(datagram) {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A viewer's registration on a [`SharedSocket`]; dropping it unregisters the
/// viewer and everything it mapped.
pub(crate) struct Attachment {
    shared: Arc<SharedSocket>,
    key: SessionKey,
}

impl Attachment {
    /// Also route datagrams naming `local_ufrag` (the one an ICE restart
    /// introduces) to this viewer.
    pub(crate) fn register_ufrag(&self, local_ufrag: &str) {
        if !self
            .shared
            .router
            .lock()
            .demux
            .register(local_ufrag, self.key)
        {
            tracing::warn!("whep: restart ufrag already belongs to another viewer");
        }
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        let mut router = self.shared.router.lock();
        router.demux.unregister(&self.key);
        router.queues.remove(&self.key);
    }
}

/// The media sockets of one advertised IP: bound one by one as viewers arrive up to
/// the configured count, then handed out round-robin.
#[derive(Default)]
struct IpSockets {
    shards: Vec<Arc<SharedSocket>>,
    next: usize,
}

/// A route's media sockets, up to `per_ip` per advertised IP, bound on first use.
pub(crate) struct SharedSockets {
    per_ip: usize,
    sockets: Mutex<HashMap<IpAddr, IpSockets>>,
}

impl Default for SharedSockets {
    fn default() -> Self {
        Self::with_sockets_per_ip(crate::config::DEFAULT_WHEP_MEDIA_SOCKETS_PER_IP)
    }
}

impl SharedSockets {
    /// Sockets per advertised IP: clamped up to one.
    pub(crate) fn with_sockets_per_ip(per_ip: usize) -> Self {
        Self {
            per_ip: per_ip.max(1),
            sockets: Mutex::new(HashMap::new()),
        }
    }

    /// A socket for `ip`: a newly bound one while the IP has fewer than the configured count,
    /// otherwise the next in rotation.
    pub(crate) fn socket_for(&self, ip: IpAddr) -> io::Result<Arc<SharedSocket>> {
        let mut sockets = self.sockets.lock();
        let entry = sockets.entry(ip).or_default();
        if entry.shards.len() < self.per_ip {
            let socket = Arc::new(SharedSocket::bind(ip)?);
            entry.shards.push(Arc::clone(&socket));
            return Ok(socket);
        }
        let socket = Arc::clone(&entry.shards[entry.next % entry.shards.len()]);
        entry.next = entry.next.wrapping_add(1);
        Ok(socket)
    }

    /// How many sockets are bound.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sockets.lock().values().map(|ip| ip.shards.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc_stun::attributes::ATTR_USERNAME;
    use rtc_stun::fingerprint::FINGERPRINT;
    use rtc_stun::message::{BINDING_REQUEST, Message, TransactionId};
    use rtc_stun::textattrs::TextAttribute;
    use std::net::Ipv4Addr;
    use std::time::Duration;

    const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const DTLS_RECORD: [u8; 4] = [22, 254, 253, 0];
    const WAIT: Duration = Duration::from_secs(5);

    fn check(username: &str) -> Vec<u8> {
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TransactionId::new()),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_string())),
                Box::new(FINGERPRINT),
            ])
            .expect("build a binding request");
        message.raw
    }

    async fn client() -> UdpSocket {
        UdpSocket::bind((LOCALHOST, 0)).await.expect("bind client")
    }

    /// The viewers of a route on one IP spread evenly over exactly the configured number of
    /// sockets, whatever that number is.
    #[tokio::test]
    async fn viewers_of_a_route_spread_over_the_configured_number_of_sockets() {
        const ROUNDS: usize = 3;
        for per_ip in [1, 3, 8] {
            let sockets = SharedSockets::with_sockets_per_ip(per_ip);
            let mut attached = Vec::new();
            let mut per_address: HashMap<SocketAddr, usize> = HashMap::new();
            for i in 0..per_ip * ROUNDS {
                let socket = sockets.socket_for(LOCALHOST).expect("socket");
                *per_address.entry(socket.local_addr()).or_default() += 1;
                attached.push(socket.attach(&format!("ufrag{i:04}")).expect("attach"));
            }
            assert_eq!(sockets.len(), per_ip, "bound sockets");
            assert_eq!(per_address.len(), per_ip, "distinct addresses");
            assert!(per_address.values().all(|viewers| *viewers == ROUNDS));
        }
    }

    #[test]
    fn a_zero_count_is_clamped_to_one_socket() {
        assert_eq!(SharedSockets::with_sockets_per_ip(0).per_ip, 1);
    }

    /// A check reaches the viewer it names, later datagrams from that address
    /// follow it, and another viewer sees none of it.
    #[tokio::test]
    async fn datagrams_reach_only_the_viewer_that_owns_the_sender() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_a, mut a_rx) = shared.attach("aaaaaaaa").expect("attach a");
        let (_b, mut b_rx) = shared.attach("bbbbbbbb").expect("attach b");
        let peer = client().await;
        peer.send_to(&check("bbbbbbbb:peer"), shared.local_addr())
            .await
            .expect("send check");
        peer.send_to(&DTLS_RECORD, shared.local_addr())
            .await
            .expect("send dtls");
        for _ in 0..2 {
            let got = tokio::time::timeout(WAIT, b_rx.recv())
                .await
                .expect("b receives")
                .expect("open");
            assert_eq!(got.peer, peer.local_addr().expect("addr"));
        }
        assert!(a_rx.try_recv().is_err());
    }

    /// A check naming an unknown ufrag, and datagrams from an unmapped
    /// address, reach nobody.
    #[tokio::test]
    async fn a_spoofed_check_reaches_nobody() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_a, mut a_rx) = shared.attach("aaaaaaaa").expect("attach");
        let peer = client().await;
        peer.send_to(&check("zzzzzzzz:peer"), shared.local_addr())
            .await
            .expect("send");
        peer.send_to(&DTLS_RECORD, shared.local_addr())
            .await
            .expect("send");
        let mapped = client().await;
        mapped
            .send_to(&check("aaaaaaaa:peer"), shared.local_addr())
            .await
            .expect("send");
        let got = tokio::time::timeout(WAIT, a_rx.recv())
            .await
            .expect("a receives")
            .expect("open");
        assert_eq!(got.peer, mapped.local_addr().expect("addr"));
        assert!(a_rx.try_recv().is_err(), "only the real check arrived");
    }

    /// A viewer whose queue is full loses datagrams; the receive loop goes on
    /// serving the others.
    #[tokio::test]
    async fn a_full_viewer_queue_drops_without_blocking_the_others() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let (_slow, _slow_rx) = shared.attach("aaaaaaaa").expect("attach slow");
        let (_fast, mut fast_rx) = shared.attach("bbbbbbbb").expect("attach fast");
        let slow_peer = client().await;
        let fast_peer = client().await;
        slow_peer
            .send_to(&check("aaaaaaaa:p"), shared.local_addr())
            .await
            .expect("send");
        for _ in 0..SESSION_INBOUND_QUEUE * 2 {
            slow_peer
                .send_to(&DTLS_RECORD, shared.local_addr())
                .await
                .expect("send");
        }
        fast_peer
            .send_to(&check("bbbbbbbb:p"), shared.local_addr())
            .await
            .expect("send");
        let got = tokio::time::timeout(WAIT, fast_rx.recv())
            .await
            .expect("the other viewer is still served")
            .expect("open");
        assert_eq!(got.peer, fast_peer.local_addr().expect("addr"));
        assert!(shared.dropped() > 0, "the full queue dropped datagrams");
    }

    /// Dropping an attachment frees its ufrag.
    #[tokio::test]
    async fn a_dropped_attachment_frees_its_ufrag() {
        let sockets = SharedSockets::default();
        let shared = sockets.socket_for(LOCALHOST).expect("socket");
        let first = shared.attach("aaaaaaaa").expect("attach");
        assert!(shared.attach("aaaaaaaa").is_none());
        drop(first);
        assert!(shared.attach("aaaaaaaa").is_some());
    }
}
